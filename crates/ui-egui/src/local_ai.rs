//! Asynchronous local generation. Only an explicit button press contacts the assistant.
use crate::PhotocraftApp;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver},
};

pub type Service = Arc<dyn Fn(Value, Arc<AtomicBool>) -> Result<Value, String> + Send + Sync>;
pub struct State {
    pub prompt: String,
    pub models: Vec<String>,
    pub model: String,
    pub size: u32,
    pub seed: u32,
    pub status: String,
    pub error: bool,
    pending: Option<Receiver<Result<Value, String>>>,
    cancel: Arc<AtomicBool>,
    started: Option<std::time::Instant>,
    operation: String,
    previous_prompt: Option<String>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            models: Vec::new(),
            model: String::new(),
            size: 512,
            seed: 42,
            status: "FLUX.2 Klein 4B • Lokal • Hasil dibuka sebagai dokumen baru".into(),
            error: false,
            pending: None,
            cancel: Arc::new(AtomicBool::new(false)),
            started: None,
            operation: String::new(),
            previous_prompt: None,
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn start(app: &mut PhotocraftApp, operation: &str, ctx: &egui::Context) {
    let Some(service) = app.services.local_ai.clone() else { return };
    let state = &mut app.local_ai;
    if state.pending.is_some() {
        return;
    }
    let req = json!({"operation":operation, "prompt":state.prompt, "model":state.model,
        "size":state.size, "seed":state.seed});
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    state.cancel = cancel.clone();
    state.operation = operation.into();
    state.started = Some(std::time::Instant::now());
    state.error = false;
    state.status = match operation {
        "models" => "Menghubungkan AI Assistant…",
        "enhance" => "Menyempurnakan prompt…",
        _ => "Membuat gambar lokal…",
    }
    .into();
    let ctx = ctx.clone();
    // Native service only; the web has no service and never reaches this point.
    match std::thread::Builder::new().name("photocraft-local-ai".into()).spawn(move || {
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| service(req, cancel))).unwrap_or_else(|_| Err("Proses AI berhenti tak terduga".into()));
        let _ = tx.send(result);
        ctx.request_repaint();
    }) {
        Ok(_) => state.pending = Some(rx),
        Err(e) => {
            state.status = e.to_string();
            state.error = true;
        }
    }
}

pub(crate) fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let result = match app.local_ai.pending.as_ref().map(|rx| rx.try_recv()) {
        Some(Ok(result)) => result,
        Some(Err(mpsc::TryRecvError::Disconnected)) => Err("Koneksi proses AI terputus".into()),
        _ => {
            if app.local_ai.pending.is_some() {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }
            return;
        }
    };
    app.local_ai.pending = None;
    if app.local_ai.cancel.load(Ordering::Relaxed) {
        app.local_ai.status = "Dibatalkan".into();
        return;
    }
    let result = result.and_then(|v| {
        match app.local_ai.operation.as_str() {
            "models" => {
                let models: Vec<String> =
                    v.get("models").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
                if models.is_empty() {
                    return Err("Server tidak menyediakan model".into());
                }
                if !models.contains(&app.local_ai.model) {
                    app.local_ai.model = models.first().cloned().unwrap_or_default();
                }
                app.local_ai.models = models;
                app.local_ai.status = "AI Assistant terhubung • 127.0.0.1:1234".into();
            }
            "enhance" => {
                let prompt = v.get("prompt").and_then(Value::as_str).filter(|p| !p.trim().is_empty()).ok_or("AI tidak mengembalikan prompt")?;
                app.local_ai.previous_prompt = Some(std::mem::replace(&mut app.local_ai.prompt, prompt.to_owned()));
                app.local_ai.status = "Prompt siap. Klik Generate untuk membuat gambar.".into();
            }
            _ => {
                app.run("ai.openGeneratedImage", v)?;
                app.sync_views();
                app.ui.status = "Gambar AI dibuat sebagai dokumen baru".into();
                app.ui.status_error = false;
                app.local_ai.status = "Gambar dibuka • Simpan melalui File → Save As".into();
            }
        }
        Ok(())
    });
    if let Err(e) = result {
        app.local_ai.status = e;
        app.local_ai.error = true;
    }
}

pub(crate) fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let t = crate::theme::Tokens::get(&ctx);
    let mut action = None;
    egui::Panel::bottom("local_ai_panel").resizable(true).default_size(174.0).min_size(135.0).frame(egui::Frame::NONE.fill(t.chrome).inner_margin(10)).show(
        ui,
        |ui| {
            let s = &mut app.local_ai;
            let busy = s.pending.is_some();
            ui.horizontal_wrapped(|ui| {
                ui.strong("AI Studio · RoboDhan");
                if ui.add_enabled(!busy, egui::Button::new("Hubungkan AI Assistant")).clicked() {
                    action = Some("models");
                }
                if !s.models.is_empty() {
                    ui.label("Model chat mengikuti pilihan aktif di RoboDhan");
                }
            });
            ui.add_enabled(
                !busy,
                egui::TextEdit::multiline(&mut s.prompt)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .char_limit(4000)
                    .hint_text("Deskripsikan gambar atau foto yang ingin dibuat…"),
            );
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    ui.label("Ukuran");
                    egui::ComboBox::from_id_salt("local-ai-size").selected_text(format!("{} × {}", s.size, s.size)).show_ui(ui, |ui| {
                        for size in [512, 768] {
                            ui.selectable_value(&mut s.size, size, format!("{size} × {size}"));
                        }
                    });
                    ui.label("Seed");
                    ui.add(egui::DragValue::new(&mut s.seed));
                });
                let prompt_ready = !s.prompt.trim().is_empty();
                if ui.add_enabled(!busy && prompt_ready && !s.model.is_empty(), egui::Button::new("Sempurnakan prompt")).clicked() {
                    action = Some("enhance");
                }
                if ui.add_enabled(!busy && s.previous_prompt.is_some(), egui::Button::new("Pulihkan prompt")).clicked()
                    && let Some(previous) = s.previous_prompt.take()
                {
                    s.prompt = previous;
                }
                if ui.add_enabled(!busy && prompt_ready, egui::Button::new("Generate gambar")).clicked() {
                    action = Some("generate");
                }
                if busy {
                    ui.spinner();
                    ui.label(format!("{} detik", s.started.map_or(0, |at| at.elapsed().as_secs())));
                    if ui.add_enabled(!s.cancel.load(Ordering::Relaxed), egui::Button::new("Abaikan hasil")).clicked() {
                        s.cancel.store(true, Ordering::Relaxed);
                        s.status = "Menunggu proses RoboDhan selesai; hasil akan diabaikan…".into();
                    }
                }
            });
            if s.error {
                ui.colored_label(ui.visuals().error_fg_color, &s.status);
            } else {
                ui.label(&s.status);
            }
        },
    );
    if let Some(operation) = action {
        start(app, operation, &ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn deliver(app: &mut PhotocraftApp, operation: &str, value: Result<Value, String>) {
        let (tx, rx) = mpsc::channel();
        app.local_ai.pending = Some(rx);
        app.local_ai.operation = operation.into();
        tx.send(value).unwrap();
        tick(app, &egui::Context::default());
    }
    #[test]
    fn assistant_rewrite_can_be_restored_and_errors_preserve_prompt() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.local_ai.prompt = "foto gunung".into();
        deliver(&mut app, "enhance", Ok(json!({"prompt":"Photograph of a mountain"})));
        assert_eq!(app.local_ai.previous_prompt.as_deref(), Some("foto gunung"));
        deliver(&mut app, "enhance", Err("offline".into()));
        assert_eq!(app.local_ai.prompt, "Photograph of a mountain");
        assert!(app.local_ai.error);
    }
    #[test]
    fn hidden_panel_keeps_prompt_and_processes_pending_response() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.local_ai.prompt = "foto gunung".into();
        app.ui.panels.ai_studio = false;
        deliver(&mut app, "enhance", Ok(json!({"prompt":"Mountain photo"})));
        assert_eq!(app.local_ai.prompt, "Mountain photo");
        assert!(!app.ui.panels.ai_studio);
        let mut saved = serde_json::to_value(&app.ui.panels).unwrap();
        assert!(!serde_json::from_value::<crate::state::Panels>(saved.clone()).unwrap().ai_studio);
        saved.as_object_mut().unwrap().remove("ai_studio");
        assert!(serde_json::from_value::<crate::state::Panels>(saved).unwrap().ai_studio);
    }
    #[test]
    fn cancelled_response_never_opens_image() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.local_ai.cancel.store(true, Ordering::Relaxed);
        deliver(&mut app, "generate", Ok(json!({"pngBase64":"invalid"})));
        assert!(app.session.active().is_none());
        assert_eq!(app.local_ai.status, "Dibatalkan");
        assert!(app.local_ai.pending.is_none());
    }
}
