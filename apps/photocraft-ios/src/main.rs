//! Native iPadOS entry point.
//!
//! The application stays in Rust: Xcode only creates and signs the `.app` bundle, while this
//! executable enters UIKit through eframe/winit and renders through wgpu/Metal.
#![deny(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(any(target_os = "ios", test))]
mod hover;
#[cfg(any(target_os = "ios", test))]
mod safe_area;
#[cfg(target_os = "ios")]
mod scene;
#[cfg(target_os = "ios")]
mod shell;

#[cfg(target_os = "ios")]
fn main() -> eframe::Result {
    use photocraft_engine::Session;
    use photocraft_ui_egui::{PhotocraftApp, Services, theme::ThemeKind};

    scene::register();
    let mut options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_title("PhotoCraft"),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    photocraft_ui_egui::gpu_canvas::use_adapter_limits(&mut options.wgpu_options.wgpu_setup);

    eframe::run_native(
        "PhotoCraft",
        options,
        Box::new(|cc| {
            scene::attach_window(cc)?;
            scene::install_pencil_hover(cc, &cc.egui_ctx)?;
            PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
            let mut app = PhotocraftApp::new(Session::new(), Services::default());
            app.set_theme(&cc.egui_ctx, ThemeKind::Pro);
            if let Some(render_state) = cc.wgpu_render_state.clone() {
                log::info!("photocraft-ios: wgpu backend {:?}", render_state.adapter.get_info().backend);
                app.set_wgpu(render_state);
            }
            Ok(Box::new(shell::IosShell(app)))
        }),
    )
}

#[cfg(not(target_os = "ios"))]
fn main() {
    eprintln!("photocraft-ios must be built through the iPadOS Xcode project");
}
