//! Edit › Preferences › AI: the local AI models PhotoCraft's generative tools use.
//!
//! A search bar and Import over one collapsible section per model kind (collapsed by default,
//! opened by a search that matches inside). Each section lists the installed models (a tick box
//! to enable or disable, size on disk, delete) and the recommended models not downloaded yet
//! (a darker, striped row with Download). A downloaded recommended model keeps its stripes on the
//! normal background. Downloads and imports run in the background with progress, time taken and
//! an ETA, and continue when the dialog closes.
//!
//! Model changes apply at once, like Photoshop's plug-in and preset managers: OK and Cancel are
//! for the dialog's other settings.

use egui::{Align, Color32, CornerRadius, Layout, Rect, RichText, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use photocraft_ml::manager::CatalogRow;
use photocraft_ml::{InstalledModel, Job, JobKind, JobState, Manager, ModelKind, ModelSource, format_duration, format_size};
use serde_json::{Map, Value, json};

use crate::i18n::fmt;
use crate::theme::Tokens;

/// Dialog field holding this tab's UI state (search text lives in the dialog's own search box).
const STATE: &str = "__ai";
/// Set by the tab when Import › Choose Files… is pressed; the dialog owner shows the file dialog.
pub(crate) const PICK_FILES: &str = "__aiPickFiles";

/// The model file types the import dialog offers.
pub(crate) const MODEL_EXTENSIONS: [&str; 9] = ["onnx", "safetensors", "ckpt", "pt", "pth", "bin", "gguf", "data", "onnx_data"];

const ROW_H: f32 = 46.0;

/// Takes the tab's state out of the dialog fields (a fresh one if missing or not an object);
/// [`body`] puts it back when it's done.
fn take_state(f: &mut Map<String, Value>) -> Map<String, Value> {
    match f.remove(STATE) {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

fn get_str(m: &Map<String, Value>, k: &str) -> String {
    m.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn kind_of(m: &Map<String, Value>) -> ModelKind {
    m.get("importKind").and_then(Value::as_str).and_then(ModelKind::from_id).unwrap_or(ModelKind::Removal)
}

/// A slightly darker shade of `c`, for recommended rows (darker in both themes).
fn darker(c: Color32) -> Color32 {
    let k = 0.86;
    Color32::from_rgba_unmultiplied((c.r() as f32 * k) as u8, (c.g() as f32 * k) as u8, (c.b() as f32 * k) as u8, c.a())
}

/// Diagonal stripes across `rect`: what marks a recommended model's row.
fn stripes(ui: &Ui, rect: Rect, color: Color32) {
    let p = ui.painter().with_clip_rect(rect.shrink(1.0));
    let gap = 9.0;
    let mut x = rect.left() - rect.height();
    while x < rect.right() {
        p.line_segment([pos2(x, rect.bottom()), pos2(x + rect.height(), rect.top())], Stroke::new(3.0, color));
        x += gap;
    }
}

fn progress_bar(ui: &mut Ui, fraction: Option<f32>, width: f32) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, 4.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(2), t.field_border);
    // Unknown size: a moving segment.
    let f = fraction.unwrap_or_else(|| (ui.input(|i| i.time) * 0.8).fract() as f32);
    let fill = if fraction.is_some() {
        Rect::from_min_size(rect.min, vec2(rect.width() * f, rect.height()))
    } else {
        Rect::from_min_size(pos2(rect.left() + rect.width() * f * 0.75, rect.top()), vec2(rect.width() * 0.25, rect.height()))
    };
    ui.painter().rect_filled(fill, CornerRadius::same(2), t.accent);
}

/// "120 MB of 208 MB · 42 s · about 30 s left".
pub(crate) fn job_status(j: &Job) -> String {
    match &j.state {
        JobState::Resolving => tl!("Looking up the model…").to_string(),
        JobState::Running => {
            let mut s = match j.total_bytes {
                Some(t) => fmt(tl!("{done} of {total}"), &[("done", &format_size(j.done_bytes)), ("total", &format_size(t))]),
                None => format_size(j.done_bytes),
            };
            s.push_str(&format!(" · {}", format_duration(j.elapsed())));
            if let Some(eta) = j.eta() {
                s.push_str(" · ");
                s.push_str(&fmt(tl!("about {time} left"), &[("time", &format_duration(eta))]));
            }
            s
        }
        JobState::Done => fmt(tl!("Finished in {time}"), &[("time", &format_duration(j.elapsed()))]),
        JobState::Failed(e) => e.clone(),
        JobState::Cancelled => tl!("Cancelled").to_string(),
    }
}

enum RowLook {
    /// Installed, not from the catalog.
    Plain,
    /// Downloaded from the catalog: normal background, stripes.
    RecommendedInstalled,
    /// Recommended, not downloaded: darker, striped.
    Recommended,
}

/// Paints a row's background and gives a child `Ui` inside it.
fn row<R>(ui: &mut Ui, look: RowLook, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::hover());
    let bg = match look {
        RowLook::Recommended => darker(t.field),
        _ => t.field,
    };
    ui.painter().rect_filled(rect, t.radius_sm, bg);
    if !matches!(look, RowLook::Plain) {
        stripes(ui, rect, t.text.gamma_multiply(0.045));
    }
    ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(10.0, 5.0))).layout(Layout::right_to_left(Align::Center)));
    add(&mut child)
}

/// Name, then a dim details line, elided to `width`.
fn texts(ui: &mut Ui, name: &str, tag: Option<&str>, details: &str, width: f32) {
    let t = Tokens::get(ui.ctx());
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add(egui::Label::new(RichText::new(name).font(crate::theme::medium(12.5)).color(t.text)).truncate().selectable(false)).on_hover_text(name);
            if let Some(tag) = tag {
                ui.label(RichText::new(tag).size(10.5).color(t.accent_text));
            }
        });
        ui.set_max_width(width);
        ui.add(egui::Label::new(RichText::new(details).size(11.0).color(t.text_faint)).truncate().selectable(false)).on_hover_text(details);
    });
}

fn delete_controls(ui: &mut Ui, id: &str, st: &mut Map<String, Value>, m: &Manager, error: &mut Option<String>) {
    let confirming = st.get("confirmDelete").and_then(Value::as_str) == Some(id);
    if confirming {
        if crate::widgets::compact_button(ui, tl!("Keep"), 22.0).clicked() {
            st.remove("confirmDelete");
        }
        if crate::widgets::compact_button(ui, tl!("Delete"), 22.0).clicked() {
            if let Err(e) = m.delete(id) {
                *error = Some(e.to_string());
            }
            st.remove("confirmDelete");
        }
    } else if crate::icons::button(ui, "trash", 24.0, false, tl!("Delete this model")).clicked() {
        st.insert("confirmDelete".into(), json!(id));
    }
}

fn installed_row(ui: &mut Ui, model: &InstalledModel, recommended: bool, st: &mut Map<String, Value>, m: &Manager, error: &mut Option<String>) {
    let t = Tokens::get(ui.ctx());
    let look = if recommended { RowLook::RecommendedInstalled } else { RowLook::Plain };
    row(ui, look, |ui| {
        delete_controls(ui, &model.id, st, m, error);
        ui.label(RichText::new(format_size(model.size_bytes)).size(11.5).color(t.text_dim).monospace());
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            let (rect, resp) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
            let resp = resp.on_hover_text(if model.enabled { tl!("Enabled: tools can use this model") } else { tl!("Disabled: no tool offers this model") });
            crate::widgets::check_box(ui.painter(), rect, model.enabled, resp.hovered(), resp.is_pointer_button_down_on(), &t);
            if resp.clicked()
                && let Err(e) = m.set_enabled(&model.id, !model.enabled)
            {
                *error = Some(e.to_string());
            }
            let mut details = model.source.describe();
            if !model.license.is_empty() {
                details = format!("{details} · {}", model.license);
            }
            let w = ui.available_width();
            texts(ui, &model.name, recommended.then_some(tl!("Recommended")), &details, w);
        });
    });
}

fn recommended_row(ui: &mut Ui, r: &CatalogRow, m: &Manager, error: &mut Option<String>) {
    let t = Tokens::get(ui.ctx());
    row(ui, RowLook::Recommended, |ui| {
        let job = r.job.as_ref().filter(|j| j.active() || matches!(j.state, JobState::Failed(_)));
        match job {
            Some(j) if j.active() => {
                if crate::widgets::compact_button(ui, tl!("Cancel"), 22.0).clicked() {
                    m.cancel(j.id);
                }
            }
            _ => {
                if crate::widgets::compact_button(ui, tl!("Download"), 22.0).on_hover_text(r.model.homepage).clicked()
                    && let Err(e) = m.download(r.model.id)
                {
                    *error = Some(e.to_string());
                }
            }
        }
        ui.label(RichText::new(format_size(r.model.size())).size(11.5).color(t.text_dim).monospace());
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            // Lines up with the installed rows' tick boxes.
            ui.add_space(20.0);
            let w = ui.available_width();
            match job {
                Some(j) => {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 3.0;
                        ui.add(egui::Label::new(RichText::new(r.model.name).font(crate::theme::medium(12.5)).color(t.text)).truncate().selectable(false));
                        if j.active() {
                            progress_bar(ui, j.fraction(), (w - 8.0).max(40.0));
                        }
                        let color = if matches!(j.state, JobState::Failed(_)) { t.danger } else { t.text_faint };
                        let s = job_status(j);
                        ui.add(egui::Label::new(RichText::new(&s).size(11.0).color(color)).truncate().selectable(false)).on_hover_text(&s);
                    });
                }
                None => {
                    let details = format!("{} · {}", r.model.description, r.model.license);
                    texts(ui, r.model.name, Some(tl!("Recommended")), &details, w);
                }
            }
        });
    });
}

/// The section header: chevron, title and counts. Returns whether the section is open.
fn header(ui: &mut Ui, kind: ModelKind, installed: usize, recommended: usize, forced_open: bool, st: &mut Map<String, Value>) -> bool {
    let t = Tokens::get(ui.ctx());
    let key = format!("open.{}", kind.id());
    let open = forced_open || st.get(&key).and_then(Value::as_bool).unwrap_or(false);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, t.radius_sm, t.hover);
    }
    let c = rect.left_center() + vec2(9.0, 0.0);
    let pts = if open {
        vec![c + vec2(-4.0, -2.0), c + vec2(4.0, -2.0), c + vec2(0.0, 3.0)]
    } else {
        vec![c + vec2(-2.0, -4.0), c + vec2(3.0, 0.0), c + vec2(-2.0, 4.0)]
    };
    ui.painter().add(egui::Shape::convex_polygon(pts, t.icon, Stroke::NONE));
    ui.painter().text(rect.left_center() + vec2(20.0, 0.0), egui::Align2::LEFT_CENTER, tl!(kind.title()), crate::theme::semibold(12.5), t.text);
    let mut counts = Vec::new();
    if installed > 0 {
        counts.push(fmt(tl!("{n} installed"), &[("n", &installed.to_string())]));
    }
    if recommended > 0 {
        counts.push(fmt(tl!("{n} to download"), &[("n", &recommended.to_string())]));
    }
    ui.painter().text(rect.right_center() - vec2(8.0, 0.0), egui::Align2::RIGHT_CENTER, counts.join(" · "), crate::theme::medium(11.0), t.text_faint);
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() && !forced_open {
        st.insert(key, json!(!open));
    }
    open
}

fn import_panel(ui: &mut Ui, st: &mut Map<String, Value>, m: &Manager, error: &mut Option<String>) {
    let t = Tokens::get(ui.ctx());
    let frame = egui::Frame::new().fill(t.card).stroke(Stroke::new(1.0, t.card_border)).corner_radius(t.radius_sm).inner_margin(10.0);
    frame.show(ui, |ui| {
        ui.set_width((ui.available_width() - 2.0).max(0.0));
        let from_link = st.get("importFrom").and_then(Value::as_str) == Some("link");
        ui.horizontal(|ui| {
            if crate::widgets::pill_tab(ui, tl!("From a File"), !from_link).clicked() {
                st.insert("importFrom".into(), json!("file"));
            }
            if crate::widgets::pill_tab(ui, tl!("From a Link"), from_link).clicked() {
                st.insert("importFrom".into(), json!("link"));
            }
        });
        ui.add_space(6.0);
        let mut kind = kind_of(st);
        let options: Vec<(ModelKind, &str)> = ModelKind::ALL.iter().map(|k| (*k, k.title())).collect();
        ui.horizontal(|ui| {
            ui.label(RichText::new(tl!("Use as")).color(t.text_dim));
            if crate::widgets::dropdown(ui, "ai-import-kind", &mut kind, &options, 200.0) {
                st.insert("importKind".into(), json!(kind.id()));
            }
        });
        let mut name = get_str(st, "importName");
        ui.horizontal(|ui| {
            ui.label(RichText::new(tl!("Name")).color(t.text_dim));
            if crate::widgets::text_field(ui, ui.id().with("ai-import-name"), &mut name, tl!("Optional"), 200.0, false).changed() {
                st.insert("importName".into(), json!(name));
            }
        });
        ui.add_space(4.0);
        let name = (!name.trim().is_empty()).then(|| name.trim().to_string());
        if from_link {
            let mut link = get_str(st, "importLink");
            let w = (ui.available_width() - 90.0).max(120.0);
            ui.horizontal(|ui| {
                if crate::widgets::text_field(ui, ui.id().with("ai-import-link"), &mut link, tl!("Hugging Face, Civitai or download link"), w, false).changed()
                {
                    st.insert("importLink".into(), json!(link));
                }
                if crate::widgets::primary_button(ui, tl!("Import"), 70.0).clicked() {
                    match m.import_link(&link, kind, name.clone()) {
                        Ok(_) => {
                            st.insert("importLink".into(), json!(""));
                            st.insert("importName".into(), json!(""));
                        }
                        Err(e) => *error = Some(e.to_string()),
                    }
                }
            });
            ui.label(
                RichText::new(tl!("Paste a Hugging Face file or repository (org/name), a Civitai model page, or a direct link to a model file."))
                    .size(11.0)
                    .color(t.text_faint),
            );
        } else {
            ui.horizontal(|ui| {
                if crate::widgets::primary_button(ui, tl!("Choose Files…"), 110.0).clicked() {
                    st.insert(PICK_FILES.into(), json!({"kind": kind.id(), "name": name}));
                }
                ui.label(
                    RichText::new(tl!("ONNX, safetensors, checkpoint or GGUF. Pick an ONNX model and its .data file together.")).size(11.0).color(t.text_faint),
                );
            });
        }
    });
}

/// Imports in progress or just finished (downloads of recommended models show in their rows).
fn import_jobs(ui: &mut Ui, jobs: &[Job], m: &Manager) {
    let t = Tokens::get(ui.ctx());
    for j in jobs.iter().filter(|j| j.kind == JobKind::Import) {
        let failed = matches!(j.state, JobState::Failed(_));
        row(ui, RowLook::Plain, |ui| {
            if j.active() {
                if crate::widgets::compact_button(ui, tl!("Cancel"), 22.0).clicked() {
                    m.cancel(j.id);
                }
            } else if crate::widgets::compact_button(ui, tl!("Dismiss"), 22.0).clicked() {
                m.dismiss(j.id);
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add_space(20.0);
                let w = ui.available_width();
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    let title = fmt(tl!("Importing {name}"), &[("name", &j.label)]);
                    ui.add(egui::Label::new(RichText::new(&title).font(crate::theme::medium(12.5)).color(t.text)).truncate().selectable(false));
                    if j.active() {
                        progress_bar(ui, j.fraction(), (w - 8.0).max(40.0));
                    }
                    let s = job_status(j);
                    ui.add(egui::Label::new(RichText::new(&s).size(11.0).color(if failed { t.danger } else { t.text_faint })).truncate().selectable(false))
                        .on_hover_text(&s);
                });
            });
        });
        ui.add_space(4.0);
    }
}

fn matches(query: &str, name: &str, extra: &str) -> bool {
    query.is_empty() || name.to_lowercase().contains(query) || extra.to_lowercase().contains(query)
}

/// The AI section's content. `search` is the dialog's search text.
pub(crate) fn body(ui: &mut Ui, f: &mut Map<String, Value>, models: Option<&Manager>) {
    let t = Tokens::get(ui.ctx());
    let Some(m) = models else {
        ui.label(RichText::new(tl!("AI models run in the desktop app. Download PhotoCraft for your computer to use them.")).color(t.text_faint));
        return;
    };
    let query = f.get("__search").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
    let mut state = take_state(f);
    let st = &mut state;
    let mut error: Option<String> = st.get("error").and_then(Value::as_str).map(str::to_string);
    if let Some(w) = m.warning() {
        ui.label(RichText::new(w).size(11.5).color(t.warning));
    }
    // Model search and Import.
    let mut find = get_str(st, "find");
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 96.0).max(120.0);
        if crate::widgets::text_field(ui, ui.id().with("ai-model-search"), &mut find, tl!("Search models by name…"), w, false).changed() {
            st.insert("find".into(), json!(find));
        }
        let open = st.get("importOpen").and_then(Value::as_bool).unwrap_or(false);
        if crate::widgets::secondary_button(ui, tl!("Import…"), 88.0).on_hover_text(tl!("Add a model from a file or a link")).clicked() {
            st.insert("importOpen".into(), json!(!open));
        }
    });
    let find = find.trim().to_lowercase();
    let query = if find.is_empty() { query } else { find };
    ui.add_space(6.0);
    if st.get("importOpen").and_then(Value::as_bool).unwrap_or(false) {
        import_panel(ui, st, m, &mut error);
        ui.add_space(6.0);
    }
    let jobs = m.jobs();
    import_jobs(ui, &jobs, m);
    if let Some(e) = error.clone() {
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(RichText::new(&e).size(11.5).color(t.danger)).wrap());
            if crate::widgets::compact_button(ui, tl!("OK"), 20.0).clicked() {
                error = None;
            }
        });
        ui.add_space(4.0);
    }
    let installed = m.installed();
    let catalog = m.catalog();
    let mut shown_any = false;
    for kind in ModelKind::ALL {
        let rec_installed: Vec<&InstalledModel> = installed
            .iter()
            .filter(|i| i.kind == kind && matches!(&i.source, ModelSource::Catalog { .. }))
            .filter(|i| matches(&query, &i.name, i.license.as_str()))
            .collect();
        let others: Vec<&InstalledModel> =
            installed.iter().filter(|i| i.kind == kind && !matches!(&i.source, ModelSource::Catalog { .. })).filter(|i| matches(&query, &i.name, "")).collect();
        let to_download: Vec<&CatalogRow> =
            catalog.iter().filter(|r| r.model.kind == kind && r.installed.is_none()).filter(|r| matches(&query, r.model.name, r.model.description)).collect();
        let total_here = rec_installed.len() + others.len() + to_download.len();
        // Every kind a tool uses keeps its section, even empty, so it's clear where an import
        // goes; "Other" appears only when something was imported there. A search hides
        // sections with no match.
        if total_here == 0 && (kind == ModelKind::Other || !query.is_empty()) {
            continue;
        }
        shown_any = true;
        let open = header(ui, kind, rec_installed.len() + others.len(), to_download.len(), !query.is_empty(), st);
        if open {
            ui.indent(("ai-section", kind.id()), |ui| {
                ui.label(RichText::new(tl!(kind.used_by())).size(11.0).color(t.text_faint));
                ui.add_space(2.0);
                for model in others {
                    installed_row(ui, model, false, st, m, &mut error);
                    ui.add_space(4.0);
                }
                for model in rec_installed {
                    installed_row(ui, model, true, st, m, &mut error);
                    ui.add_space(4.0);
                }
                for r in to_download {
                    recommended_row(ui, r, m, &mut error);
                    ui.add_space(4.0);
                }
                if total_here == 0 {
                    ui.label(RichText::new(tl!("No models yet. Use Import… to add one.")).size(11.5).color(t.text_dim));
                }
            });
        }
        ui.add_space(2.0);
    }
    if !shown_any {
        ui.label(
            RichText::new(if query.is_empty() { tl!("No models yet. Use Import… to add one.") } else { tl!("No models match your search.") })
                .color(t.text_faint),
        );
    }
    ui.add_space(8.0);
    ui.label(RichText::new(fmt(tl!("Models are stored in {path}"), &[("path", &m.dir().display().to_string())])).size(11.0).color(t.text_faint));
    match error {
        Some(e) => st.insert("error".into(), json!(e)),
        None => st.remove("error"),
    };
    if m.any_active() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
    }
    // Choose Files… is answered by the dialog's owner, which has the file dialog.
    if let Some(pick) = state.remove(PICK_FILES) {
        f.insert(PICK_FILES.into(), pick);
    }
    f.insert(STATE.into(), Value::Object(state));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn darker_is_darker_and_keeps_alpha() {
        let c = darker(Color32::from_rgba_unmultiplied(200, 100, 50, 255));
        assert!(c.r() < 200 && c.g() < 100 && c.b() < 50);
        assert_eq!(c.a(), 255);
        assert_eq!(darker(Color32::BLACK), Color32::BLACK);
    }

    #[test]
    fn the_tab_state_recovers_from_a_wrong_type() {
        let mut f = Map::new();
        f.insert(STATE.into(), json!(42));
        assert!(take_state(&mut f).is_empty());
        f.insert(STATE.into(), json!({"find": "sam"}));
        assert_eq!(take_state(&mut f).get("find"), Some(&json!("sam")));
    }

    #[test]
    fn search_matches_name_or_details_case_insensitively() {
        assert!(matches("", "x", ""));
        assert!(matches("lama", "Big LaMa", ""));
        assert!(matches("apache", "x", "Apache-2.0"));
        assert!(!matches("sdxl", "LaMa", "removal"));
    }
}
