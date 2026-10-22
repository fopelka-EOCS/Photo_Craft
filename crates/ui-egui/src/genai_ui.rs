//! Generative AI in the shell (#41): Generative AI Settings (provider, model and API key), the
//! Edit › Generative Fill prompt, and installing the configured provider on the session.
//!
//! Off by default: until the user picks a provider and saves a key, no provider exists, the
//! Generative Fill menu item is hidden and nothing is ever contacted. The provider and model live
//! in preferences (shell-owned `dialogs` entry); the key lives only in the OS credential store
//! ([`crate::GenAiServices`]) and in process memory. The key is typed into a buffer outside the
//! dialog's fields, so `ui.inspect` and the control channel never see it.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

/// Opens Generative AI Settings.
pub const SETTINGS: &str = "ui.generativeAiSettings";
const FILL: &str = photocraft_engine::genai_cmds::FILL;
const EXPAND: &str = photocraft_engine::genai_cmds::EXPAND;
const SETTINGS_MARK: &str = "__genaiSettings";
const PROMPT_MARK: &str = "__genaiPrompt";
/// `prefs.dialogs` entry holding `{"provider", "model"}`.
const PREF: &str = "ui.generativeAiSettings";

/// Shell state that must not be serialised or inspected.
#[derive(Default)]
pub struct Runtime {
    /// What is typed in the settings dialog's key field. Cleared when the dialog closes.
    key_input: String,
    last_result: String,
}

pub(crate) fn result_status(app: &mut PhotocraftApp, result: &Value) {
    let provider = result.get("provider").and_then(Value::as_str).unwrap_or("Cloud AI");
    let model = result.get("model").and_then(Value::as_str).unwrap_or("");
    let elapsed = result.get("elapsedMs").and_then(Value::as_u64).unwrap_or(0);
    app.genai.last_result = format!("{provider} / {model} — {:.1}s — new layer added", elapsed as f64 / 1000.0);
    app.ui.status.clone_from(&app.genai.last_result);
    app.ui.status_error = false;
}

/// Providers the settings offer: (id, name). Only providers the app can build appear.
fn providers() -> Vec<(String, &'static str)> {
    vec![
        (String::new(), tl!("None")),
        (photocraft_genai::cloud::OPENAI.into(), tl!("OpenAI-compatible Images")),
        (photocraft_genai::cloud::GEMINI.into(), tl!("Gemini-compatible Images")),
        (photocraft_genai::gemini::ID.into(), "Google Gemini (Interactions)"),
    ]
}

fn credential_id(provider: &str, base: &str) -> String {
    if provider == photocraft_genai::gemini::ID { provider.into() } else { format!("{provider}:{}", base.trim().trim_end_matches('/')) }
}

fn saved(app: &PhotocraftApp) -> (String, String) {
    let v = app.session.prefs().dialogs.get(PREF);
    let get = |k: &str| v.and_then(|v| v.get(k)).and_then(Value::as_str).unwrap_or("").to_owned();
    (get("provider"), get("model"))
}

/// Build the saved provider and install it on the session (at launch and after Settings). Reads
/// the credential store only when a provider is configured; never touches the network.
pub fn restore(app: &mut PhotocraftApp) -> Result<(), String> {
    let (provider, model) = saved(app);
    if provider.is_empty() {
        app.session.set_image_provider(None);
        return Ok(());
    }
    let Some(services) = app.services.genai.as_ref() else {
        return Ok(());
    };
    let config = app.session.prefs().dialogs.get(PREF).cloned().unwrap_or(json!({}));
    if config.get("schemaVersion").and_then(Value::as_u64).is_some_and(|v| v > 1) {
        return Err("Cloud AI settings were written by a newer version; keep a backup and use that version.".into());
    }
    let account = credential_id(&provider, config.get("baseUrl").and_then(Value::as_str).unwrap_or(""));
    let key = (services.load_key)(&account)?.and_then(|k| photocraft_genai::ApiKey::new(&k));
    let Some(key) = key else {
        app.session.set_image_provider(None);
        return Err(crate::i18n::fmt(tl!("No API key is saved for {provider}."), &[("provider", &provider)]));
    };
    let built = (services.provider)(&provider, &model, key, &config)?;
    app.session.set_image_provider(Some(built));
    Ok(())
}

/// Menu, palette and shortcut entry points. Clicking Generative Fill asks for a prompt; with
/// `prompt` already in the params (scripted use) it runs directly.
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    match id {
        SETTINGS => Some(Ok(json!({"dialog": open_settings(app)}))),
        EXPAND if params.get("prompt").is_none() => {
            if app.session.image_provider().is_none() {
                return Some(Ok(json!({"dialog": open_settings(app)})));
            }
            if let Some(why) = app.session.disabled_reason(EXPAND) {
                return Some(Err(why));
            }
            let id = open_prompt(app);
            if let Some(dialog) = app.ui.dialogs.iter_mut().find(|d| d.id == id) {
                dialog.fields.insert("__label".into(), json!(tl!("Generative Expand")));
                dialog.fields.insert("__command".into(), json!(EXPAND));
                for field in ["left", "top", "right", "bottom"] {
                    dialog.fields.insert(field.into(), json!(128));
                }
            }
            Some(Ok(json!({"dialog": id})))
        }
        FILL if params.get("prompt").is_none() => {
            if app.session.image_provider().is_none() {
                return Some(Ok(json!({"dialog": open_settings(app)})));
            }
            if let Some(why) = app.session.disabled_reason(FILL) {
                return Some(Err(why));
            }
            Some(Ok(json!({"dialog": open_prompt(app)})))
        }
        _ => None,
    }
}

/// Is the Edit › Generative Fill item shown? Only once a provider can run it (#41: no
/// "coming soon" items).
pub fn menu_visible(app: &PhotocraftApp) -> bool {
    app.session.image_provider().is_some_and(|p| p.capabilities().edit)
}

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(SETTINGS_MARK) || fields.contains_key(PROMPT_MARK)
}

pub fn ok_label(fields: &Map<String, Value>) -> Option<&'static str> {
    if fields.contains_key(PROMPT_MARK) { Some(tl!("Generate")) } else { None }
}

pub fn open_settings(app: &mut PhotocraftApp) -> u64 {
    if let Some(d) = app.ui.dialogs.iter().find(|d| d.fields.contains_key(SETTINGS_MARK)) {
        return d.id;
    }
    app.genai.key_input.clear();
    let (mut provider, mut model) = saved(app);
    if provider.is_empty() {
        provider = photocraft_genai::cloud::OPENAI.into();
    }
    if model.is_empty() && provider == photocraft_genai::gemini::ID {
        model = photocraft_genai::gemini::DEFAULT_MODEL.into();
    }
    let default_base = if provider == photocraft_genai::cloud::OPENAI {
        "https://api.openai.com/v1"
    } else if provider == photocraft_genai::cloud::GEMINI {
        "https://generativelanguage.googleapis.com/v1beta"
    } else {
        ""
    };
    let base = app.session.prefs().dialogs.get(PREF).and_then(|v| v.get("baseUrl")).and_then(Value::as_str).unwrap_or(default_base);
    let account = credential_id(&provider, base);
    let has_key = app.services.genai.as_ref().and_then(|s| (s.load_key)(&account).ok().flatten()).is_some();
    let mut f = Map::new();
    f.insert(SETTINGS_MARK.into(), json!(true));
    f.insert("__label".into(), json!("Generative AI Settings"));
    f.insert("provider".into(), json!(provider));
    f.insert("model".into(), json!(model));
    f.insert("hasKey".into(), json!(has_key));
    f.insert("baseUrl".into(), json!(base));
    if let Some(config) = app.session.prefs().dialogs.get(PREF) {
        for field in ["baseUrl", "timeout"] {
            if let Some(value) = config.get(field) {
                f.insert(field.into(), value.clone());
            }
        }
    }
    app.ui.open_dialog(DialogKind::Command, f)
}

fn open_prompt(app: &mut PhotocraftApp) -> u64 {
    if let Some(d) = app.ui.dialogs.iter().find(|d| d.fields.contains_key(PROMPT_MARK)) {
        return d.id;
    }
    let last = app.session.prefs().dialogs.get(FILL).and_then(|v| v.get("prompt")).and_then(Value::as_str).unwrap_or("").to_owned();
    let mut f = Map::new();
    f.insert(PROMPT_MARK.into(), json!(true));
    f.insert("__label".into(), json!("Generative Fill"));
    f.insert("prompt".into(), json!(last));
    app.ui.open_dialog(DialogKind::Command, f)
}

pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    if f.contains_key(PROMPT_MARK) {
        prompt_body(app, ui, f);
    } else {
        settings_body(app, ui, f);
    }
}

fn faint(ui: &mut egui::Ui, text: &str) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.add(egui::Label::new(egui::RichText::new(text).color(t.text_faint).size(12.0)).wrap());
}

fn prompt_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    if f.get("__command").and_then(Value::as_str) == Some(EXPAND) {
        ui.label(tl!("Add pixels beyond the current canvas. The original image is preserved; one Undo restores the original canvas."));
        egui::Grid::new("expand-margins").num_columns(2).show(ui, |ui| {
            for (key, label) in [("left", tl!("Left")), ("top", tl!("Top")), ("right", tl!("Right")), ("bottom", tl!("Bottom"))] {
                ui.label(label);
                let mut value = f.get(key).and_then(Value::as_u64).unwrap_or(128).min(4096);
                if ui.add(egui::DragValue::new(&mut value).range(0..=4096).suffix(" px")).changed() {
                    f.insert(key.into(), json!(value));
                }
                ui.end_row();
            }
        });
    }
    let mut prompt = f.get("prompt").and_then(Value::as_str).unwrap_or("").to_owned();
    ui.label(tl!("Describe what to generate in the selection, or leave it empty to continue the image through it."));
    ui.add_space(6.0);
    let r = ui
        .add(egui::TextEdit::singleline(&mut prompt).hint_text(tl!("Prompt (optional)")).desired_width(f32::INFINITY).char_limit(photocraft_genai::MAX_PROMPT));
    crate::field_tab::register(ui.ctx(), r.id);
    if !f.contains_key("__focused") {
        f.insert("__focused".into(), json!(true));
        crate::field_tab::focus(ui.ctx(), r.id);
    }
    if r.gained_focus() {
        crate::field_tab::select_all(ui.ctx(), r.id, &prompt);
    }
    if r.changed() {
        f.insert("prompt".into(), json!(prompt));
    }
    ui.add_space(6.0);
    if let Some(info) = app.session.image_provider().map(|p| p.info()) {
        faint(
            ui,
            &crate::i18n::fmt(
                tl!("The selected area, the image around it and the prompt are sent to {name} ({model})."),
                &[("name", info.name), ("model", &info.model)],
            ),
        );
    }
}

fn settings_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    if app.services.genai.is_none() {
        faint(ui, tl!("Generative AI needs the desktop app."));
        return;
    }
    let mut provider = f.get("provider").and_then(Value::as_str).unwrap_or("").to_owned();
    let options = providers();
    let opts: Vec<(String, &str)> = options.iter().map(|(id, name)| (id.clone(), *name)).collect();
    egui::Grid::new("genai-settings").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
        ui.label(tl!("Provider"));
        if crate::widgets::dropdown(ui, "genai-provider", &mut provider, &opts, 270.0) {
            f.insert("provider".into(), json!(provider));
            f.insert("hasKey".into(), json!(false));
            app.genai.key_input.clear();
            f.insert("model".into(), json!(""));
            let base = if provider == photocraft_genai::cloud::OPENAI {
                "https://api.openai.com/v1"
            } else if provider == photocraft_genai::cloud::GEMINI {
                "https://generativelanguage.googleapis.com/v1beta"
            } else {
                ""
            };
            f.insert("baseUrl".into(), json!(base));
        }
        ui.end_row();
        if !provider.is_empty() {
            if provider != photocraft_genai::gemini::ID {
                ui.label(tl!("API Base URL"));
                let mut base = f.get("baseUrl").and_then(Value::as_str).unwrap_or("").to_owned();
                let response = ui.add(egui::TextEdit::singleline(&mut base).desired_width(320.0).char_limit(1024));
                crate::field_tab::register(ui.ctx(), response.id);
                if response.changed() {
                    f.insert("baseUrl".into(), json!(base));
                }
                ui.end_row();
            }
            ui.label(tl!("Model"));
            let mut model = f.get("model").and_then(Value::as_str).unwrap_or("").to_owned();
            let response = ui.add(egui::TextEdit::singleline(&mut model).hint_text(tl!("Image editing model ID")).desired_width(320.0).char_limit(160));
            crate::field_tab::register(ui.ctx(), response.id);
            if response.changed() {
                f.insert("model".into(), json!(model));
            }
            ui.end_row();
            ui.label(tl!("Timeout (seconds)"));
            let mut timeout = f.get("timeout").and_then(Value::as_u64).unwrap_or(180).clamp(30, 600);
            if ui.add(egui::DragValue::new(&mut timeout).range(30..=600)).changed() {
                f.insert("timeout".into(), json!(timeout));
            }
            ui.end_row();
            ui.label(tl!("API key"));
            let has_key = f.get("hasKey").and_then(Value::as_bool) == Some(true);
            ui.horizontal(|ui| {
                let hint = if has_key { tl!("Saved (paste to replace)") } else { tl!("Paste API key") };
                let response = ui.add(egui::TextEdit::singleline(&mut app.genai.key_input).password(true).hint_text(hint).desired_width(240.0));
                crate::field_tab::register(ui.ctx(), response.id);
                if crate::widgets::secondary_button(ui, tl!("Remove Key"), 0.0).clicked() {
                    let base = f.get("baseUrl").and_then(Value::as_str).unwrap_or("");
                    let account = credential_id(&provider, base);
                    if let Some(services) = app.services.genai.as_ref() {
                        match (services.delete_key)(&account) {
                            Ok(()) => {
                                f.insert("hasKey".into(), json!(false));
                                app.session.set_image_provider(None);
                            }
                            Err(error) => app.ui.status = error,
                        }
                    }
                }
            });
            ui.end_row();
        }
    });
    ui.add_space(8.0);
    crate::widgets::hairline(ui);
    faint(
        ui,
        tl!(
            "Only when you generate: the selected image area, surrounding context, mask and prompt are uploaded to the configured HTTPS endpoint. Usage is billed by your provider. No local model or fallback is used."
        ),
    );
    faint(
        ui,
        tl!(
            "Images protocol: Base URL ends at /v1 (or equivalent); server must support /images/edits with base64 results. Gemini protocol: Base URL ends at /v1beta (or equivalent). A chat-only endpoint cannot edit images."
        ),
    );
    faint(
        ui,
        tl!("Cancel discards the result; it may not stop a request already running at the provider. Input is limited to 2048 pixels on the longest side."),
    );
    let store = app.services.genai.as_ref().map_or("", |s| s.key_store_name);
    faint(ui, &format!("API key storage: {store}. The key is excluded from document and settings files."));
}
/// OK: remember the choice, save a newly typed key, and install the provider (or remove it).
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    if f.contains_key(PROMPT_MARK) {
        let prompt = f.get("prompt").and_then(Value::as_str).unwrap_or("").trim().to_owned();
        app.session.prefs.edit(|p| p.dialogs.insert(FILL.into(), json!({"prompt": prompt})));
        if f.get("__command").and_then(Value::as_str) == Some(EXPAND) {
            let mut params = Map::new();
            params.insert("prompt".into(), json!(prompt));
            for key in ["left", "top", "right", "bottom"] {
                params.insert(key.into(), f.get(key).cloned().unwrap_or(json!(0)));
            }
            let result = app.run(EXPAND, Value::Object(params))?;
            if result.get("elapsedMs").is_some() {
                result_status(app, &result);
            }
            return Ok(result);
        }
        let result = app.run(FILL, json!({"prompt": prompt}))?;
        if result.get("elapsedMs").is_some() {
            result_status(app, &result);
        }
        return Ok(result);
    }
    let typed = std::mem::take(&mut app.genai.key_input);
    let provider = f.get("provider").and_then(Value::as_str).unwrap_or("").to_owned();
    let model = f.get("model").and_then(Value::as_str).unwrap_or("").to_owned();
    let base = f.get("baseUrl").and_then(Value::as_str).unwrap_or("").trim().trim_end_matches('/').to_owned();
    let timeout = f.get("timeout").and_then(Value::as_u64).unwrap_or(180).clamp(30, 600);
    let config = json!({"schemaVersion": 1, "provider": provider, "model": model, "baseUrl": base, "timeout": timeout});
    let mut ready = None;
    if !provider.is_empty() {
        let services = app.services.genai.as_ref().ok_or_else(|| tl!("Generative AI needs the desktop app.").to_string())?;
        let account = credential_id(&provider, &base);
        let saved_key = (services.load_key)(&account)?;
        let key = photocraft_genai::ApiKey::new(&typed).or_else(|| saved_key.as_deref().and_then(photocraft_genai::ApiKey::new));
        if let Some(key) = key {
            // Check the provider builds (model id) before the key is stored.
            ready = Some((services.provider)(&provider, &model, key.clone(), &config)?);
            if !typed.trim().is_empty() {
                (services.save_key)(&account, key.expose())?;
            }
        } else {
            return Err("Enter an API key before enabling cloud AI.".into());
        }
    }
    app.session.prefs.edit(|p| p.dialogs.insert(PREF.into(), config));
    app.session.set_image_provider(ready);
    Ok(json!({"provider": provider, "model": model, "ready": app.session.image_provider().is_some()}))
}

/// Dialog closed without OK: drop whatever was typed in the key field.
pub fn cancelled(app: &mut PhotocraftApp) {
    app.genai.key_input.clear();
}

/// Preferences › Integrations: where Generative AI is set up.
pub fn prefs_rows(app: &PhotocraftApp, ui: &mut egui::Ui) -> bool {
    let t = crate::theme::Tokens::get(ui.ctx());
    if app.services.genai.is_none() {
        return false;
    }
    ui.label(egui::RichText::new(tl!("Generative AI")).font(crate::theme::semibold(12.5)).color(t.text));
    let status = match app.session.image_provider().map(|p| p.info()) {
        Some(info) => crate::i18n::fmt(tl!("Generative Fill uses {name} ({model})."), &[("name", info.name), ("model", &info.model)]),
        None => tl!("Off. Set up a provider with your own API key to use Generative Fill.").to_string(),
    };
    faint(ui, &status);
    if !app.genai.last_result.is_empty() {
        faint(ui, &app.genai.last_result);
    }
    ui.add_space(4.0);
    crate::widgets::secondary_button(ui, tl!("Generative AI Settings…"), 0.0).clicked()
}

#[cfg(test)]
#[path = "genai_ui_tests.rs"]
mod tests;
