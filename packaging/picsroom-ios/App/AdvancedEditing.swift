import SwiftUI
import CoreImage
import UniformTypeIdentifiers

struct RAWSettings: Codable, Equatable {
    var temperature: Double
    var tint: Double
    var lensCorrection: Bool
    var lensSupported: Bool
    var decoder: String
}
struct SourceEdit: Codable, Equatable {
    var kind: String
    var x: Double
    var y: Double
    var width: Double
    var height: Double
}
struct Grade: Codable, Equatable {
    var hue = 0.0
    var saturation = 0.0
    var luminance = 0.0
}
struct RepairSpot: Codable, Equatable, Identifiable {
    var id=UUID().uuidString
    var source: MaskPoint
    var destination: MaskPoint
    var radius=0.04
    var heal=true
}
struct AdvancedSettings: Codable, Equatable {
    var curves = Array(repeating:[MaskPoint(x:0,y:0),MaskPoint(x:0.25,y:0.25),MaskPoint(x:0.5,y:0.5),MaskPoint(x:0.75,y:0.75),MaskPoint(x:1,y:1)],count:4)
    var grades = Array(repeating:Grade(),count:3)
    var balance=0.0
    var angle=0.0
    var vertical=0.0
    var horizontal=0.0
    var repairs:[RepairSpot]=[]
}

/// Original files remain separate from the 8-bit compatibility layer in the document.
/// Each call creates its own RAW filter; mutable filters never cross render queues.
enum OriginalPipeline {
    static let linear = CGColorSpace(name:CGColorSpace.extendedLinearDisplayP3)!
    static func rawSettings(_ url:URL)->RAWSettings? {
        guard let f=CIRAWFilter(imageURL:url) else { return nil }
        return RAWSettings(temperature:Double(f.neutralTemperature),tint:Double(f.neutralTint),lensCorrection:f.isLensCorrectionEnabled,lensSupported:f.isLensCorrectionSupported,decoder:f.decoderVersion.rawValue)
    }
    static func image(_ url:URL,state:NativeState,maxSide:CGFloat?) throws -> CIImage {
        var image:CIImage
        if let raw=state.raw {
            guard let f=CIRAWFilter(imageURL:url) else { throw EditorError(message:"Dieses RAW wird von dieser iOS-Version nicht unterstützt. Das Original wurde nicht verändert.") }
            let decoder=CIRAWDecoderVersion(rawValue:raw.decoder)
            guard f.supportedDecoderVersions.contains(decoder) else { throw EditorError(message:"Der gespeicherte RAW-Decoder ist auf diesem Gerät nicht verfügbar.") }
            f.decoderVersion=decoder
            f.neutralTemperature=Float(raw.temperature);f.neutralTint=Float(raw.tint)
            f.exposure=Float(state.development.exposure)
            f.extendedDynamicRangeAmount=1
            if f.isLensCorrectionSupported { f.isLensCorrectionEnabled=raw.lensCorrection }
            if let maxSide { f.scaleFactor=Float(min(1,maxSide/max(f.nativeSize.width,f.nativeSize.height))) }
            guard let output=f.outputImage else { throw EditorError(message:"RAW konnte nicht entwickelt werden. Gegebenenfalls benötigt iOS Kameradaten oder ein Systemupdate.") }
            image=output
        } else {
            guard let decoded=CIImage(contentsOf:url,options:[.applyOrientationProperty:true]) else { throw EditorError(message:"Original konnte nicht gelesen werden.") }
            image=decoded
        }
        image=origin(image)
        let extent=image.extent
        let bound=min(1,min(8192/max(extent.width,extent.height),sqrt(50_000_000/(extent.width*extent.height))))
        if bound<1 { image=image.transformed(by:CGAffineTransform(scaleX:bound,y:bound)) }
        if let maxSide {
            let scale=min(1,maxSide/max(image.extent.width,image.extent.height))
            image=image.transformed(by:CGAffineTransform(scaleX:scale,y:scale))
        }
        // Retouch the un-cropped original so a clone source outside a later crop remains available.
        var repairs=state.advanced?.repairs ?? []
        for edit in (state.sourceEdits ?? []).reversed() {
            func inverse(_ point:MaskPoint)->MaskPoint {
                switch edit.kind {
                case "image.crop": return MaskPoint(x:edit.x+point.x*edit.width,y:edit.y+point.y*edit.height)
                case "image.imageRotation.90cw": return MaskPoint(x:point.y,y:1-point.x)
                case "image.imageRotation.flipCanvasHorizontal": return MaskPoint(x:1-point.x,y:point.y)
                default: return point
                }
            }
            for i in repairs.indices { repairs[i].source=inverse(repairs[i].source);repairs[i].destination=inverse(repairs[i].destination);repairs[i].radius *= edit.width }
        }
        image=DevelopRenderer.shared.applyRepairs(image,repairs)
        for edit in state.sourceEdits ?? [] {
            let e=image.extent
            switch edit.kind {
            case "image.crop": image=origin(image.cropped(to:CGRect(x:edit.x*e.width,y:(1-edit.y-edit.height)*e.height,width:edit.width*e.width,height:edit.height*e.height)))
            case "image.imageRotation.90cw": image=origin(image.oriented(.right))
            case "image.imageRotation.flipCanvasHorizontal": image=origin(image.oriented(.upMirrored))
            default: break
            }
        }
        return image
    }
    static func origin(_ image:CIImage)->CIImage { image.transformed(by:CGAffineTransform(translationX:-image.extent.minX,y:-image.extent.minY)) }
}

struct CurveEditor:View {
    @Environment(\.studioPanelHeight) private var panelHeight
    @Binding var points:[MaskPoint]
    var changed:()->Void
    var finished:()->Void
    var body:some View {
        GeometryReader { geo in
            ZStack {
                Color.white.opacity(0.04)
                Path { p in
                    for i in 1...3 { let v=CGFloat(i)/4;p.move(to:CGPoint(x:geo.size.width*v,y:0));p.addLine(to:CGPoint(x:geo.size.width*v,y:geo.size.height));p.move(to:CGPoint(x:0,y:geo.size.height*v));p.addLine(to:CGPoint(x:geo.size.width,y:geo.size.height*v)) }
                }.stroke(.white.opacity(0.15))
                Path { path in for (i,p) in points.enumerated() { let q=CGPoint(x:p.x*geo.size.width,y:(1-p.y)*geo.size.height);if i==0 { path.move(to:q) } else { path.addLine(to:q) } } }.stroke(.white,lineWidth:2)
                ForEach(points.indices,id:\.self) { i in
                    Circle().fill(.blue).frame(width:16,height:16).contentShape(Rectangle().size(width:32,height:32))
                        .position(x:points[i].x*geo.size.width,y:(1-points[i].y)*geo.size.height)
                        .gesture(DragGesture(minimumDistance:0,coordinateSpace:.named("curve")).onChanged { v in
                            let low=i==0 ? 0 : points[i-1].x+0.015, high=i==points.count-1 ? 1 : points[i+1].x-0.015
                            if i>0 && i<points.count-1 { points[i].x=min(high,max(low,v.location.x/geo.size.width)) }
                            points[i].y=min(1,max(0,1-v.location.y/geo.size.height));changed()
                        }.onEnded { _ in finished() })
                        .accessibilityLabel("Kurvenpunkt \(i+1)")
                        .accessibilityAdjustableAction { direction in points[i].y=min(1,max(0,points[i].y+(direction == .increment ? 0.02 : -0.02)));changed();finished() }
                }
            }.coordinateSpace(name:"curve")
        }.frame(height:min(180,max(90,panelHeight-120))).padding(8)
    }
}
struct GradingWheel:View {
    @Binding var grade:Grade
    let changed:()->Void
    let finished:()->Void
    var body:some View {
        ZStack {
            Circle().fill(AngularGradient(colors:[.red,.yellow,.green,.cyan,.blue,.purple,.red],center:.center))
            Circle().fill(RadialGradient(colors:[.gray,.clear],center:.center,startRadius:0,endRadius:80))
            Circle().stroke(.white,lineWidth:3).frame(width:14,height:14)
                .offset(x:cos(grade.hue * .pi/180)*grade.saturation*0.75,y:sin(grade.hue * .pi/180)*grade.saturation*0.75)
        }.frame(width:160,height:160).contentShape(Circle())
            .gesture(DragGesture(minimumDistance:0).onChanged { v in
                let x=v.location.x-80,y=v.location.y-80
                grade.hue=(atan2(y,x)*180 / .pi+360).truncatingRemainder(dividingBy:360)
                grade.saturation=min(100,hypot(x,y)/0.75);changed()
            }.onEnded { _ in finished() }).accessibilityLabel("Farbrad")
    }
}

struct CatalogEntry:Codable { var album="";var rating=0 }
struct UserLook:Codable,Identifiable { var id=UUID().uuidString;var name:String;var development:Development;var curves:[[MaskPoint]];var grades:[Grade];var balance:Double;var importedValues:[String:Double]? = nil }
struct PhotoCatalog:Codable { var entries:[String:CatalogEntry]=[:];var presets:[UserLook]=[] }

/// Reads only Adobe Camera Raw scalar settings; never resolves external XML entities.
final class PresetXML: NSObject, XMLParserDelegate {
    static let namespace="http://ns.adobe.com/camera-raw-settings/1.0/"
    static let fields:[String:(WritableKeyPath<Development,Double>,ClosedRange<Double>)]=[
        "Exposure2012":(\.exposure,-5...5), "Contrast2012":(\.contrast,-100...100),
        "Highlights2012":(\.highlights,-100...100), "Shadows2012":(\.shadows,-100...100),
        "Whites2012":(\.whites,-100...100), "Blacks2012":(\.blacks,-100...100),
        "Saturation":(\.saturation,-100...100), "Vibrance":(\.vibrance,-100...100),
        "Texture":(\.texture,-100...100), "Clarity2012":(\.clarity,-100...100), "Dehaze":(\.dehaze,-100...100)]
    var values:[String:Double]=[:]
    private var namespaces:[String:[String]]=[:]
    private var elements:[(String?,String,String)]=[]
    func parser(_ parser:XMLParser,didStartMappingPrefix prefix:String,toURI namespaceURI:String) {namespaces[prefix,default:[]].append(namespaceURI)}
    func parser(_ parser:XMLParser,didEndMappingPrefix prefix:String) { _=namespaces[prefix]?.popLast() }
    private func accept(_ key:String,_ text:String) {
        guard let field=Self.fields[key],let value=Double(text.trimmingCharacters(in:.whitespacesAndNewlines)),value.isFinite else {return}
        values[key]=min(field.1.upperBound,max(field.1.lowerBound,value))
    }
    func parser(_ parser:XMLParser,didStartElement elementName:String,namespaceURI:String?,qualifiedName:String?,attributes:[String:String]) {
        elements.append((namespaceURI,elementName,""))
        for (name,value) in attributes {
            let parts=name.split(separator:":",maxSplits:1).map(String.init)
            if parts.count==2,namespaces[parts[0]]?.last == Self.namespace {accept(parts[1],value)}
        }
    }
    func parser(_ parser:XMLParser,foundCharacters string:String) {if !elements.isEmpty {elements[elements.count-1].2 += string}}
    func parser(_ parser:XMLParser,didEndElement elementName:String,namespaceURI:String?,qualifiedName:String?) {
        if let element=elements.popLast(),element.0 == Self.namespace {accept(element.1,element.2)}
    }
    static func read(_ data:Data,name:String) throws -> UserLook {
        let delegate=PresetXML(),parser=XMLParser(data:data)
        parser.shouldProcessNamespaces=true;parser.shouldReportNamespacePrefixes=true;parser.shouldResolveExternalEntities=false;parser.delegate=delegate
        guard parser.parse(),!delegate.values.isEmpty else {throw EditorError(message:"Diese XMP-Datei enthält keine unterstützten Licht- oder Farbwerte.")}
        let advanced=AdvancedSettings()
        return UserLook(name:name,development:Development(),curves:advanced.curves,grades:advanced.grades,balance:0,importedValues:delegate.values)
    }
}
