//! Single-window UIKit scene bridge for winit 0.30.
//!
//! UIKit still posts application lifecycle notifications when scenes are enabled, so winit
//! receives its existing resume/suspend events. Do not synthesize duplicate notifications.
//! Remove this adapter when the pinned eframe/winit stack supports scenes itself.
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

use std::cell::RefCell;
use std::io;

use objc2::rc::Retained;
use objc2::{ClassType, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{NSArray, NSNumber, NSObject, NSObjectProtocol};
use objc2_ui_kit::{
    UIApplication, UICoordinateSpace, UIGestureRecognizerState, UIHoverGestureRecognizer, UIScene, UISceneConnectionOptions, UISceneDelegate, UISceneSession,
    UITouchType, UIView, UIWindow, UIWindowScene, UIWindowSceneDelegate,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

thread_local! {
    // Retain the existing winit window across scene disconnection/reconnection. No second
    // renderer, event loop, or window is created, and UIKit objects never cross threads.
    static WINDOW: RefCell<Option<Retained<UIWindow>>> = const { RefCell::new(None) };
    static HOVER: RefCell<crate::hover::HoverInput> = RefCell::new(crate::hover::HoverInput::default());
    // UIKit gesture recognizers do not retain their target. Keep it for the app lifetime.
    static HOVER_TARGET: RefCell<Option<(Retained<PencilHoverTarget>, eframe::egui::Context)>> = const { RefCell::new(None) };
}

define_class!(
    // SAFETY: No custom ivars or Drop; NSObject imposes no subclassing requirements.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "PhotoCraftPencilHoverTarget"]
    struct PencilHoverTarget;

    // SAFETY: NSObjectProtocol has no additional requirements.
    unsafe impl NSObjectProtocol for PencilHoverTarget {}

    impl PencilHoverTarget {
        // SAFETY: Exact target/action ABI for UIHoverGestureRecognizer, invoked on main thread.
        #[unsafe(method(pencilHover:))]
        fn pencil_hover(&self, recognizer: &UIHoverGestureRecognizer) {
            let state = recognizer.state();
            let position = if state == UIGestureRecognizerState::Began || state == UIGestureRecognizerState::Changed {
                let view = recognizer.view();
                let point = recognizer.locationInView(view.as_deref());
                Some([point.x, point.y])
            } else { None };
            HOVER.with(|slot| {
                if let Ok(mut hover) = slot.try_borrow_mut() { hover.update(position); }
            });
            HOVER_TARGET.with(|slot| {
                if let Ok(target) = slot.try_borrow()
                    && let Some((_, ctx)) = target.as_ref() { ctx.request_repaint(); }
            });
        }
    }
);

/// Register Pencil-only proximity input on the rendering view, preserving winit touch events.
pub fn install_pencil_hover(owner: &impl HasWindowHandle, ctx: &eframe::egui::Context) -> Result<(), io::Error> {
    let mtm = MainThreadMarker::new().ok_or_else(|| io::Error::other("Pencil hover must be installed on the main thread"))?;
    let handle = owner.window_handle().map_err(io::Error::other)?;
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else {
        return Err(io::Error::other("Expected UIKit rendering view"));
    };
    // SAFETY: Borrowed UIKit handle keeps this UIView alive; all access is main-thread-only.
    let view = unsafe { handle.ui_view.cast::<UIView>().as_ref() };
    // SAFETY: PencilHoverTarget inherits NSObject's init and has no ivars to initialize.
    let target: Retained<PencilHoverTarget> = unsafe { msg_send![PencilHoverTarget::alloc(mtm), init] };
    // SAFETY: Retained main-thread target implements the exact pencilHover: action above.
    let recognizer = unsafe { UIHoverGestureRecognizer::initWithTarget_action(UIHoverGestureRecognizer::alloc(mtm), Some(&target), Some(sel!(pencilHover:))) };
    recognizer.setAllowedTouchTypes(&NSArray::<NSNumber>::from_slice(&[&NSNumber::numberWithInteger(UITouchType::Pencil.0)]));
    recognizer.setCancelsTouchesInView(false);
    recognizer.setDelaysTouchesBegan(false);
    recognizer.setDelaysTouchesEnded(false);
    HOVER_TARGET.with(|slot| {
        let mut slot = slot.try_borrow_mut().map_err(|_| io::Error::other("Reentrant Pencil hover installation"))?;
        *slot = Some((target, ctx.clone()));
        Ok::<_, io::Error>(())
    })?;
    view.addGestureRecognizer(&recognizer);
    Ok(())
}

pub fn drain_pencil_hover(input: &mut eframe::egui::RawInput, zoom: f32) {
    HOVER.with(|slot| {
        if let Ok(mut hover) = slot.try_borrow_mut() {
            hover.drain(input, zoom);
        }
    });
}

define_class!(
    // SAFETY: NSObject imposes no subclassing requirements; there are no custom ivars or Drop.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "PhotoCraftSceneDelegate"]
    struct SceneDelegate;

    // SAFETY: NSObjectProtocol has no additional requirements.
    unsafe impl NSObjectProtocol for SceneDelegate {}
    // SAFETY: All delegate callbacks use UIKit's exact signatures and run on the main thread.
    unsafe impl UISceneDelegate for SceneDelegate {
        #[unsafe(method(scene:willConnectToSession:options:))]
        fn connect(&self, scene: &UIScene, _session: &UISceneSession, _options: &UISceneConnectionOptions) {
            let Some(scene) = scene.downcast_ref::<UIWindowScene>() else { return };
            let window = WINDOW.with(|slot| slot.try_borrow().ok().and_then(|w| w.clone()));
            if let Some(window) = window {
                attach(&window, scene);
            }
        }
    }
    // SAFETY: Optional window-scene methods are not required for our single-window adapter.
    unsafe impl UIWindowSceneDelegate for SceneDelegate {}
);

/// Register the name referenced by Info.plist before winit enters UIApplicationMain.
pub fn register() {
    let _ = SceneDelegate::class();
}

/// Attach the exact window owned by eframe, rather than selecting a keyboard/system window.
pub fn attach_window(owner: &impl HasWindowHandle) -> Result<(), io::Error> {
    let mtm = MainThreadMarker::new().ok_or_else(|| io::Error::other("UIKit window must be attached on the main thread"))?;
    let handle = owner.window_handle().map_err(io::Error::other)?;
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else {
        return Err(io::Error::other("PhotoCraft expected a UIKit window"));
    };
    // SAFETY: HasWindowHandle guarantees the non-null UIKit UIView remains valid for the
    // borrowed handle's lifetime. We access it only on the main thread and do not take ownership.
    let view = unsafe { handle.ui_view.cast::<UIView>().as_ref() };
    let window = view.window().ok_or_else(|| io::Error::other("The editor view has no UIWindow"))?;
    let scenes = UIApplication::sharedApplication(mtm).connectedScenes();
    let scene =
        scenes.iter().find_map(|scene| scene.downcast::<UIWindowScene>().ok()).ok_or_else(|| io::Error::other("UIKit has not connected a window scene"))?;
    attach(&window, &scene);
    WINDOW.with(|slot| {
        let mut slot = slot.try_borrow_mut().map_err(|_| io::Error::other("Reentrant UIKit window attachment"))?;
        *slot = Some(window);
        Ok(())
    })
}

fn attach(window: &UIWindow, scene: &UIWindowScene) {
    window.setWindowScene(Some(scene));
    // iPadOS 17 is supported; effectiveGeometry is only available on newer systems.
    #[allow(deprecated)]
    window.setFrame(scene.coordinateSpace().bounds());
    window.makeKeyAndVisible();
}

/// Insets of the editor's window in UIKit points (not physical Retina pixels).
pub fn safe_area_insets() -> Option<[f64; 4]> {
    let _mtm = MainThreadMarker::new()?;
    let window = WINDOW.with(|slot| slot.try_borrow().ok().and_then(|w| w.clone()))?;
    let insets = window.safeAreaInsets();
    Some([insets.top, insets.left, insets.right, insets.bottom])
}
