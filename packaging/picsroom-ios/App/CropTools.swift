import SwiftUI

enum CropGeometry {
    static let ratios:[(String,Double)] = [("1:1",1),("1:2",0.5),("2:3",2.0/3),("3:4",0.75),("4:5",0.8),("5:7",5.0/7),("8,5:11",8.5/11),("9:16",9.0/16),("10:16",10.0/16)]
    static func fit(ratio:Double,image:CGSize,center:CGPoint=CGPoint(x:0.5,y:0.5))->CGRect {
        let r=ratio*image.height/image.width
        let w=min(1,r),h=min(1,1/r)
        return CGRect(x:min(max(0,center.x-w/2),1-w),y:min(max(0,center.y-h/2),1-h),width:w,height:h)
    }
    /// Eight handles: four corners, then left/right/top/bottom midpoint.
    static func resize(_ rect:CGRect,handle:Int,delta:CGSize,ratio:Double?)->CGRect {
        let left=handle==0 || handle==2 || handle==4
        let top=handle==0 || handle==1 || handle==6
        let horizontal=handle<6,vertical=handle<4 || handle>=6
        let dx=delta.width*(left ? -1:1),dy=delta.height*(top ? -1:1)
        if let ratio {
            let anchorX=horizontal ? (left ? rect.maxX:rect.minX):rect.midX
            let anchorY=vertical ? (top ? rect.maxY:rect.minY):rect.midY
            let maxW=horizontal ? (left ? anchorX:1-anchorX):2*min(anchorX,1-anchorX)
            let maxH=vertical ? (top ? anchorY:1-anchorY):2*min(anchorY,1-anchorY)
            let target=horizontal && (!vertical || abs(dx)>=abs(dy*ratio)) ? rect.width+dx : (rect.height+dy)*ratio
            let limit=min(maxW,maxH*ratio)
            let w=min(limit,max(min(0.04,limit),target)),h=w/ratio
            return CGRect(x:horizontal ? (left ? anchorX-w:anchorX):anchorX-w/2,y:vertical ? (top ? anchorY-h:anchorY):anchorY-h/2,width:w,height:h)
        }
        let x0=horizontal && left ? min(rect.maxX-0.04,max(0,rect.minX+delta.width)):rect.minX
        let x1=horizontal && !left ? max(rect.minX+0.04,min(1,rect.maxX+delta.width)):rect.maxX
        let y0=vertical && top ? min(rect.maxY-0.04,max(0,rect.minY+delta.height)):rect.minY
        let y1=vertical && !top ? max(rect.minY+0.04,min(1,rect.maxY+delta.height)):rect.maxY
        return CGRect(x:x0,y:y0,width:x1-x0,height:y1-y0)
    }
}

struct CropOverlay:View {
    @Binding var rect:CGRect
    let imageSize:CGSize
    var ratio:Double?
    @State private var start:CGRect?
    var body:some View {
        GeometryReader { geo in
            let scale=min(geo.size.width/imageSize.width,geo.size.height/imageSize.height)
            let size=CGSize(width:imageSize.width*scale,height:imageSize.height*scale)
            let origin=CGPoint(x:(geo.size.width-size.width)/2,y:(geo.size.height-size.height)/2)
            let frame=CGRect(x:origin.x+rect.minX*size.width,y:origin.y+rect.minY*size.height,width:rect.width*size.width,height:rect.height*size.height)
            ZStack {
                Path {p in p.addRect(CGRect(origin:origin,size:size));p.addRect(frame)}.fill(.black.opacity(0.5),style:FillStyle(eoFill:true)).allowsHitTesting(false)
                Color.clear.contentShape(Rectangle()).frame(width:frame.width,height:frame.height).position(x:frame.midX,y:frame.midY)
                    .gesture(DragGesture().onChanged {value in
                        if start==nil {start=rect};guard let start else {return}
                        rect.origin=CGPoint(x:min(max(0,start.minX+value.translation.width/size.width),1-rect.width),y:min(max(0,start.minY+value.translation.height/size.height),1-rect.height))
                    }.onEnded {_ in start=nil}).accessibilityIdentifier("crop-move")
                Path {p in
                    p.addRect(frame)
                    for index in 1...2 {let f=CGFloat(index)/3;p.move(to:CGPoint(x:frame.minX+frame.width*f,y:frame.minY));p.addLine(to:CGPoint(x:frame.minX+frame.width*f,y:frame.maxY));p.move(to:CGPoint(x:frame.minX,y:frame.minY+frame.height*f));p.addLine(to:CGPoint(x:frame.maxX,y:frame.minY+frame.height*f))}
                }.stroke(.white.opacity(0.8),lineWidth:0.8).allowsHitTesting(false)
                ForEach(0..<8) {handle in
                    let x=handle<4 ? (handle%2==1 ? frame.maxX:frame.minX) : handle==4 ? frame.minX:handle==5 ? frame.maxX:frame.midX
                    let y=handle<4 ? (handle>=2 ? frame.maxY:frame.minY) : handle==6 ? frame.minY:handle==7 ? frame.maxY:frame.midY
                    RoundedRectangle(cornerRadius:2).fill(.white).frame(width:handle==4 || handle==5 ? 4:18,height:handle==6 || handle==7 ? 4:18).frame(width:36,height:36).contentShape(Rectangle()).position(x:x,y:y)
                        .gesture(DragGesture(minimumDistance:0).onChanged {value in
                            if start==nil {start=rect};guard let start else {return}
                            rect=CropGeometry.resize(start,handle:handle,delta:CGSize(width:value.translation.width/size.width,height:value.translation.height/size.height),ratio:ratio.map{$0*imageSize.height/imageSize.width})
                        }.onEnded {_ in start=nil})
                        .accessibilityLabel(handle<4 ? "Ausschnitt Ecke \(handle+1)":"Ausschnitt Kante \(handle-3)").accessibilityIdentifier("crop-handle-\(handle)")
                }
            }.accessibilityElement(children:.contain).accessibilityIdentifier("crop-overlay").accessibilityValue(String(format:"%.4f",rect.width*imageSize.width/(rect.height*imageSize.height)))
        }
    }
}
