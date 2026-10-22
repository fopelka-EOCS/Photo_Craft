//! Linux-only host: transport lifetime and eframe forwarding. Editing stays in ui-egui.

use craft_desktop::appmenu::Bridge;
use photocraft_ui_egui::{PhotocraftApp, desktop_menu};

pub struct App {
    app: PhotocraftApp,
    menu: Option<Bridge>,
    menu_state: Option<u64>,
}

impl App {
    pub fn new(app: PhotocraftApp, cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();
        let menu = cc
            .winit_window()
            .and_then(|window| Bridge::start(window.clone(), super::APP_ID, "PHOTOCRAFT", desktop_menu::snapshot(&app), move || ctx.request_repaint()));
        Self { app, menu, menu_state: None }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        eframe::App::logic(&mut self.app, ctx, frame);
        if let Some(menu) = self.menu.as_mut() {
            for click in menu.take_events() {
                if let Err(error) = desktop_menu::activate(&mut self.app, ctx, click.document, &click.action) {
                    self.app.ui.status = error;
                    self.app.ui.status_error = true;
                }
                self.menu_state = None;
            }
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        eframe::App::raw_input_hook(&mut self.app, ctx, input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        eframe::App::ui(&mut self.app, ui, frame);
        if let Some(menu) = self.menu.as_ref() {
            let state = desktop_menu::state_key(&self.app, ui.ctx());
            if self.menu_state != Some(state) {
                menu.publish(desktop_menu::snapshot(&self.app));
                self.menu_state = Some(state);
            }
        }
    }

    fn on_exit(&mut self) {
        self.menu.take();
        eframe::App::on_exit(&mut self.app);
    }
}
