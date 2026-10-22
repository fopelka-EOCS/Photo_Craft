import SwiftUI
import UIKit

struct ZoomCanvas: UIViewRepresentable {
    let image: UIImage
    var onCompare: (Bool) -> Void = { _ in }
    func makeUIView(context: Context) -> PhotoViewport { PhotoViewport() }
    func updateUIView(_ view: PhotoViewport, context: Context) { view.imageView.image = image; view.onCompare=onCompare }
}
final class PhotoViewport: UIScrollView, UIScrollViewDelegate {
    let imageView = UIImageView()
    var onCompare: (Bool) -> Void = { _ in }
    private var previousSize = CGSize.zero
    override init(frame: CGRect) {
        super.init(frame: frame)
        delegate = self
        minimumZoomScale = 1
        maximumZoomScale = 6
        showsVerticalScrollIndicator = false
        showsHorizontalScrollIndicator = false
        imageView.contentMode = .scaleAspectFit
        imageView.accessibilityLabel = "Bildvorschau. Gedrückt halten für Vorher. Mit zwei Fingern zoomen und verschieben."
        imageView.isAccessibilityElement = true
        addSubview(imageView)
        let tap = UITapGestureRecognizer(target: self, action: #selector(resetZoom))
        tap.numberOfTapsRequired = 2
        addGestureRecognizer(tap)
        let hold=UILongPressGestureRecognizer(target:self,action:#selector(compareHeld(_:)))
        hold.minimumPressDuration=0.25; hold.allowableMovement=12
        hold.numberOfTouchesRequired=1; hold.cancelsTouchesInView=false
        addGestureRecognizer(hold)
        accessibilityIdentifier="photo-canvas"
    }
    required init?(coder: NSCoder) { return nil }
    override func layoutSubviews() {
        super.layoutSubviews()
        if previousSize != bounds.size {
            previousSize = bounds.size
            setZoomScale(1, animated: false)
            imageView.frame = CGRect(origin: .zero, size: bounds.size)
            contentSize = bounds.size
        }
    }
    func viewForZooming(in scrollView: UIScrollView) -> UIView? { imageView }
    @objc private func compareHeld(_ gesture:UILongPressGestureRecognizer) {
        switch gesture.state {
        case .began: onCompare(true)
        case .ended,.cancelled,.failed: onCompare(false)
        default: break
        }
    }
    @objc private func resetZoom() { setZoomScale(1, animated: true) }
}
