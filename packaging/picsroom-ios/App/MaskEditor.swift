import SwiftUI
import CoreImage

struct MaskEditor: View {
    @Binding var mask: LocalMask
    let imageSize: CGSize
    let brushRadius: Double
    let erase: Bool
    let overlay: Bool
    var pencilOnly = false
    let changed: () -> Void
    let finished: () -> Void
    @State private var stroke: MaskStroke?
    @State private var origin: MaskPoint?
    @State private var initialGeometry: (MaskPoint,MaskPoint)?
    @State private var automaticOverlay:UIImage?
    private var overlayState:LocalMask {
        var m=mask;m.exposure=0;m.warmth=0;m.saturation=0;return m
    }
    var body: some View {
        GeometryReader { geo in
            let scale=min(geo.size.width/imageSize.width,geo.size.height/imageSize.height)
            let size=CGSize(width:imageSize.width*scale,height:imageSize.height*scale)
            let rect=CGRect(x:(geo.size.width-size.width)/2,y:(geo.size.height-size.height)/2,width:size.width,height:size.height)
            ZStack {
                if overlay {
                    if mask.kind == "ai",let image=automaticOverlay {
                        Image(uiImage:image).resizable().frame(width:size.width,height:size.height).position(x:rect.midX,y:rect.midY)
                    } else if mask.kind == "radial" {
                        Ellipse().fill(.red.opacity(0.22)).overlay(Ellipse().stroke(.white,lineWidth:1))
                            .frame(width:abs(mask.end.x-mask.start.x)*size.width,height:abs(mask.end.y-mask.start.y)*size.height)
                            .position(x:rect.minX+(mask.start.x+mask.end.x)/2*size.width,y:rect.minY+(mask.start.y+mask.end.y)/2*size.height)
                    } else if mask.kind == "linear" {
                        Path { p in p.move(to:point(mask.start,rect)); p.addLine(to:point(mask.end,rect)) }.stroke(.white,lineWidth:2)
                    } else {
                        Canvas { context,_ in
                            for item in mask.strokes + (stroke.map { [$0] } ?? []) {
                                var path=Path(); guard let first=item.points.first else { continue }
                                path.move(to:point(first,rect)); for p in item.points.dropFirst() { path.addLine(to:point(p,rect)) }
                                context.stroke(path,with:.color(item.erase ? .cyan.opacity(0.5) : .red.opacity(0.4)),style:StrokeStyle(lineWidth:item.radius*size.width*2,lineCap:.round,lineJoin:.round))
                            }
                        }
                    }
                    if mask.kind == "ai",let stroke {
                        Path { path in
                            if let first=stroke.points.first { path.move(to:point(first,rect));for p in stroke.points.dropFirst() { path.addLine(to:point(p,rect)) } }
                        }.stroke(stroke.erase ? Color.cyan.opacity(0.5) : Color.red.opacity(0.4),style:StrokeStyle(lineWidth:stroke.radius*size.width*2,lineCap:.round,lineJoin:.round))
                    }
                    if mask.kind != "brush" && mask.kind != "ai" {
                        ForEach(0..<2,id:\.self) { index in
                            Circle().fill(.white).frame(width:12,height:12).position(point(index == 0 ? mask.start : mask.end,rect))
                        }
                    }
                }
                MaskInputSurface(pencilOnly:pencilOnly,strokeCount:mask.strokes.count) { location,phase in
                    let p=MaskPoint(x:min(1,max(0,location.x/size.width)),y:min(1,max(0,location.y/size.height)))
                    if phase == .cancelled {
                        stroke=nil;origin=nil
                        if let initialGeometry { mask.start=initialGeometry.0;mask.end=initialGeometry.1;changed() }
                        initialGeometry=nil;return
                    }
                    if phase == .began { initialGeometry=(mask.start,mask.end) }
                    if mask.kind == "brush" || mask.kind == "ai" {
                        if stroke == nil { stroke=MaskStroke(points:[],radius:brushRadius,erase:erase) }
                        if stroke!.points.count<1200 { stroke!.points.append(p) }
                    } else {
                        if origin == nil { origin=p;mask.start=p }
                        mask.end=p;changed()
                    }
                    if phase == .ended {
                        if let stroke { mask.strokes.append(stroke);self.stroke=nil }
                        origin=nil;initialGeometry=nil;changed();finished()
                    }
                }.frame(width:size.width,height:size.height).position(x:rect.midX,y:rect.midY)
            }.coordinateSpace(name:"maskCanvas")
                .task(id:overlayState) {
                    guard mask.kind == "ai" else { return }
                    let state=overlayState
                    let h=640*imageSize.height/imageSize.width
                    let rendered=await Task.detached(priority:.userInitiated) { () -> UIImage? in
                        let extent=CGRect(x:0,y:0,width:640,height:h)
                        let renderer=DevelopRenderer.shared
                        let matte=renderer.maskImage(state,extent:extent)
                        let red=CIImage(color:CIColor(red:1,green:0.15,blue:0.2,alpha:0.38)).cropped(to:extent)
                        let output=red.applyingFilter("CIBlendWithMask",parameters:[kCIInputBackgroundImageKey:CIImage(color:.clear).cropped(to:extent),kCIInputMaskImageKey:matte])
                        return renderer.context.createCGImage(output,from:extent).map { UIImage(cgImage:$0) }
                    }.value
                    if !Task.isCancelled { automaticOverlay=rendered }
                }
        }
    }
    private func point(_ p:MaskPoint,_ r:CGRect)->CGPoint { CGPoint(x:r.minX+p.x*r.width,y:r.minY+p.y*r.height) }
    private func normalized(_ p:CGPoint,_ r:CGRect)->MaskPoint { MaskPoint(x:min(1,max(0,(p.x-r.minX)/r.width)),y:min(1,max(0,(p.y-r.minY)/r.height))) }
}

/// System slider keeps VoiceOver/keyboard support with a smaller hollow thumb.
struct StudioSlider: UIViewRepresentable {
    @Binding var value: Double
    let range: ClosedRange<Double>
    var identifier: String
    var label: String
    var onEditing: (Bool) -> Void
    func makeCoordinator()->Coordinator { Coordinator(self) }
    func makeUIView(context:Context)->UISlider {
        let slider=ThinStudioSlider(); slider.minimumValue=Float(range.lowerBound); slider.maximumValue=Float(range.upperBound)
        let image=UIGraphicsImageRenderer(size:CGSize(width:24,height:24)).image { ctx in
            UIColor(white:0.12,alpha:0.9).setFill(); ctx.cgContext.fillEllipse(in:CGRect(x:4,y:4,width:16,height:16))
            UIColor(white:0.85,alpha:1).setStroke(); ctx.cgContext.setLineWidth(2);ctx.cgContext.strokeEllipse(in:CGRect(x:4,y:4,width:16,height:16))
        }
        slider.setThumbImage(image,for:.normal); slider.minimumTrackTintColor=UIColor(white:0.7,alpha:1); slider.maximumTrackTintColor=UIColor(white:0.3,alpha:1)
        slider.addTarget(context.coordinator,action:#selector(Coordinator.change(_:)),for:.valueChanged)
        slider.addTarget(context.coordinator,action:#selector(Coordinator.begin),for:.touchDown)
        slider.addTarget(context.coordinator,action:#selector(Coordinator.end),for:[.touchUpInside,.touchUpOutside,.touchCancel])
        let doubleTap=UITapGestureRecognizer(target:context.coordinator,action:#selector(Coordinator.reset(_:)))
        doubleTap.numberOfTapsRequired=2;doubleTap.cancelsTouchesInView=true
        slider.addGestureRecognizer(doubleTap)
        slider.accessibilityHint="Doppeltippen setzt den Wert auf null zurück."
        slider.accessibilityCustomActions=[UIAccessibilityCustomAction(name:"Auf null zurücksetzen",target:context.coordinator,selector:#selector(Coordinator.accessibleReset))]
        slider.accessibilityIdentifier=identifier; slider.accessibilityLabel=label
        return slider
    }
    func updateUIView(_ slider:UISlider,context:Context) {
        context.coordinator.parent=self
        if !slider.isTracking { slider.value=Float(value) }
    }
    class Coordinator: NSObject {
        var parent:StudioSlider
        init(_ parent:StudioSlider) { self.parent=parent }
        @objc func change(_ s:UISlider) { parent.value=Double(s.value); if !s.isTracking { parent.onEditing(false) } }
        private func resetValue() { parent.onEditing(true);parent.value=min(parent.range.upperBound,max(parent.range.lowerBound,0));parent.onEditing(false) }
        @objc func reset(_ gesture:UITapGestureRecognizer) {
            guard gesture.state == .ended else { return }
            (gesture.view as? UISlider)?.setValue(Float(min(parent.range.upperBound,max(parent.range.lowerBound,0))),animated:true)
            resetValue()
        }
        @objc func accessibleReset()->Bool { resetValue();return true }
        @objc func begin() { parent.onEditing(true) }
        @objc func end() { parent.onEditing(false) }
    }
}

struct WhiteBalanceSampler: View {
    let imageSize:CGSize
    var instruction="Auf eine neutrale graue oder weiße Stelle tippen"
    let selected:(MaskPoint)->Void
    var body:some View {
        GeometryReader { geo in
            let scale=min(geo.size.width/imageSize.width,geo.size.height/imageSize.height)
            let size=CGSize(width:imageSize.width*scale,height:imageSize.height*scale)
            Color.clear.contentShape(Rectangle()).frame(width:size.width,height:size.height)
                .overlay(alignment:.top) { Text(instruction).font(.caption).padding(8).background(.black.opacity(0.7)).allowsHitTesting(false) }
                .gesture(DragGesture(minimumDistance:0).onEnded { value in
                    selected(MaskPoint(x:min(1,max(0,value.location.x/size.width)),y:min(1,max(0,value.location.y/size.height))))
                })
                .position(x:geo.size.width/2,y:geo.size.height/2)
        }
    }
}

final class ThinStudioSlider: UISlider {
    override func trackRect(forBounds bounds:CGRect)->CGRect {
        let rect=super.trackRect(forBounds:bounds)
        return CGRect(x:rect.minX,y:rect.midY-1,width:rect.width,height:2)
    }
}

/// One input stroke at a time; Pencil-only mode ignores finger and palm contacts.
private struct MaskInputSurface:UIViewRepresentable {
    let pencilOnly:Bool
    let strokeCount:Int
    let event:(CGPoint,UITouch.Phase)->Void
    func makeUIView(context:Context)->MaskTouchView { MaskTouchView() }
    func updateUIView(_ view:MaskTouchView,context:Context) { view.pencilOnly=pencilOnly;view.event=event;view.accessibilityValue="\(strokeCount)" }
}
private final class MaskTouchView:UIView {
    var pencilOnly=false
    var event:((CGPoint,UITouch.Phase)->Void)?
    private weak var activeTouch:UITouch?
    override init(frame:CGRect) {
        super.init(frame:frame);backgroundColor = .clear;isMultipleTouchEnabled=true
        accessibilityIdentifier="mask-input";isAccessibilityElement=true;accessibilityLabel="Maske zeichnen"
    }
    required init?(coder:NSCoder) { nil }
    override func touchesBegan(_ touches:Set<UITouch>,with uiEvent:UIEvent?) {
        guard activeTouch == nil else { return }
        guard let touch=touches.first(where:{$0.type == .pencil}) ?? (pencilOnly ? nil : touches.first(where:{$0.type == .direct || $0.type == .indirectPointer})) else { return }
        activeTouch=touch;event?(touch.location(in:self),.began)
    }
    override func touchesMoved(_ touches:Set<UITouch>,with uiEvent:UIEvent?) {
        guard let touch=activeTouch,touches.contains(touch) else { return }
        for sample in uiEvent?.coalescedTouches(for:touch) ?? [touch] { event?(sample.location(in:self),.moved) }
    }
    override func touchesEnded(_ touches:Set<UITouch>,with uiEvent:UIEvent?) {
        guard let touch=activeTouch,touches.contains(touch) else { return }
        event?(touch.location(in:self),.ended);activeTouch=nil
    }
    override func touchesCancelled(_ touches:Set<UITouch>,with uiEvent:UIEvent?) {
        guard let touch=activeTouch,touches.contains(touch) else { return }
        event?(touch.location(in:self),.cancelled);activeTouch=nil
    }
}
