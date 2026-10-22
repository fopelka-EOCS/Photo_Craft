import Foundation
import Vision
import CoreML
import CoreImage
import UIKit

enum AutomaticRegion: String, CaseIterable, Identifiable {
    case subject="Motiv", objects="Objekte", background="Hintergrund", sky="Himmel", skin="Haut", hair="Haar", eyes="Augen"
    var id:String { rawValue }
    var symbol:String {
        switch self {
        case .subject: "person.crop.rectangle.dashed"
        case .objects: "viewfinder"
        case .background: "person.and.background.dotted"
        case .sky: "cloud.fill"
        case .skin: "face.smiling.fill"
        case .hair: "person.crop.circle"
        case .eyes: "eye.fill"
        }
    }
}

/// Only accessed on the dedicated segmentation queue. No image leaves the device.
final class AutomaticMaskEngine {
    static let shared=AutomaticMaskEngine()
    private let context=CIContext(options:[.cacheIntermediates:false])
    private var cachedModel:MLModel?
    private var cachedName:String?
    private func model(_ name:String) throws -> MLModel {
        if cachedName == name, let cachedModel { return cachedModel }
        cachedModel=nil; cachedName=nil
        guard let url=Bundle.main.url(forResource:name,withExtension:"mlmodelc") else { throw EditorError(message:"Das Erkennungsmodell fehlt in dieser Installation.") }
        let config=MLModelConfiguration()
        #if targetEnvironment(simulator)
        config.computeUnits = .cpuOnly
        #else
        config.computeUnits = name == "FaceParsing" ? .cpuAndGPU : .all
        #endif
        let result=try MLModel(contentsOf:url,configuration:config)
        cachedModel=result; cachedName=name; return result
    }
    func generate(_ region:AutomaticRegion,image:CGImage) throws -> [LocalMask] {
        if [.subject,.objects,.background].contains(region) { return try foreground(region,image:image) }
        if region == .sky {
            let model=try model("DETRResnet50SemanticSegmentationF16")
            let metadata=model.modelDescription.metadata[.creatorDefinedKey] as? [String:String]
            let json=metadata?["com.apple.coreml.model.preview.params"]?.data(using:.utf8)
            let labels=(json.flatMap { try? JSONSerialization.jsonObject(with:$0) } as? [String:Any])?["labels"] as? [String] ?? []
            guard let sky=labels.firstIndex(where:{$0.hasPrefix("sky")}) else { throw EditorError(message:"Das Himmelmodell enthält keine passende Klasse.") }
            let map=try predict(model,image:image)
            return [try makeMask(name:region.rawValue,map:map,classes:[sky])]
        }
        let request=VNDetectFaceRectanglesRequest()
        #if targetEnvironment(simulator)
        request.usesCPUOnly=true;request.revision=VNDetectFaceRectanglesRequestRevision2
        #endif
        try VNImageRequestHandler(cgImage:image,orientation:.up).perform([request])
        let faces=(request.results ?? []).filter { $0.confidence>0.5 }.sorted { $0.boundingBox.midX<$1.boundingBox.midX }
        guard !faces.isEmpty else { throw EditorError(message:"Kein ausreichend deutliches Gesicht gefunden. Nutze ein schärferes Porträt oder den Pinsel.") }
        let model=try model("FaceParsing")
        let classes:Set<Int> = region == .skin ? [1,7,8,10,14] : region == .hair ? [17] : [4,5]
        var result:[LocalMask]=[]
        for (index,face) in faces.prefix(12).enumerated() {
            let b=face.boundingBox
            // A square portrait crop includes hair above the detected face and neck below it.
            let width=CGFloat(image.width),height=CGFloat(image.height)
            let side=max(b.width*width*1.9,b.height*height*1.9)
            let cx=b.midX*width,cy=(1-b.midY)*height-b.height*height*0.08
            let rect=CGRect(x:cx-side/2,y:cy-side/2,width:side,height:side).intersection(CGRect(x:0,y:0,width:width,height:height)).integral
            guard let crop=image.cropping(to:rect) else { continue }
            let map=try predict(model,image:crop)
            if var mask=try? makeMask(name:"\(region.rawValue) · Person \(index+1)",map:map,classes:classes) {
                mask.rasterCorners=[MaskPoint(x:rect.minX/width,y:rect.minY/height),MaskPoint(x:rect.maxX/width,y:rect.minY/height),MaskPoint(x:rect.minX/width,y:rect.maxY/height)]
                result.append(mask)
            }
        }
        guard !result.isEmpty else { throw EditorError(message:"\(region.rawValue) wurde nicht sicher erkannt. Prüfe, ob dieser Bereich deutlich sichtbar ist.") }
        return result
    }
    private func foreground(_ region:AutomaticRegion,image:CGImage) throws -> [LocalMask] {
        let request=VNGenerateForegroundInstanceMaskRequest()
        let handler=VNImageRequestHandler(cgImage:image,orientation:.up)
        try handler.perform([request])
        guard let observation=request.results?.first,!observation.allInstances.isEmpty else { throw EditorError(message:"Kein klar abgrenzbares Motiv gefunden. Du kannst den Bereich mit dem Pinsel markieren.") }
        let groups=region == .objects ? observation.allInstances.map { IndexSet(integer:$0) } : [observation.allInstances]
        return try groups.prefix(12).enumerated().map { index,instances in
            let pixels=try observation.generateScaledMaskForImage(forInstances:instances,from:handler)
            var ci=CIImage(cvPixelBuffer:pixels)
            if region == .background { ci=ci.applyingFilter("CIColorInvert") }
            let scale=min(1,1024/max(ci.extent.width,ci.extent.height))
            ci=ci.transformed(by:CGAffineTransform(scaleX:scale,y:scale))
            guard let cg=context.createCGImage(ci,from:ci.extent), let data=UIImage(cgImage:cg).pngData() else { throw EditorError(message:"Die Auswahl konnte nicht erstellt werden.") }
            var mask=LocalMask(); mask.kind="ai";mask.name=region == .objects ? "Objekt \(index+1)" : region.rawValue
            mask.rasterPNG=data;mask.rasterCorners=Self.fullCorners;mask.feather=0.05
            return mask
        }
    }
    private func predict(_ model:MLModel,image:CGImage) throws -> MLMultiArray {
        guard let input=model.modelDescription.inputDescriptionsByName.first(where:{$0.value.type == .image}),let constraint=input.value.imageConstraint else { throw EditorError(message:"Unbekanntes Modellformat.") }
        var buffer:CVPixelBuffer?
        let status=CVPixelBufferCreate(kCFAllocatorDefault,constraint.pixelsWide,constraint.pixelsHigh,kCVPixelFormatType_32BGRA,[kCVPixelBufferIOSurfacePropertiesKey:[:]] as CFDictionary,&buffer)
        guard status == kCVReturnSuccess,let buffer else { throw EditorError(message:"Nicht genügend Speicher für die Erkennung.") }
        let ci=CIImage(cgImage:image).transformed(by:CGAffineTransform(scaleX:CGFloat(constraint.pixelsWide)/CGFloat(image.width),y:CGFloat(constraint.pixelsHigh)/CGFloat(image.height)))
        context.render(ci,to:buffer,bounds:CGRect(x:0,y:0,width:constraint.pixelsWide,height:constraint.pixelsHigh),colorSpace:CGColorSpace(name:CGColorSpace.sRGB))
        let output=try model.prediction(from:MLDictionaryFeatureProvider(dictionary:[input.key:MLFeatureValue(pixelBuffer:buffer)]))
        guard let map=output.featureNames.compactMap({output.featureValue(for:$0)?.multiArrayValue}).first else { throw EditorError(message:"Die Erkennung hat kein Ergebnis geliefert.") }
        return map
    }
    private func makeMask(name:String,map:MLMultiArray,classes:Set<Int>) throws -> LocalMask {
        let shape=map.shape.map(\.intValue),strides=map.strides.map(\.intValue)
        guard shape.count>=2,let width=shape.last,width>0 else { throw EditorError(message:"Ungültige Segmentkarte.") }
        let height=shape[shape.count-2],sx=strides.last!,sy=strides[strides.count-2]
        var bytes=[UInt8](repeating:0,count:width*height), count=0
        for y in 0..<height { for x in 0..<width {
            let offset=y*sy+x*sx
            let label:Int
            switch map.dataType {
            case .int32: label=Int(map.dataPointer.assumingMemoryBound(to:Int32.self)[offset])
            case .float32: label=Int(map.dataPointer.assumingMemoryBound(to:Float.self)[offset])
            default: label=map[[NSNumber(value:y),NSNumber(value:x)]].intValue
            }
            if classes.contains(label) { bytes[y*width+x]=255;count+=1 }
        } }
        guard count>=8 else { throw EditorError(message:"\(name) wurde in diesem Foto nicht erkannt.") }
        guard let provider=CGDataProvider(data:Data(bytes) as CFData),let cg=CGImage(width:width,height:height,bitsPerComponent:8,bitsPerPixel:8,bytesPerRow:width,space:CGColorSpaceCreateDeviceGray(),bitmapInfo:CGBitmapInfo(rawValue:0),provider:provider,decode:nil,shouldInterpolate:true,intent:.defaultIntent),let png=UIImage(cgImage:cg).pngData() else { throw EditorError(message:"Die Auswahl konnte nicht gespeichert werden.") }
        var mask=LocalMask();mask.kind="ai";mask.name=name;mask.rasterPNG=png;mask.rasterCorners=Self.fullCorners;mask.feather=0.05
        return mask
    }
    static let fullCorners=[MaskPoint(x:0,y:0),MaskPoint(x:1,y:0),MaskPoint(x:0,y:1)]
}
