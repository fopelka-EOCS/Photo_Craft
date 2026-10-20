//! PDF Options: the prompt File › Save As and Save a Copy show before writing a PDF, as
//! Photoshop's Save Adobe PDF dialog does with its Standard menu: a plain PDF, or PDF/X-1a:2001
//! for a printer, with the output condition (an ICC v2 CMYK profile) and the bleed already inside
//! the document. An RGB document, or a CMYK one in another condition, is converted to the output
//! profile on the way out (Photoshop's Convert to Destination); the document itself is untouched.
//!
//! The save is parked in [`Prompt`] while the modal is up (egui is immediate-mode) and written
//! once answered. Cancel drops the save. The last answer is remembered between sessions.

use std::sync::Arc;

use photocraft_cms::{ColorSpace, Profile, ProfileClass};
use photocraft_color::ColorMode;

use crate::{ExportSettings, PhotocraftApp};

/// A CMYK profile the output condition can be: its label in the menu and the profile.
#[derive(Clone)]
pub struct Candidate {
    pub label: String,
    pub profile: Arc<Profile>,
}

impl Candidate {
    /// PDF/X-1a:2001 is PDF 1.3, which knows ICC version 2 only.
    pub fn fits_x1a(&self) -> bool {
        self.profile.version.0 == 2
    }
}

/// A save waiting on the PDF Options answer.
pub struct Prompt {
    /// The document that requested the save, even if tabs change while this is open.
    doc: photocraft_doc::DocId,
    /// Where the document goes.
    pub path: String,
    /// Save a Copy, or a flat PDF of a layered document: it keeps its path and unsaved changes.
    pub copy: bool,
    /// The step the answered save is recorded as (see `targa_options_ui::Prompt::record`).
    pub record: &'static str,
    /// Standard: PDF/X-1a:2001 rather than none.
    pub x1a: bool,
    /// The chosen output condition, an index into `candidates`.
    pub profile: usize,
    /// The bleed inside the document, in millimetres.
    pub bleed_mm: f32,
    pub candidates: Vec<Candidate>,
    /// The document's own profile, as its colour is read, and its mode.
    source: Arc<Profile>,
    mode: ColorMode,
    /// The remembered answer has been applied (it needs the egui context, so on the first frame).
    restored: bool,
}

/// `true` when `path` names a PDF.
pub fn is_pdf_path(path: &str) -> bool {
    photocraft_engine::file_cmds::extension(path).as_deref() == Some("pdf")
}

/// The output conditions on offer: the document's own CMYK profile, the working CMYK space, then
/// every installed CMYK output profile, each once (by description).
fn candidates(app: &PhotocraftApp, doc: &photocraft_doc::Document) -> Vec<Candidate> {
    let mut list: Vec<Candidate> = Vec::new();
    let mut push = |label: String, profile: Arc<Profile>| {
        if profile.color_space == ColorSpace::Cmyk && !list.iter().any(|c| c.profile.description == profile.description) {
            list.push(Candidate { label, profile });
        }
    };
    if doc.mode == ColorMode::Cmyk {
        let p = photocraft_engine::color_cmds::document_profile(doc);
        push(crate::i18n::fmt(tl!("Document: {profile}"), &[("profile", &p.description)]), p);
    }
    let working = app.session.color.working(ColorMode::Cmyk);
    push(crate::i18n::fmt(tl!("Working CMYK: {profile}"), &[("profile", &working.description)]), working);
    for p in photocraft_engine::installed_profiles::installed() {
        if p.color_space != ColorSpace::Cmyk || p.class != ProfileClass::Output {
            continue;
        }
        if let Ok(profile) = photocraft_engine::color_cmds::resolve_profile(&p.path, None, Some(ColorMode::Cmyk)) {
            push(p.description.clone(), profile);
        }
    }
    list
}

/// Parks the save behind the prompt. A prompt already waiting is never replaced.
pub fn park(app: &mut PhotocraftApp, path: String, copy: bool, record: &'static str) -> Result<(), String> {
    if app.save_options_open() {
        return Err("Answer the TIFF, Targa, DDS or PDF Options before starting another save".into());
    }
    let doc = app.active_doc_id()?;
    let st = app.session.active().ok_or("no document")?;
    let (source, mode) = (photocraft_engine::color_cmds::document_profile(&st.doc), st.doc.mode);
    let candidates = candidates(app, &st.doc);
    // Start on the document's own condition when it can be one, else the first that fits.
    let profile = candidates.iter().position(Candidate::fits_x1a).unwrap_or(0);
    app.pdf_options = Some(Prompt { doc, path, copy, record, x1a: false, profile, bleed_mm: 0.0, candidates, source, mode, restored: false });
    Ok(())
}

/// The remembered answer: (PDF/X-1a, output profile description, bleed in mm).
type Remembered = (bool, String, f32);

fn remembered_id() -> egui::Id {
    egui::Id::new("pdf-options-last")
}

/// The [`photocraft_io::PdfOptions`] for `p`'s answer.
pub fn options(p: &Prompt, intent: photocraft_cms::Intent, bpc: bool) -> Result<photocraft_io::PdfOptions, String> {
    if !p.x1a {
        return Ok(photocraft_io::PdfOptions::default());
    }
    let c = p.candidates.get(p.profile).ok_or("PDF/X-1a needs a CMYK output profile; none is installed")?;
    if !c.fits_x1a() {
        return Err(format!("PDF/X-1a:2001 needs an ICC version 2 output profile and \"{}\" is version {}", c.profile.description, c.profile.version.0));
    }
    // A CMYK or gray document already in the condition keeps its numbers.
    let same = p.mode == ColorMode::Grayscale || (p.mode == ColorMode::Cmyk && p.source.description == c.profile.description);
    let source = (!same).then(|| p.source.to_bytes());
    Ok(photocraft_io::PdfOptions {
        x1a: Some(photocraft_io::PdfX1a { profile: c.profile.to_bytes(), source, intent, bpc, bleed_pt: f64::from(p.bleed_mm.max(0.0)) * 72.0 / 25.4 }),
    })
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = &mut app.pdf_options else { return };
    if !p.restored {
        p.restored = true;
        if let Some((x1a, desc, bleed)) = ctx.data_mut(|d| d.get_persisted::<Remembered>(remembered_id())) {
            p.x1a = x1a;
            p.bleed_mm = bleed.clamp(0.0, 50.0);
            if let Some(i) = p.candidates.iter().position(|c| c.profile.description == desc) {
                p.profile = i;
            }
        }
    }
    let (mut ok, mut cancel) = (false, false);
    let (mut x1a, mut profile, mut bleed) = (p.x1a, p.profile, p.bleed_mm);
    let fits = p.candidates.get(profile).is_some_and(Candidate::fits_x1a);
    let can_save = !x1a || fits;
    let modal = egui::Modal::new(egui::Id::new("pdf-options")).show(ctx, |ui| {
        ui.set_max_width(440.0);
        ui.label(egui::RichText::new(tl!("PDF Options")).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        crate::widgets::section_label(ui, tl!("Standard"));
        ui.radio_value(&mut x1a, false, tl!("None"));
        ui.radio_value(&mut x1a, true, "PDF/X-1a:2001");
        if x1a {
            ui.add_space(8.0);
            crate::widgets::section_label(ui, tl!("Output Condition"));
            if p.candidates.is_empty() {
                ui.label(tl!(
                    "No CMYK profile is installed. Install the printer's profile (for example ISO Coated v2 or Coated FOGRA39) in Windows and save again."
                ));
            } else {
                let labels: Vec<(usize, String)> = p
                    .candidates
                    .iter()
                    .enumerate()
                    .map(|(i, c)| (i, if c.fits_x1a() { c.label.clone() } else { format!("{} (ICC v{})", c.label, c.profile.version.0) }))
                    .collect();
                let options: Vec<(usize, &str)> = labels.iter().map(|(i, l)| (*i, l.as_str())).collect();
                crate::widgets::dropdown_literal(ui, "pdf-output-profile", &mut profile, &options, 420.0);
                if !fits {
                    ui.add_space(4.0);
                    ui.label(tl!("PDF/X-1a:2001 needs an ICC version 2 profile. Choose the printer's profile (for example ISO Coated v2 or Coated FOGRA39)."));
                } else {
                    ui.add_space(4.0);
                    let same = p.mode == ColorMode::Grayscale
                        || (p.mode == ColorMode::Cmyk && p.candidates.get(profile).is_some_and(|c| c.profile.description == p.source.description));
                    ui.weak(if same {
                        tl!("The CMYK numbers are kept.")
                    } else {
                        tl!("The colours are converted to this profile in the PDF; the document is not changed.")
                    });
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(tl!("Bleed:"));
                crate::widgets::value_field(ui, &mut bleed, 0.0..=50.0, "mm", 70.0);
            });
            ui.weak(tl!("The bleed already in the document: the trim box sits this far inside the page."));
        }
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ok = ui.add_enabled_ui(can_save, |ui| crate::widgets::primary_button(ui, tl!("Save PDF"), 84.0)).inner.clicked();
            cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked();
        });
    });
    cancel |= modal.should_close();
    p.x1a = x1a;
    p.profile = profile;
    p.bleed_mm = bleed;
    // A focused button takes Enter as its click without consuming the key: Enter on Cancel cancels.
    let enter = can_save && !cancel && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    if ok || enter {
        let Some(p) = app.pdf_options.take() else { return };
        let (intent, bpc) = (app.session.color.settings.intent(), app.session.color.settings.bpc);
        let pdf = match options(&p, intent, bpc) {
            Ok(pdf) => pdf,
            Err(e) => {
                app.ui.status = crate::i18n::fmt(tl!("Save failed: {error}"), &[("error", &e)]);
                app.ui.status_error = true;
                return;
            }
        };
        let desc = p.candidates.get(p.profile).map(|c| c.profile.description.clone()).unwrap_or_default();
        ctx.data_mut(|d| d.insert_persisted::<Remembered>(remembered_id(), (p.x1a, desc, p.bleed_mm)));
        let settings = ExportSettings { pdf, ..app.save_settings() };
        let step = if p.record == "file.save" { serde_json::json!({}) } else { serde_json::json!({"path": p.path}) };
        let written = app.with_document(p.doc, |app| app.write_document(p.path, &settings, p.copy));
        if written.is_ok() {
            // An action being recorded keeps the Save As or Save a Copy.
            app.session.journal.push((p.record.into(), step));
        }
        match written {
            Err(e) => {
                app.ui.status = crate::i18n::fmt(tl!("Save failed: {error}"), &[("error", &e)]);
                app.ui.status_error = true;
            }
            Ok(Some(_)) if !p.copy => crate::discard_ui::saved_document(app, ctx, p.doc),
            // A background save releases the close prompt when it finishes (`jobs_ui`).
            Ok(_) => {}
        }
    } else if cancel {
        app.pdf_options = None;
        app.ui.status = tl!("Save cancelled").into();
        app.ui.status_error = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_names() {
        assert!(is_pdf_path("a.pdf"));
        assert!(is_pdf_path("C:\\x\\y.PDF"));
        assert!(!is_pdf_path("a.pdf.psd"));
        assert!(!is_pdf_path("a.png"));
    }
}
