//! TX Options on Save As and Save a Copy: the render texture's settings it asks for, the answer it
//! remembers, and the copy it always writes.
use super::*;
use crate::{FileDialogAnswer, tx_options_ui};
use photocraft_io::{TxOptions, TxPreset};
use std::{cell::RefCell, rc::Rc};

/// Each write: the path, and the TX settings the export was asked for (their maketx options).
type Writes = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

fn asked(tx: Option<&TxOptions>) -> Vec<u8> {
    serde_json::to_vec(&tx.map(TxOptions::maketx_args)).unwrap()
}

/// One document whose exports record their TX settings, in a harness showing TX Options and
/// answering a save dialog with `to`.
fn tx_app(to: &str) -> (Prompted, Writes) {
    let mut app = app_with_docs(1);
    let writes: Writes = Rc::default();
    let written = writes.clone();
    app.services.file_dialog = Some(crate::file_dialog::fake(vec![Some(FileDialogAnswer::SaveTo(to.into()))]).0);
    app.services.export = Some(Box::new(|_, _, settings| Ok((asked(settings.tx.as_ref()), Vec::new()))));
    app.services.write = Some(Box::new(move |path, bytes| {
        written.borrow_mut().push((path.to_string(), bytes.to_vec()));
        Ok(())
    }));
    let h = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 700.0)).build_ui_state(
        |ui, app| {
            show(app, ui.ctx());
            tx_options_ui::show(app, ui.ctx());
            app.poll_file_dialog(ui.ctx(), None);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.ctx.set_os(egui::os::OperatingSystem::Windows);
    (h, writes)
}

/// Save As to a .tx asks first, starting at the V-Ray preset, writes once answered, and writes a
/// copy: the document keeps its path and its unsaved changes.
#[test]
fn save_as_to_a_tx_asks_and_writes_a_copy() {
    let (mut h, writes) = tx_app("unused.tx");
    make_dirty(h.state_mut(), 0);
    h.state_mut().save_as(Some("wall.tx".into())).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().tx_options.as_ref().map(|p| p.options.clone()), Some(TxOptions::preset(TxPreset::VRay)));
    assert!(writes.borrow().is_empty(), "nothing is written before the answer");
    assert!(h.state_mut().save_as(Some("other.tx".into())).is_err(), "one save at a time");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("wall.tx".to_string(), asked(Some(&TxOptions::default())))]);
    assert!(h.state().tx_options.is_none());
    let st = h.state().session.active().unwrap();
    assert_eq!(st.path, None, "a texture never becomes the document's file");
    assert!(st.is_dirty());
}

/// The settings answered are the ones written, and the next prompt starts there; Cancel writes
/// nothing.
#[test]
fn tx_options_writes_the_choice_remembers_it_or_cancels() {
    let (mut h, writes) = tx_app("unused.tx");
    h.state_mut().save_as(Some("a.tx".into())).unwrap();
    h.run_steps(2);
    let classic = TxOptions::preset(TxPreset::Classic);
    h.state_mut().tx_options.as_mut().unwrap().options = classic.clone();
    h.run_steps(1);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("a.tx".to_string(), asked(Some(&classic)))]);

    h.state_mut().save_as(Some("b.tx".into())).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().tx_options.as_ref().map(|p| p.options.clone()), Some(classic), "starts at the last answer");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().tx_options.is_none());
    assert_eq!(writes.borrow().len(), 1, "cancelled: nothing more written");
    assert_eq!(h.state().ui.status, "Save cancelled");
}

/// Save a Copy to a .tx asks too.
#[test]
fn save_a_copy_to_a_tx_asks() {
    let (mut h, writes) = tx_app("copy.tx");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveACopy", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().tx_options.as_ref().map(|p| p.path.as_str()), Some("copy.tx"));
    assert!(tx_options_ui::park(h.state_mut(), "other.tx".into(), "file.saveAs").is_err(), "the waiting prompt is kept");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("copy.tx".to_string(), asked(Some(&TxOptions::default())))]);
}

/// Enter on a focused Cancel cancels: it never doubles as the OK shortcut.
#[test]
fn enter_on_a_focused_cancel_writes_nothing() {
    use egui_kittest::kittest::Queryable;
    let (mut h, writes) = tx_app("unused.tx");
    h.state_mut().save_as(Some("a.tx".into())).unwrap();
    h.run_steps(2);
    h.get_by_label("Cancel").focus();
    h.run_steps(1);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(writes.borrow().is_empty(), "nothing written");
    assert!(h.state().tx_options.is_none(), "the prompt is closed");
    assert_eq!(h.state().ui.status, "Save cancelled");
}
