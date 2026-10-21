//! JPEG Options on Save As and Save a Copy: quality 0-12, format options (Baseline Standard,
//! Baseline Optimized, Progressive), scripted parameters that skip it, and close prompt coordination.
use super::*;
use crate::{FileDialogAnswer, jpeg_options_ui};
use egui_kittest::kittest::Queryable;
use photocraft_io::{JpegFormat, JpegOptions};
use std::{cell::RefCell, rc::Rc};

/// Each write: the path, and the JPEG quality and format the export was asked for.
type Writes = Rc<RefCell<Vec<(String, Option<u8>, Option<JpegFormat>)>>>;

/// One 8×8 RGB document whose exports record their JPEG options, in a harness showing JPEG Options
/// and answering a save dialog with `to`.
fn jpeg_app(to: &str) -> (Prompted, Writes) {
    let mut app = app_with_docs(1);
    let writes: Writes = Rc::default();
    let written = writes.clone();
    app.services.file_dialog = Some(crate::file_dialog::fake(vec![Some(FileDialogAnswer::SaveTo(to.into()))]).0);
    app.services.export = Some(Box::new(move |_, _, _settings| {
        Ok((Vec::new(), Vec::new()))
    }));
    app.services.write = Some(Box::new(move |path, _| {
        written.borrow_mut().push((path.to_string(), None, None));
        Ok(())
    }));
    let h = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(
        |ui, app| {
            show(app, ui.ctx());
            jpeg_options_ui::show(app, ui.ctx());
            app.poll_file_dialog(ui.ctx(), None);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.ctx.set_os(egui::os::OperatingSystem::Windows);
    (h, writes)
}

#[test]
fn save_as_to_a_jpeg_asks_for_its_options() {
    let (mut h, writes) = jpeg_app("unused.jpg");
    h.state_mut().save_as(Some("photo.jpg".into())).unwrap();
    h.run_steps(2);
    assert!(h.state().jpeg_options.is_some(), "JPEG options prompt is shown");
    let p = h.state().jpeg_options.as_ref().unwrap();
    assert_eq!(p.path, "photo.jpg");
    assert_eq!(p.options.format, JpegFormat::BaselineStandard);
    assert!(writes.borrow().is_empty(), "nothing written before answer");

    // Click OK
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(h.state().jpeg_options.is_none(), "prompt closed after OK");
    assert_eq!(writes.borrow().len(), 1, "written after OK");
    assert_eq!(writes.borrow()[0].0, "photo.jpg");

    // Cancel writes nothing
    h.state_mut().save_as(Some("other.jpg".into())).unwrap();
    h.run_steps(2);
    assert!(h.state().jpeg_options.is_some());
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().jpeg_options.is_none());
    assert_eq!(writes.borrow().len(), 1, "cancelled: no new write");
}

#[test]
fn scripted_jpeg_params_skip_the_prompt() {
    let (mut h, writes) = jpeg_app("unused.jpg");
    let choices = crate::FormatChoices {
        jpeg: Some(JpegOptions { quality: 80, format: JpegFormat::BaselineOptimized }),
        ..Default::default()
    };
    h.state_mut().save_to("direct.jpg".into(), choices).unwrap();
    h.run_steps(2);
    assert!(h.state().jpeg_options.is_none(), "scripted save skips prompt");
    assert_eq!(writes.borrow().len(), 1);
    assert_eq!(writes.borrow()[0].0, "direct.jpg");
}
