//! TX Options: what File › Save As and Save a Copy ask before writing a `.tx` render texture
//! (tiled and MIP-mapped, as OpenImageIO's `maketx` writes it; see `photocraft_io::TxOptions`).
//! The preset "V-Ray / Rendering" is the default; "maketx --oiio (classic)" is exactly the call
//! `maketx --oiio --tile 64 64 --filter lanczos3 --monochrome-detect`.
//!
//! A texture is always written as a copy: the document keeps its own file and unsaved changes,
//! and a `.tx` never becomes the file File › Save writes back to (it opens read-only, too).
//! The save is parked in [`Prompt`] while the modal is up and written once answered; the answer
//! is remembered for the next texture in this session. Saves without the dialog (scripts, a
//! recorded action, the CLI) write the V-Ray preset.
//!
//! Labels are translated (`tl!`); product and filter names (V-Ray, maketx, Lanczos) are not.

use photocraft_io::{TX_TILE_SIZES, TxDataType, TxFilter, TxFixNan, TxOptions, TxPreset, TxWrap};

use crate::{ExportSettings, PhotocraftApp};

/// A save waiting on the TX Options answer.
pub struct Prompt {
    /// The document that requested the save, even if tabs change while this is open.
    doc: photocraft_doc::DocId,
    /// Where the texture goes.
    pub path: String,
    pub options: TxOptions,
    /// The document holds 32-bit floats (what "As document" writes).
    doc_float: bool,
    /// The step the answered save is recorded as (`file.saveAs` or `file.saveACopy`).
    pub record: &'static str,
}

/// `true` when `path` names a `.tx` texture.
pub fn is_tx_path(path: &str) -> bool {
    photocraft_engine::file_cmds::extension(path).as_deref() == Some("tx")
}

/// Parks the save behind the prompt, starting at the last answer of this session (else the
/// V-Ray preset). A prompt already waiting is never replaced.
pub fn park(app: &mut PhotocraftApp, path: String, record: &'static str) -> Result<(), String> {
    if app.save_options_open() {
        return Err(crate::SAVE_OPTIONS_OPEN.into());
    }
    let doc = app.active_doc_id()?;
    let doc_float = app.session.active().is_some_and(|st| st.doc.pixel_format().sample == photocraft_color::SampleType::F32);
    let options = app.tx_last.clone().unwrap_or_default();
    app.tx_options = Some(Prompt { doc, path, options, doc_float, record });
    Ok(())
}

/// Whether `o` writes floats for a document that holds floats when `doc_float` (highlight
/// compression applies only then, see `photocraft_io::TxOptions::hicomp`).
fn writes_float(o: &TxOptions, doc_float: bool) -> bool {
    match o.data_type {
        TxDataType::Half | TxDataType::Float => true,
        TxDataType::AsDocument => doc_float,
        TxDataType::U8 | TxDataType::U16 => false,
    }
}

/// `None` stands for "Custom" in the preset list.
// The option lists, translated at runtime (`tl!` can't run in a `const`). Product and filter names
// (V-Ray, maketx, Lanczos) stay as they are.
fn presets() -> [(Option<TxPreset>, &'static str); 3] {
    [(Some(TxPreset::VRay), "V-Ray / Rendering"), (Some(TxPreset::Classic), "maketx --oiio (classic)"), (None, tl!("Custom"))]
}
fn data_types() -> [(TxDataType, &'static str); 5] {
    [
        (TxDataType::AsDocument, tl!("As document")),
        (TxDataType::U8, tl!("8 bit")),
        (TxDataType::U16, tl!("16 bit")),
        (TxDataType::Half, tl!("16 bit float (half)")),
        (TxDataType::Float, tl!("32 bit float")),
    ]
}
fn filters() -> [(TxFilter, &'static str); 4] {
    [
        (TxFilter::Auto, tl!("Auto (Lanczos 3; triangle for HDR)")),
        (TxFilter::Lanczos3, "Lanczos 3"),
        (TxFilter::Triangle, tl!("Triangle (HDR)")),
        (TxFilter::Box, tl!("Box (maketx: bilinear)")),
    ]
}
fn fix_nan() -> [(TxFixNan, &'static str); 3] {
    [(TxFixNan::Off, tl!("Off")), (TxFixNan::Black, tl!("Black")), (TxFixNan::Box3, tl!("Average 3×3 (box3)"))]
}
fn wraps() -> [(TxWrap, &'static str); 4] {
    [(TxWrap::Black, tl!("Black")), (TxWrap::Clamp, tl!("Clamp")), (TxWrap::Periodic, tl!("Periodic")), (TxWrap::Mirror, tl!("Mirror"))]
}

/// One labelled row: a fixed-width label, then the control.
fn row<R>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(egui::vec2(150.0, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(150.0);
            ui.label(label);
        });
        add(ui)
    })
    .inner
}

/// The dialog's controls for `o`. Pure UI on the options, so it can be tested without a save.
pub(crate) fn controls(ui: &mut egui::Ui, o: &mut TxOptions, doc_float: bool) {
    let mut preset = o.as_preset();
    row(ui, tl!("Preset"), |ui| {
        if crate::widgets::dropdown_literal(ui, "tx-preset", &mut preset, &presets(), 190.0)
            && let Some(p) = preset
        {
            *o = TxOptions::preset(p);
        }
    });
    ui.add_space(8.0);
    crate::widgets::section_label(ui, tl!("Texture"));
    row(ui, tl!("Data type"), |ui| crate::widgets::dropdown_literal(ui, "tx-type", &mut o.data_type, &data_types(), 190.0));
    let tiles: Vec<(u32, String)> = TX_TILE_SIZES.iter().map(|t| (*t, format!("{t} × {t}"))).collect();
    let tile_opts: Vec<(u32, &str)> = tiles.iter().map(|(t, s)| (*t, s.as_str())).collect();
    row(ui, tl!("Tile size"), |ui| crate::widgets::dropdown_literal(ui, "tx-tile", &mut o.tile, &tile_opts, 190.0));
    row(ui, tl!("Compression"), |ui| {
        crate::widgets::dropdown_literal(ui, "tx-comp", &mut o.compress, &[(true, tl!("Deflate + predictor")), (false, tl!("None"))], 190.0)
    });
    ui.add_space(8.0);
    crate::widgets::section_label(ui, tl!("MIP map"));
    crate::widgets::checkbox(ui, &mut o.mipmap, tl!("Build MIP levels down to 1×1"));
    ui.add_enabled_ui(o.mipmap, |ui| {
        row(ui, tl!("Filter"), |ui| crate::widgets::dropdown_literal(ui, "tx-filter", &mut o.filter, &filters(), 190.0));
        let float = writes_float(o, doc_float);
        ui.add_enabled_ui(float, |ui| crate::widgets::checkbox(ui, &mut o.hicomp, tl!("Highlight compression (HDR, --hicomp)")));
    });
    ui.add_space(8.0);
    crate::widgets::section_label(ui, tl!("Wrap mode"));
    row(ui, tl!("Horizontal (s)"), |ui| crate::widgets::dropdown_literal(ui, "tx-swrap", &mut o.wrap.0, &wraps(), 190.0));
    row(ui, tl!("Vertical (t)"), |ui| crate::widgets::dropdown_literal(ui, "tx-twrap", &mut o.wrap.1, &wraps(), 190.0));
    ui.add_space(8.0);
    crate::widgets::section_label(ui, tl!("Detection"));
    crate::widgets::checkbox(ui, &mut o.monochrome_detect, tl!("Write equal RGB channels as gray"));
    crate::widgets::checkbox(ui, &mut o.constant_color_detect, tl!("Write one colour as a single tile"));
    // The repair runs on the document's floats before any conversion, so it follows the source.
    ui.add_enabled_ui(doc_float, |ui| row(ui, tl!("Repair NaN / Inf"), |ui| crate::widgets::dropdown_literal(ui, "tx-nan", &mut o.fix_nan, &fix_nan(), 190.0)));
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(tl!(
            "Values and bit depth are written as they are, without colour conversion. Alpha is stored premultiplied; an alpha that is opaque everywhere is dropped."
        ))
        .small()
        .weak(),
    );
    // The filter Auto takes for this document, so the line is a maketx call that writes the same file.
    ui.label(egui::RichText::new(format!("maketx {}", o.resolved(writes_float(o, doc_float)).maketx_args())).small().monospace().weak());
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = &mut app.tx_options else { return };
    let (mut ok, mut cancel) = (false, false);
    // Enter is the OK shortcut only while nothing else would take it: no open list (picking an
    // entry closes it within this frame) and no focused control (Enter on Cancel is Cancel). Stricter
    // than TIFF/Targa Options, where Cancel winning is enough: here a focused dropdown opens on the
    // same Enter, which would otherwise also save.
    let enter_free = !egui::Popup::is_any_open(ctx) && ctx.memory(|m| m.focused().is_none());
    let mut options = p.options.clone();
    let doc_float = p.doc_float;
    let modal = egui::Modal::new(egui::Id::new("tx-options")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.label(egui::RichText::new(tl!("TX Options")).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        // Scrolls on a short screen, so OK and Cancel below stay reachable.
        let max_h = (ctx.content_rect().height() - 140.0).max(200.0);
        egui::ScrollArea::vertical().max_height(max_h).show(ui, |ui| controls(ui, &mut options, doc_float));
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ok = crate::widgets::primary_button(ui, tl!("OK"), 84.0).clicked();
            cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked();
        });
    });
    cancel |= modal.should_close();
    p.options = options;
    let enter = !cancel && enter_free && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    if (ok || enter) && !cancel {
        let Some(p) = app.tx_options.take() else { return };
        app.tx_last = Some(p.options.clone());
        let settings = ExportSettings { tx: Some(p.options), ..app.save_settings() };
        let step = serde_json::json!({"path": p.path});
        let written = app.with_document(p.doc, |app| app.write_document(p.path, &settings, true));
        match written {
            Ok(_) => app.session.journal.push((p.record.into(), step)),
            Err(e) => {
                app.ui.status = crate::i18n::fmt(tl!("Save failed: {error}"), &[("error", &e)]);
                app.ui.status_error = true;
            }
        }
    } else if cancel {
        app.tx_options = None;
        app.ui.status = tl!("Save cancelled").into();
        app.ui.status_error = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tx_names() {
        assert!(is_tx_path("a.tx"));
        assert!(is_tx_path("C:\\tex\\Wall.TX"));
        assert!(!is_tx_path("a.tx.psd"));
        assert!(!is_tx_path("a.tif"));
        assert!(!is_tx_path("tx"));
    }

    #[test]
    fn float_only_options_follow_the_output_type() {
        let o = TxOptions::default();
        assert!(!writes_float(&o, false));
        assert!(writes_float(&o, true));
        assert!(writes_float(&TxOptions { data_type: TxDataType::Half, ..o.clone() }, false));
        assert!(!writes_float(&TxOptions { data_type: TxDataType::U16, ..o }, true));
    }
}
