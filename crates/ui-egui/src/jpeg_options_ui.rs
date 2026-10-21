//! JPEG Options: the quality (0–12) and format options (Baseline Standard, Baseline Optimized,
//! Progressive) File › Save As and Save a Copy ask for before writing a JPEG, matching Photoshop's
//! JPEG Options dialog.
//!
//! The save is parked in [`Prompt`] while the modal is up (egui is immediate-mode) and written
//! once answered. Cancel drops the save. Scripts skip the prompt by passing `jpegQuality` or
//! `jpegFormat`.

use photocraft_io::{JpegFormat, JpegOptions, percent_to_quality_level, quality_level_to_percent, quality_tier_name};

use crate::{ExportSettings, PhotocraftApp};

/// A save waiting on the JPEG Options answer.
pub struct Prompt {
    /// The document that requested the save, even if tabs change while this is open.
    doc: photocraft_doc::DocId,
    /// Where the image goes.
    pub path: String,
    /// The options chosen (quality level 0..=12, format).
    pub options: JpegOptions,
    /// The Photoshop quality level (0..=12).
    pub level: u8,
    /// Save a Copy: the document keeps its path and unsaved changes.
    pub copy: bool,
    /// The step the answered save is recorded as (`file.saveAs` or `file.saveACopy`).
    pub record: &'static str,
}

/// `true` when `path` names a JPEG image (`.jpg`, `.jpeg`, `.jpe`, `.jfif`).
pub fn is_jpeg_path(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && (ext.eq_ignore_ascii_case("jpg")
                || ext.eq_ignore_ascii_case("jpeg")
                || ext.eq_ignore_ascii_case("jpe")
                || ext.eq_ignore_ascii_case("jfif"))
    })
}

/// Whether saving the active document to `path` asks for JPEG Options.
pub fn wants_prompt(path: &str) -> bool {
    is_jpeg_path(path)
}

/// The JPEG options a command or control call asks for (`"jpegQuality"`, `"jpegFormat"`, `"jpegScans"`),
/// which skip the prompt.
pub fn options_param(params: &serde_json::Value, cmd: &str) -> Result<Option<JpegOptions>, String> {
    photocraft_engine::file_cmds::jpeg_param(params, cmd).map_err(|e| e.to_string())
}

/// The params a save with `options` is recorded with.
pub fn record(options: JpegOptions, step: &mut serde_json::Value) {
    step["jpegQuality"] = options.quality.into();
    step["jpegFormat"] = options.format.name().into();
    if let JpegFormat::Progressive { scans } = options.format {
        step["jpegScans"] = scans.into();
    }
}

/// Parks the save behind the prompt. A prompt already waiting is never replaced.
pub fn park(app: &mut PhotocraftApp, path: String, copy: bool, record: &'static str) -> Result<(), String> {
    if app.save_options_open() {
        return Err("Answer the TIFF, Targa or DDS Options before starting another save".into());
    }
    let doc = app.active_doc_id()?;
    let initial_pct = app.save_settings().jpeg_quality.unwrap_or_else(|| app.session.prefs().export.jpeg_quality.clamp(1, 100) as u8);
    let level = percent_to_quality_level(initial_pct);
    let options = JpegOptions {
        quality: quality_level_to_percent(level),
        format: JpegFormat::BaselineStandard,
    };
    app.jpeg_options = Some(Prompt { doc, path, options, level, copy, record });
    Ok(())
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = &mut app.jpeg_options else { return };
    let (mut ok, mut cancel) = (false, false);
    let mut level = p.level;
    let mut format = p.options.format;
    let modal = egui::Modal::new(egui::Id::new("jpeg-options")).show(ctx, |ui| {
        ui.set_max_width(380.0);
        ui.label(egui::RichText::new(tl!("JPEG Options")).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);

        crate::widgets::section_label(ui, tl!("Image Options"));
        ui.horizontal(|ui| {
            ui.label(format!("{}:", tl!("Quality")));
            let tier = quality_tier_name(level);
            ui.label(egui::RichText::new(tl!(tier)).font(crate::theme::semibold(12.0)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut int_val = level as i64;
                if ui.add(egui::DragValue::new(&mut int_val).range(0..=12)).changed() {
                    level = int_val.clamp(0, 12) as u8;
                }
            });
        });

        let mut slider_val = level;
        if ui.add(egui::Slider::new(&mut slider_val, 0..=12).show_value(false)).changed() {
            level = slider_val;
        }
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("small file")).size(11.0).color(ui.visuals().weak_text_color()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(tl!("large file")).size(11.0).color(ui.visuals().weak_text_color()));
            });
        });

        ui.add_space(10.0);
        crate::widgets::section_label(ui, tl!("Format Options"));
        let is_standard = matches!(format, JpegFormat::BaselineStandard);
        let is_opt = matches!(format, JpegFormat::BaselineOptimized);
        let is_prog = matches!(format, JpegFormat::Progressive { .. });
        let mut scans = match format {
            JpegFormat::Progressive { scans } => scans,
            _ => 3,
        };

        if ui.radio(is_standard, tl!("Baseline (\"Standard\")")).clicked() {
            format = JpegFormat::BaselineStandard;
        }
        if ui.radio(is_opt, tl!("Baseline Optimized")).clicked() {
            format = JpegFormat::BaselineOptimized;
        }
        ui.horizontal(|ui| {
            if ui.radio(is_prog, tl!("Progressive")).clicked() {
                format = JpegFormat::Progressive { scans };
            }
            if is_prog {
                ui.add_space(8.0);
                ui.label(format!("{}:", tl!("Scans")));
                egui::ComboBox::from_id_salt("jpeg-scans")
                    .selected_text(format!("{scans}"))
                    .show_ui(ui, |ui| {
                        for s in [3, 4, 5] {
                            if ui.selectable_value(&mut scans, s, format!("{s}")).clicked() {
                                format = JpegFormat::Progressive { scans };
                            }
                        }
                    });
            }
        });

        ui.add_space(14.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ok = crate::widgets::primary_button(ui, tl!("OK"), 84.0).clicked();
            cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked();
        });
    });
    cancel |= modal.should_close();
    p.level = level;
    p.options.quality = quality_level_to_percent(level);
    p.options.format = format;

    if ok || (!cancel && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))) {
        confirm(app, Some(ctx));
    } else if cancel {
        app.jpeg_options = None;
        app.ui.status = tl!("Save cancelled").into();
        app.ui.status_error = false;
    }
}

/// Confirms the parked JPEG options dialog programmatically or on OK click.
pub fn confirm(app: &mut PhotocraftApp, ctx: Option<&egui::Context>) {
    let Some(p) = app.jpeg_options.take() else { return };
    let settings = ExportSettings {
        jpeg_quality: Some(p.options.quality),
        jpeg_format: Some(p.options.format),
        ..app.save_settings()
    };
    let mut step = if p.record == "file.save" { serde_json::json!({}) } else { serde_json::json!({"path": p.path}) };
    record(p.options, &mut step);
    let written = app.with_document(p.doc, |app| app.write_document(p.path, &settings, p.copy));
    if written.is_ok() {
        app.session.journal.push((p.record.into(), step));
    }
    match written {
        Err(e) => {
            app.ui.status = crate::i18n::fmt(tl!("Save failed: {error}"), &[("error", &e)]);
            app.ui.status_error = true;
        }
        Ok(Some(_)) if !p.copy => {
            if let Some(ctx) = ctx {
                crate::discard_ui::saved_document(app, ctx, p.doc);
            }
        }
        Ok(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_names() {
        assert!(wants_prompt("a.jpg"));
        assert!(wants_prompt("dir/sub/a.JPEG"));
        assert!(wants_prompt("C:\\x\\y.jpe"));
        assert!(wants_prompt("photo.jfif"));
        assert!(!wants_prompt("a.jpg.psd"));
        assert!(!wants_prompt(".jpg"));
        assert!(!wants_prompt("a.png"));
    }

    #[test]
    fn options_params() {
        use serde_json::json;
        assert_eq!(options_param(&json!({}), "file.saveAs"), Ok(None));
        assert_eq!(
            options_param(&json!({"jpegQuality": 10}), "file.saveAs"),
            Ok(Some(JpegOptions { quality: 90, format: JpegFormat::BaselineStandard }))
        );
        assert_eq!(
            options_param(&json!({"jpegFormat": "baselineOptimized"}), "file.saveAs"),
            Ok(Some(JpegOptions { quality: 90, format: JpegFormat::BaselineOptimized }))
        );
        let mut step = json!({"path": "a.jpg"});
        record(JpegOptions { quality: 85, format: JpegFormat::Progressive { scans: 4 } }, &mut step);
        assert_eq!(step, json!({"path": "a.jpg", "jpegQuality": 85, "jpegFormat": "progressive", "jpegScans": 4}));
        assert_eq!(
            options_param(&step, "file.saveAs"),
            Ok(Some(JpegOptions { quality: 85, format: JpegFormat::Progressive { scans: 4 } }))
        );
    }
}
