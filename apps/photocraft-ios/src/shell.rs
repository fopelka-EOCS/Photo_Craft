//! iPad-only integration; the shared editor and its input coordinates stay unchanged.
use eframe::{App, Frame, egui};
use photocraft_ui_egui::PhotocraftApp;

pub struct IosShell(pub PhotocraftApp);

impl App for IosShell {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut Frame) {
        self.0.logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut Frame) {
        crate::safe_area::show_editor(ui, |ui| self.0.ui(ui, frame));
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        self.0.raw_input_hook(ctx, input);
        crate::scene::drain_pencil_hover(input, ctx.zoom_factor());
        // winit can sample zero insets before our scene attachment, with no later resize.
        // Query the actual retained window each frame, including after rotation or resume.
        if let Some(insets) = crate::scene::safe_area_insets() {
            let [top, left, right, bottom] = crate::safe_area::logical_insets(insets, ctx.zoom_factor());
            input.safe_area_insets = Some(egui::SafeAreaInsets(egui::epaint::MarginF32 { top, left, right, bottom }));
        }
    }
}
