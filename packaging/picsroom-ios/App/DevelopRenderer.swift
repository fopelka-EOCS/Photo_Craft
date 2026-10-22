import UIKit
import CoreImage
import Metal

struct MaskPoint: Codable, Hashable { var x: Double; var y: Double }
struct MaskStroke: Codable, Hashable { var points: [MaskPoint]; var radius: Double; var erase: Bool }
struct LocalMask: Codable, Hashable, Identifiable {
    var id = UUID().uuidString
    var name = "Maske"
    var kind = "radial"
    var start = MaskPoint(x:0.3,y:0.3)
    var end = MaskPoint(x:0.7,y:0.7)
    var feather = 0.6
    var inverted = false
    var enabled = true
    var exposure = 0.0
    var warmth = 0.0
    var saturation = 0.0
    var strokes: [MaskStroke] = []
    var adjustments: Development?
    var rasterPNG: Data?
    var rasterCorners: [MaskPoint]?
}
struct ColorBand: Codable, Hashable { var hue = 0.0; var saturation = 0.0; var luminance = 0.0 }
struct NativeState: Codable, Equatable {
    var development = Development()
    var masks: [LocalMask] = []
    var raw: RAWSettings?
    var sourceEdits: [SourceEdit]?
    var advanced: AdvancedSettings?
}

/// A reused thread-safe GPU context and bounded matte cache. Rendering stays off the main thread.
final class DevelopRenderer {
    static let shared = DevelopRenderer()
    let context: CIContext
    private let lensWarp: CIWarpKernel?
    private let channels: CIColorKernel?
    private let colorMix: CIColorKernel?
    private let tone: CIColorKernel?
    private let maskKernel: CIColorKernel?
    private let matteCache=NSCache<NSData,CIImage>()
    private let detailKernel: CIColorKernel?
    private let curveKernel: CIColorKernel?
    private let gradeKernel: CIColorKernel?
    private let healKernel: CIColorKernel?
    private let rawCache=NSCache<NSString,CIImage>()
    private init() {
        rawCache.countLimit=2;rawCache.totalCostLimit=48*1024*1024
        matteCache.countLimit=16;matteCache.totalCostLimit=32*1024*1024
        if let device = MTLCreateSystemDefaultDevice() {
            context = CIContext(mtlDevice:device, options:[.cacheIntermediates:false, .workingColorSpace:CGColorSpace(name:CGColorSpace.extendedSRGB)!, .workingFormat:CIFormat.RGBAh])
        } else { context = CIContext(options:[.useSoftwareRenderer:false]) }
        healKernel=CIColorKernel(source:"kernel vec4 heal(__sample p,__sample s,__sample d){return vec4(max(p.rgb-s.rgb+d.rgb,vec3(0.0)),p.a);}")
        curveKernel=CIColorKernel(source:"""
        float mapped(float v,vec2 a,vec2 b,vec2 c,vec2 d,vec2 e) {
            if(v>1.0) return e.y+(v-1.0);
            vec2 p=a;vec2 q=b;
            if(v>=b.x){p=b;q=c;} if(v>=c.x){p=c;q=d;} if(v>=d.x){p=d;q=e;}
            return mix(p.y,q.y,clamp((v-p.x)/max(0.0001,q.x-p.x),0.0,1.0));
        }
        kernel vec4 curve(__sample s,vec2 a,vec2 b,vec2 c,vec2 d,vec2 e,float channel) {
            vec3 v=unpremultiply(s).rgb;
            if(channel<0.5 || channel<1.5) v.r=mapped(v.r,a,b,c,d,e);
            if(channel<0.5 || (channel>1.5 && channel<2.5)) v.g=mapped(v.g,a,b,c,d,e);
            if(channel<0.5 || channel>2.5) v.b=mapped(v.b,a,b,c,d,e);
            return premultiply(vec4(v,s.a));
        }
        """)
        gradeKernel=CIColorKernel(source:"""
        kernel vec4 grade(__sample s,vec3 shadows,vec3 mids,vec3 highs,float balance) {
            vec3 c=unpremultiply(s).rgb;float l=dot(c,vec3(0.2126,0.7152,0.0722));
            float low=1.0-smoothstep(0.0,0.65+balance*0.2,l);float hi=smoothstep(0.35+balance*0.2,1.0,l);
            c+=shadows*low+mids*max(0.0,1.0-low-hi)+highs*hi;
            return premultiply(vec4(max(c,vec3(0.0)),s.a));
        }
        """)
        lensWarp=CIWarpKernel(source:"""
        kernel vec2 lens(float w,float h,float amount) {
            vec2 center=vec2(w,h)*0.5;
            vec2 p=(destCoord()-center)/max(w,h);
            return center+p*(1.0+amount*dot(p,p))*max(w,h);
        }
        """)
        channels=CIColorKernel(source:"""
        kernel vec4 channels(__sample original,__sample red,__sample blue) {
            return vec4(red.r,original.g,blue.b,original.a);
        }
        """)
        colorMix = CIColorKernel(source:"""
        kernel vec4 band(__sample p,float target,float hue,float sat,float light) {
            vec3 c=unpremultiply(p).rgb;
            float mx=max(c.r,max(c.g,c.b)); float mn=min(c.r,min(c.g,c.b)); float delta=mx-mn;
            float h=0.0;
            if (delta>0.00001) {
                if (mx==c.r) h=mod((c.g-c.b)/delta,6.0);
                else if (mx==c.g) h=(c.b-c.r)/delta+2.0;
                else h=(c.r-c.g)/delta+4.0;
                h=fract(h/6.0+1.0);
            }
            float distance=abs(h-target); distance=min(distance,1.0-distance);
            float weight=(1.0-smoothstep(0.035,0.16,distance))*smoothstep(0.01,0.1,delta);
            h=fract(h+hue/720.0*weight+1.0);
            float saturation=mx>0.00001 ? delta/mx : 0.0;
            saturation=clamp(saturation*(1.0+sat/100.0*weight),0.0,1.0);
            float value=max(mx+light/100.0*weight*0.35,0.0);
            vec3 rgb=clamp(abs(fract(vec3(h)+vec3(0.0,2.0/3.0,1.0/3.0))*6.0-3.0)-1.0,0.0,1.0);
            return premultiply(vec4(value*mix(vec3(1.0),rgb,saturation),p.a));
        }
        """)
        tone = CIColorKernel(source:"""
        kernel vec4 develop(__sample p, float exposure, float brightness, float contrast, float highlights, float shadows, float whites, float blacks, float saturation, float vibrance, float warmth, float tint, float dehaze, float fade, float gradeWarm, float gradeCool) {
            vec3 c = unpremultiply(p).rgb;
            c *= pow(2.0,exposure);
            float l = dot(c,vec3(0.2126,0.7152,0.0722));
            float lo = 1.0-smoothstep(0.0,0.65,l);
            float hi = smoothstep(0.35,1.0,l);
            c += shadows*lo*0.45 + highlights*hi*0.35;
            c += whites*smoothstep(0.65,1.0,l)*0.3 + blacks*(1.0-smoothstep(0.0,0.35,l))*0.25;
            c = (c-0.5)*(1.0+contrast*0.7)+0.5+brightness*0.25;
            c *= vec3(1.0+warmth*0.17,1.0-tint*0.12,1.0-warmth*0.17);
            c += vec3(gradeWarm*hi*0.08,0.0,gradeCool*lo*0.08);
            c = (c-dehaze*0.09)/(1.0-dehaze*0.18);
            float lum = dot(c,vec3(0.2126,0.7152,0.0722));
            float chroma = max(c.r,max(c.g,c.b))-min(c.r,min(c.g,c.b));
            c = mix(vec3(lum),c,max(0.0,1.0+saturation+vibrance*(1.0-chroma)*0.7));
            c = mix(c,vec3(0.12)+c*0.88,fade);
            return premultiply(vec4(max(c,vec3(0.0)),p.a));
        }
        """)
        maskKernel = CIColorKernel(source:"""
        kernel vec4 area(float w,float h,float x0,float y0,float x1,float y1,float feather,float radial,float invert) {
            vec2 p = destCoord()/vec2(w,h);
            vec2 a = vec2(x0,1.0-y0); vec2 b = vec2(x1,1.0-y1);
            float v;
            if (radial > 0.5) {
                vec2 center = (a+b)*0.5;
                vec2 radius = max(abs(b-a)*0.5,vec2(0.01));
                float d = length((p-center)/radius);
                v = 1.0-smoothstep(max(0.0,1.0-feather),1.0,d);
            } else {
                vec2 ab=b-a;
                v=clamp(dot(p-a,ab)/max(dot(ab,ab),0.00001),0.0,1.0);
                v=smoothstep(0.0,1.0,v);
            }
            if (invert>0.5) v=1.0-v;
            return vec4(v,v,v,1.0);
        }
        """)
        detailKernel = CIColorKernel(source:"""
        kernel vec4 detail(__sample source,__sample soft,float amount,float threshold,float mode) {
            vec3 c=unpremultiply(source).rgb; vec3 b=unpremultiply(soft).rgb;
            float edge=length(c-b);
            float gate=smoothstep(threshold,threshold+0.025,edge);
            if (mode>0.5) {
                float l=dot(c,vec3(0.2126,0.7152,0.0722));
                float bl=dot(b,vec3(0.2126,0.7152,0.0722));
                c=mix(c,vec3(l)+(b-vec3(bl)),amount*(1.0-gate));
            } else { c=c+(c-b)*amount*gate; }
            return premultiply(vec4(max(c,vec3(0.0)),source.a));
        }
        """)
    }
    private func applyTone(_ image:CIImage,_ d:Development) -> CIImage {
        let args:[Any] = [image,d.exposure,d.brightness/150,d.contrast/100,d.highlights/100,d.shadows/100,d.whites/100,d.blacks/100,d.saturation/100,d.vibrance/100,d.warmth/100,d.tint/100,d.dehaze/100,d.fade/100,d.gradeWarm/100,d.gradeCool/100]
        return tone?.apply(extent:image.extent,arguments:args) ?? image
    }
    func maskImage(_ m:LocalMask, extent:CGRect) -> CIImage {
        if m.kind != "brush" && m.kind != "ai" {
            return maskKernel?.apply(extent:extent,arguments:[extent.width,extent.height,m.start.x,m.start.y,m.end.x,m.end.y,max(0.01,m.feather),m.kind == "radial" ? 1.0 : 0.0,m.inverted ? 1.0 : 0.0]) ?? CIImage(color:.black).cropped(to:extent)
        }
        var geometry=m;geometry.exposure=0;geometry.warmth=0;geometry.saturation=0;geometry.adjustments=nil;geometry.name="";geometry.enabled=true
        let encoder=JSONEncoder();encoder.outputFormatting = .sortedKeys
        var key=(try? encoder.encode(geometry)) ?? Data()
        key.append(Data("aspect:\(extent.width/extent.height)".utf8))
        if let bitmap=matteCache.object(forKey:key as NSData) {
            return bitmap.transformed(by:CGAffineTransform(scaleX:extent.width/bitmap.extent.width,y:extent.height/bitmap.extent.height)).cropped(to:extent)
        }
        // Fixed mask resolution makes cache entries reusable for thumbnails and full-size export.
        let width = max(1,Int(1024*extent.width/max(extent.width,extent.height))); let height=max(1,Int(Double(width)*extent.height/extent.width))
        let format=UIGraphicsImageRendererFormat(); format.scale=1; format.opaque=true
        let bitmap=UIGraphicsImageRenderer(size:CGSize(width:width,height:height),format:format).image { ctx in
            UIColor.black.setFill(); ctx.fill(CGRect(x:0,y:0,width:width,height:height))
            if let data=m.rasterPNG,let image=UIImage(data:data),let points=m.rasterCorners,points.count==3 {
                let a=points[0],b=points[1],d=points[2],c=ctx.cgContext
                c.saveGState()
                c.concatenate(CGAffineTransform(a:(b.x-a.x)*Double(width),b:(b.y-a.y)*Double(height),c:(d.x-a.x)*Double(width),d:(d.y-a.y)*Double(height),tx:a.x*Double(width),ty:a.y*Double(height)))
                image.draw(in:CGRect(x:0,y:0,width:1,height:1));c.restoreGState()
            }
            for stroke in m.strokes {
                guard let first=stroke.points.first else { continue }
                let c=ctx.cgContext; c.setStrokeColor((stroke.erase ? UIColor.black : UIColor.white).cgColor)
                c.setFillColor((stroke.erase ? UIColor.black : UIColor.white).cgColor)
                c.setLineWidth(stroke.radius*Double(width)*2); c.setLineCap(.round); c.setLineJoin(.round)
                c.move(to:CGPoint(x:first.x*Double(width),y:first.y*Double(height)))
                for p in stroke.points.dropFirst() { c.addLine(to:CGPoint(x:p.x*Double(width),y:p.y*Double(height))) }
                if stroke.points.count == 1 { c.fillEllipse(in:CGRect(x:first.x*Double(width)-stroke.radius*Double(width),y:first.y*Double(height)-stroke.radius*Double(width),width:stroke.radius*Double(width)*2,height:stroke.radius*Double(width)*2)) }
                else { c.strokePath() }
            }
        }
        var result=CIImage(cgImage:bitmap.cgImage!).applyingFilter("CIGaussianBlur",parameters:[kCIInputRadiusKey:m.feather*12]).cropped(to:CGRect(x:0,y:0,width:width,height:height))
        if m.inverted { result=result.applyingFilter("CIColorInvert") }
        if let cg=context.createCGImage(result,from:result.extent) {
            let cached=CIImage(cgImage:cg)
            matteCache.setObject(cached,forKey:key as NSData,cost:width*height*4)
            result=cached
        }
        return result.transformed(by:CGAffineTransform(scaleX:extent.width/CGFloat(width),y:extent.height/CGFloat(height))).cropped(to:extent)
    }
    func process(_ source:CIImage,state:NativeState) -> CIImage {
        let d=state.development; let extent=source.extent; let scale=extent.width/1200
        var optical=applyRepairs(source,state.advanced?.repairs ?? [])
        if d.lens != 0 {
            optical=lensWarp?.apply(extent:extent,roiCallback:{ _,r in r.insetBy(dx:-extent.width*0.2,dy:-extent.height*0.2) },image:source.clampedToExtent(),arguments:[extent.width,extent.height,d.lens/100]) ?? source
        }
        if d.redFringe != 0 || d.blueFringe != 0 {
            func channel(_ value:Double)->CIImage {
                let k=1+value/10000
                return optical.clampedToExtent().transformed(by:CGAffineTransform(translationX:-extent.midX,y:-extent.midY)).transformed(by:CGAffineTransform(scaleX:k,y:k)).transformed(by:CGAffineTransform(translationX:extent.midX,y:extent.midY))
            }
            optical=channels?.apply(extent:extent,arguments:[optical,channel(d.redFringe),channel(d.blueFringe)]) ?? optical
        }
        var image=applyTone(optical,d)
        if d.curveShadows != 0 || d.curveMid != 0 || d.curveHighlights != 0 {
            image=image.applyingFilter("CIToneCurve",parameters:["inputPoint0":CIVector(x:0,y:0),"inputPoint1":CIVector(x:0.25,y:0.25+d.curveShadows/400),"inputPoint2":CIVector(x:0.5,y:0.5+d.curveMid/400),"inputPoint3":CIVector(x:0.75,y:0.75+d.curveHighlights/400),"inputPoint4":CIVector(x:1,y:1)])
        }
        if let a=state.advanced {
            for (index,points) in a.curves.prefix(4).enumerated() where points.count == 5 {
                let values=points.map { CIVector(x:$0.x,y:$0.y) }
                image=curveKernel?.apply(extent:extent,arguments:[image]+values+[Double(index)]) ?? image
            }
            if a.grades.count==3 {
                let colors=a.grades.map { g -> CIVector in
                    let color=UIColor(hue:g.hue/360,saturation:1,brightness:1,alpha:1)
                    var r:CGFloat=0,b:CGFloat=0,green:CGFloat=0,alpha:CGFloat=0;color.getRed(&r,green:&green,blue:&b,alpha:&alpha)
                    let mean=(r+green+b)/3;let k=g.saturation/100*0.22;let l=g.luminance/100*0.15
                    return CIVector(x:(r-mean)*k+l,y:(green-mean)*k+l,z:(b-mean)*k+l)
                }
                image=gradeKernel?.apply(extent:extent,arguments:[image]+colors+[a.balance/100]) ?? image
            }
        }
        for (index,band) in d.mix.enumerated() where band != ColorBand() {
            let targets=[0.0,0.0833,0.1667,0.3333,0.5,0.6667,0.75,0.9167]
            image=colorMix?.apply(extent:extent,arguments:[image,targets[min(index,7)],band.hue,band.saturation,band.luminance]) ?? image
        }
        if d.hue != 0 { image=image.applyingFilter("CIHueAdjust",parameters:[kCIInputAngleKey:d.hue * .pi/180]) }
        if d.noise > 0 {
            image=image.applyingFilter("CINoiseReduction",parameters:["inputNoiseLevel":d.noise/100*0.08*(1-d.noiseDetail/150),"inputSharpness":d.noiseContrast/100*0.5])
        }
        if d.colorNoise > 0 {
            let soft=image.clampedToExtent().applyingFilter("CIGaussianBlur",parameters:[kCIInputRadiusKey:(1+d.colorSmooth/15)*scale]).cropped(to:extent)
            image=detailKernel?.apply(extent:extent,arguments:[image,soft,d.colorNoise/100,0.01+d.colorDetail/100*0.1,1.0]) ?? image
        }
        for (amount,radius,threshold) in [(d.texture/80,1.5*scale,0.0),(d.clarity/70,12*scale,0.0),(d.sharpness/40*(0.5+d.sharpDetail/100),d.sharpRadius*scale,d.sharpMask/100*0.12)] where amount != 0 {
            let soft=image.clampedToExtent().applyingFilter("CIGaussianBlur",parameters:[kCIInputRadiusKey:max(0.1,radius)]).cropped(to:extent)
            image=detailKernel?.apply(extent:extent,arguments:[image,soft,amount,threshold,0.0]) ?? image
        }
        if d.blur > 0 { image=image.clampedToExtent().applyingFilter("CIGaussianBlur",parameters:[kCIInputRadiusKey:d.blur/5*scale]).cropped(to:extent) }
        if d.vignette != 0 { image=image.applyingFilter("CIVignette",parameters:[kCIInputIntensityKey:-d.vignette/50,kCIInputRadiusKey:extent.width*(0.15+d.vignetteMid/100*0.6)]) }
        if d.grain > 0 {
            let noise=CIFilter(name:"CIRandomGenerator")!.outputImage!.cropped(to:extent).applyingFilter("CIColorControls",parameters:[kCIInputSaturationKey:0])
            let alpha=noise.applyingFilter("CIColorMatrix",parameters:["inputAVector":CIVector(x:0,y:0,z:0,w:d.grain/500)])
            image=alpha.applyingFilter("CISourceOverCompositing",parameters:[kCIInputBackgroundImageKey:image])
        }
        for m in state.masks where m.enabled {
            var local=m.adjustments ?? Development(); local.exposure=m.exposure; local.warmth=m.warmth; local.saturation=m.saturation
            let adjusted=process(image,state:NativeState(development:local))
            image=adjusted.applyingFilter("CIBlendWithMask",parameters:[kCIInputBackgroundImageKey:image,kCIInputMaskImageKey:maskImage(m,extent:extent)])
        }
        if let a=state.advanced {
            if a.horizontal != 0 || a.vertical != 0 {
                let x=a.horizontal/100*extent.width*0.2,y=a.vertical/100*extent.height*0.2
                image=image.applyingFilter("CIPerspectiveTransform",parameters:["inputTopLeft":CIVector(x:x,y:extent.height-y),"inputTopRight":CIVector(x:extent.width-x,y:extent.height+y),"inputBottomLeft":CIVector(x:-x,y:y),"inputBottomRight":CIVector(x:extent.width+x,y:-y)]).cropped(to:extent)
            }
            if a.angle != 0 {
                let t=CGAffineTransform(translationX:extent.midX,y:extent.midY).rotated(by:a.angle * .pi/180).translatedBy(x:-extent.midX,y:-extent.midY)
                image=image.clampedToExtent().transformed(by:t).cropped(to:extent)
            }
        }
        return image.cropped(to:extent)
    }
    func applyRepairs(_ source:CIImage,_ spots:[RepairSpot])->CIImage {
        var image=source;let e=source.extent
        for spot in spots {
            let from=CGPoint(x:spot.source.x*e.width,y:(1-spot.source.y)*e.height),to=CGPoint(x:spot.destination.x*e.width,y:(1-spot.destination.y)*e.height)
            let r=max(1,spot.radius*e.width)
            var patch=image.clampedToExtent().transformed(by:CGAffineTransform(translationX:to.x-from.x,y:to.y-from.y)).cropped(to:e)
            if spot.heal {
                let local=image.clampedToExtent().applyingFilter("CIGaussianBlur",parameters:[kCIInputRadiusKey:r*0.6])
                let soft=patch.clampedToExtent().applyingFilter("CIGaussianBlur",parameters:[kCIInputRadiusKey:r*0.6])
                // Frequency separation transfers texture while retaining destination illumination.
                patch=healKernel?.apply(extent:e,arguments:[patch,soft,local]) ?? patch
            }
            let matte=CIFilter(name:"CIRadialGradient",parameters:["inputCenter":CIVector(cgPoint:to),"inputRadius0":r*0.6,"inputRadius1":r,"inputColor0":CIColor.white,"inputColor1":CIColor.black])!.outputImage!.cropped(to:e)
            image=patch.applyingFilter("CIBlendWithMask",parameters:[kCIInputBackgroundImageKey:image,kCIInputMaskImageKey:matte])
        }
        return image
    }
    func renderOriginal(_ url:URL?,fallback:CGImage,state:NativeState,maxSide:CGFloat=1600) throws -> UIImage {
        guard let url else { return try render(fallback,state:state) }
        var decodeState=NativeState(development:Development(),raw:state.raw,sourceEdits:state.sourceEdits)
        decodeState.advanced=AdvancedSettings();decodeState.advanced!.repairs=state.advanced?.repairs ?? []
        decodeState.development.exposure=state.raw == nil ? 0 : state.development.exposure
        let encoder=JSONEncoder();encoder.outputFormatting = .sortedKeys
        let key=(url.path+"-\(maxSide)-"+String(data:try encoder.encode(decodeState),encoding:.utf8)!) as NSString
        let input:CIImage
        if let cached=rawCache.object(forKey:key) { input=cached } else {
            let decoded=try OriginalPipeline.image(url,state:decodeState,maxSide:maxSide)
            guard let cg=context.createCGImage(decoded,from:decoded.extent,format:.RGBAh,colorSpace:OriginalPipeline.linear) else { throw EditorError(message:"Original-Vorschau konnte nicht berechnet werden.") }
            input=CIImage(cgImage:cg);rawCache.setObject(input,forKey:key,cost:cg.width*cg.height*8)
        }
        var adjusted=state;adjusted.advanced?.repairs=[];if state.raw != nil { adjusted.development.exposure=0 }
        let output=process(input,state:adjusted)
        guard let cg=context.createCGImage(output,from:output.extent,format:.RGBA8,colorSpace:CGColorSpace(name:CGColorSpace.displayP3)!) else { throw EditorError(message:"Vorschau fehlgeschlagen.") }
        return UIImage(cgImage:cg)
    }
    func render(_ base:CGImage,state:NativeState) throws -> UIImage {
        guard tone != nil, maskKernel != nil, detailKernel != nil, colorMix != nil, lensWarp != nil, channels != nil else { throw EditorError(message:"Die GPU-Bildfilter konnten nicht initialisiert werden.") }
        let result=process(CIImage(cgImage:base),state:state)
        guard let cg=context.createCGImage(result,from:result.extent) else { throw EditorError(message:"Die Vorschau konnte nicht berechnet werden.") }
        return UIImage(cgImage:cg)
    }
    func export(_ source:URL,to destination:URL,state:NativeState,format:String,original:Bool=false,options:ExportOptions=ExportOptions()) throws {
        let input:CIImage
        if original { input=try OriginalPipeline.image(source,state:state,maxSide:nil) }
        else {
            guard let loaded=CIImage(contentsOf:source,options:[.applyOrientationProperty:true]) else { throw EditorError(message:"Exportquelle konnte nicht geladen werden.") };input=loaded
        }
        var adjusted=state;if original { adjusted.advanced?.repairs=[] };if original && state.raw != nil { adjusted.development.exposure=0 }
        var output=process(input,state:adjusted)
        if options.maxSide>0 {
            let scale=min(1,Double(options.maxSide)/max(output.extent.width,output.extent.height))
            if scale<1 {output=output.applyingFilter("CILanczosScaleTransform",parameters:[kCIInputScaleKey:scale,kCIInputAspectRatioKey:1])}
        }
        output=output.cropped(to:CGRect(x:output.extent.minX,y:output.extent.minY,width:floor(output.extent.width),height:floor(output.extent.height)))
        if !options.watermark.isEmpty {
            let width=output.extent.width,height=output.extent.height
            let font=UIFont.systemFont(ofSize:max(10,min(width,height)*0.035),weight:.medium)
            let text=String(options.watermark.prefix(80)) as NSString
            let attributes:[NSAttributedString.Key:Any]=[.font:font,.foregroundColor:UIColor.white,.strokeColor:UIColor.black,.strokeWidth:-2]
            let textSize=text.size(withAttributes:attributes)
            let scale=min(1,width*0.85/max(1,textSize.width))
            let format=UIGraphicsImageRendererFormat();format.scale=1
            let image=UIGraphicsImageRenderer(size:CGSize(width:ceil(textSize.width+8),height:ceil(textSize.height+8)),format:format).image {_ in text.draw(at:CGPoint(x:4,y:4),withAttributes:attributes)}
            if let cg=image.cgImage {
                let mark=CIImage(cgImage:cg).transformed(by:CGAffineTransform(scaleX:scale,y:scale))
                let inset=min(width,height)*0.025
                output=mark.transformed(by:CGAffineTransform(translationX:output.extent.maxX-mark.extent.width-inset,y:output.extent.minY+inset)).composited(over:output).cropped(to:output.extent)
            }
        }
        output=output.settingProperties([:])
        let space=CGColorSpace(name:options.colorSpace == "P3" ? CGColorSpace.displayP3 : CGColorSpace.sRGB)!
        switch format {
        case "png": try context.writePNGRepresentation(of:output,to:destination,format:.RGBA8,colorSpace:space)
        case "tiff": try context.writeTIFFRepresentation(of:output,to:destination,format:.RGBA16,colorSpace:space,options:[:])
        case "hdr": try context.writeHEIF10Representation(of:output,to:destination,colorSpace:CGColorSpace(name:CGColorSpace.itur_2100_HLG)!,options:[:])
        default: try context.writeJPEGRepresentation(of:output,to:destination,colorSpace:space,options:[kCGImageDestinationLossyCompressionQuality as CIImageRepresentationOption:min(1,max(0.1,options.quality))])
        }
    }
}
