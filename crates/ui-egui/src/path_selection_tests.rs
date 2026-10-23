//! #306: ⌘↩ (Ctrl+Enter off the Mac) loads a path as a selection, as in Photoshop: the path
//! being drawn with the Pen, the path selected in the Paths panel, or the work path. The key is
//! rebindable in Edit › Keyboard Shortcuts, and with no path it says why instead of doing nothing.

use egui::{Event, Key, Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;
use crate::vector_ui::PenPath;

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    h.run_steps(6);
    h
}

/// ⌘ on the Mac, Ctrl elsewhere, as egui-winit reports them.
fn command() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers { command: true, mac_cmd: true, ..Default::default() }
    } else {
        Modifiers { command: true, ctrl: true, ..Default::default() }
    }
}

fn press(h: &mut Harness<'_, PhotocraftApp>, key: Key, m: Modifiers) {
    h.event(Event::ModifiersChanged(m));
    h.event(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: m });
    h.event(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: m });
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(3);
}

fn rect_path(x: f64, y: f64, w: f64, h: f64) -> serde_json::Value {
    let k = |x: f64, y: f64| json!({"anchor": [x, y]});
    json!({"subpaths": [{"closed": true, "knots": [k(x, y), k(x + w, y), k(x + w, y + h), k(x, y + h)]}]})
}

/// The selection's bounds (x, y, width, height), if any.
fn selection(h: &Harness<'_, PhotocraftApp>) -> Option<[i64; 4]> {
    let r = h.state().session.active()?.doc.selection.as_ref()?.content_bounds();
    Some([r.x0, r.y0, r.x1 - r.x0, r.y1 - r.y0].map(i64::from))
}

#[test]
fn cmd_enter_with_the_pen_loads_the_work_path() {
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(10.0, 10.0, 50.0, 40.0)})).unwrap();
    h.state_mut().ui.tool = Tool::Pen;
    h.run_steps(2);
    assert!(h.state().session.active().unwrap().doc.selection.is_none());
    press(&mut h, Key::Enter, command());
    assert!(h.state().session.active().unwrap().doc.selection.is_some(), "⌘↩ made a selection: {}", h.state().ui.status);
    assert_eq!(selection(&h), Some([10, 10, 50, 40]));
}

#[test]
fn cmd_enter_while_drawing_finishes_the_path_and_loads_it() {
    let mut h = harness();
    h.state_mut().ui.tool = Tool::Pen;
    let p = |x: f64, y: f64| [[x, y], [x, y], [x, y]];
    h.state_mut().ui.pen = Some(PenPath { knots: vec![p(20.0, 20.0), p(120.0, 20.0), p(120.0, 80.0)], ..Default::default() });
    h.run_steps(2);
    press(&mut h, Key::Enter, command());
    let app = h.state();
    assert!(app.ui.pen.is_none(), "the pen path was finished");
    assert!(app.session.active().unwrap().doc.work_path.is_none(), "into the work path, and its conversion spends it (Photoshop)");
    assert!(app.session.active().unwrap().doc.selection.is_some(), "and loaded as a selection: {}", app.ui.status);
    // Plain ↩ still only finishes a path.
    let mut h = harness();
    h.state_mut().ui.tool = Tool::Pen;
    h.state_mut().ui.pen = Some(PenPath { knots: vec![p(20.0, 20.0), p(120.0, 20.0), p(120.0, 80.0)], ..Default::default() });
    h.run_steps(2);
    press(&mut h, Key::Enter, Modifiers::NONE);
    assert!(h.state().session.active().unwrap().doc.work_path.is_some(), "plain ↩ only finishes the path: it stays");
    assert!(h.state().session.active().unwrap().doc.selection.is_none());
}

#[test]
fn cmd_enter_loads_the_path_selected_in_the_paths_panel() {
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(0.0, 0.0, 30.0, 30.0)})).unwrap();
    h.state_mut().run("path.set", json!({"name": "Path 1", "path": rect_path(100.0, 40.0, 60.0, 50.0)})).unwrap();
    // Any tool, with a path selected in the Paths panel (as in Photoshop).
    h.state_mut().ui.tool = Tool::Brush;
    h.state_mut().ui.selected_path = Some("Path 1".into());
    h.run_steps(2);
    press(&mut h, Key::Enter, command());
    assert_eq!(selection(&h), Some([100, 40, 60, 50]), "{}", h.state().ui.status);
    // A selected path that went away falls back to the work path.
    h.state_mut().run("path.delete", json!({"name": "Path 1"})).unwrap();
    h.state_mut().run("select.deselect", json!({})).unwrap();
    press(&mut h, Key::Enter, command());
    assert_eq!(selection(&h), Some([0, 0, 30, 30]));
}

#[test]
fn without_a_path_it_says_why_and_the_key_can_be_rebound() {
    let mut h = harness();
    h.state_mut().ui.tool = Tool::Pen;
    press(&mut h, Key::Enter, command());
    let app = h.state();
    assert!(app.session.active().unwrap().doc.selection.is_none());
    assert!(app.ui.status_error && app.ui.status.contains("No path"), "status: {}", app.ui.status);
    // It's a listed, rebindable shortcut.
    let cmd_enter = crate::shortcuts::parse("Cmd+Enter").unwrap();
    assert!(crate::shortcut_dispatch::bindings(app).iter().any(|(id, sc)| id == "path.toSelection" && *sc == cmd_enter));
    h.state_mut().run("edit.keyboardShortcuts", json!({"set": {"path.toSelection": "Cmd+Shift+Y"}})).unwrap();
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(10.0, 10.0, 20.0, 20.0)})).unwrap();
    press(&mut h, Key::Enter, command());
    assert!(h.state().session.active().unwrap().doc.selection.is_none(), "⌘↩ was moved");
    let mut m = command();
    m.shift = true;
    press(&mut h, Key::Y, m);
    assert_eq!(selection(&h), Some([10, 10, 20, 20]), "{}", h.state().ui.status);
}

fn panel(h: &mut Harness<'_, PhotocraftApp>, tab: usize) {
    let ctx = h.ctx.clone();
    let (req, _) = crate::control::ControlRequest::new(
        "ui.set",
        json!({"theme":"promedium", "dockTabs":{"layers":tab}, "dock":{"collapsed":["color","properties","history","navigator"]}}),
    );
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(4);
}

fn click(h: &mut Harness<'_, PhotocraftApp>, at: egui::Pos2, modifiers: Modifiers) {
    h.hover_at(at);
    h.event(Event::ModifiersChanged(modifiers));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers });
    h.run_steps(1);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

#[test]
fn thumbnail_selection_ctrl_click_shape_keeps_active_layer_and_supports_undo() {
    let mut h = harness();
    let shape =
        h.state_mut().run("shape.create", json!({"kind":"rect","rect":[40,20,60,40],"fill":"#cc3333","name":"Badge"})).unwrap()["layer"].as_u64().unwrap();
    let active = h.state_mut().run("layer.new.layer", json!({"name":"Other"})).unwrap()["layer"].as_u64().unwrap();
    panel(&mut h, 0);
    let row = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == shape).unwrap();
    click(&mut h, row.name.unwrap().center(), command());
    assert!(selection(&h).is_none(), "Ctrl-click on a shape's name only selects the layer");
    h.state_mut().run("layer.select", json!({"layer": active})).unwrap();
    h.run_steps(2);
    click(&mut h, egui::pos2(row.row.left() + 46.0, row.row.center().y), command());
    assert_eq!(selection(&h), Some([40, 20, 60, 40]), "{}", h.state().ui.status);
    assert_eq!(h.state().session.active().unwrap().active_layer, Some(photocraft_doc::LayerId(active)));
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert!(selection(&h).is_none());
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!(selection(&h), Some([40, 20, 60, 40]));
}

#[test]
fn thumbnail_selection_ctrl_click_paths_targets_thumbnail_and_clicked_path() {
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name":"work","path":rect_path(10.0,10.0,50.0,40.0)})).unwrap();
    h.state_mut().run("path.set", json!({"name":"Saved","path":rect_path(100.0,40.0,60.0,50.0)})).unwrap();
    h.state_mut().ui.selected_path = Some("Saved".into());
    panel(&mut h, 2);
    let row = h.get_by_label("Work Path").rect();
    click(&mut h, egui::pos2(row.left() + 70.0, row.center().y), command());
    assert!(selection(&h).is_none(), "Ctrl-click on the name only selects the path row");
    h.state_mut().ui.selected_path = Some("Saved".into());
    h.run_steps(2);
    click(&mut h, egui::pos2(row.left() + 22.0, row.center().y), command());
    assert_eq!(selection(&h), Some([10, 10, 50, 40]), "{}", h.state().ui.status);
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("Saved"), "loading a thumbnail does not change the selected path");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert!(selection(&h).is_none());
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!(selection(&h), Some([10, 10, 50, 40]));
    h.state_mut().run("select.deselect", json!({})).unwrap();
    let saved = h.get_by_label("Saved").rect();
    click(&mut h, egui::pos2(saved.left() + 22.0, saved.center().y), command());
    assert_eq!(selection(&h), Some([100, 40, 60, 50]));
}

#[test]
fn thumbnail_selection_ctrl_click_paths_combines_with_shift_and_alt() {
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name":"work","path":rect_path(10.0,10.0,50.0,40.0)})).unwrap();
    panel(&mut h, 2);
    let row = h.get_by_label("Work Path").rect();
    let at = egui::pos2(row.left() + 22.0, row.center().y);
    h.state_mut().run("select.rect", json!({"x":100,"y":40,"width":20,"height":20})).unwrap();
    click(&mut h, at, command() | Modifiers::SHIFT);
    let selected = h.state().session.active().unwrap().doc.selection.as_ref().unwrap();
    assert!(selected.sample_channel(20, 20, 0) > 0.99 && selected.sample_channel(110, 50, 0) > 0.99);
    // ⌘-clicking the Work Path thumbnail spends it (Photoshop), so rebuild it for the Alt part.
    h.state_mut().run("path.set", json!({"name":"work","path":rect_path(10.0,10.0,50.0,40.0)})).unwrap();
    h.state_mut().run("select.all", json!({})).unwrap();
    click(&mut h, at, command() | Modifiers::ALT);
    let selected = h.state().session.active().unwrap().doc.selection.as_ref().unwrap();
    assert!(selected.sample_channel(20, 20, 0) < 0.01 && selected.sample_channel(110, 50, 0) > 0.99);
    h.state_mut().run("select.rect", json!({"x":30,"y":0,"width":100,"height":100})).unwrap();
    h.state_mut().run("path.set", json!({"name":"work","path":rect_path(10.0,10.0,50.0,40.0)})).unwrap();
    click(&mut h, at, command() | Modifiers::SHIFT | Modifiers::ALT);
    assert_eq!(selection(&h), Some([30, 10, 30, 40]));
}

#[test]
fn thumbnail_selection_ctrl_click_layer_path_preserves_panel_target() {
    let mut h = harness();
    h.state_mut().run("shape.create", json!({"kind":"rect","rect":[40,20,60,40],"fill":"#cc3333","name":"Badge"})).unwrap();
    h.state_mut().run("path.set", json!({"name":"work","path":rect_path(0.0,0.0,10.0,10.0)})).unwrap();
    h.state_mut().ui.selected_path = Some("work".into());
    panel(&mut h, 2);
    let row = h.get_by_label("Badge Shape Path").rect();
    click(&mut h, egui::pos2(row.left() + 22.0, row.center().y), command());
    assert_eq!(selection(&h), Some([40, 20, 60, 40]));
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("work"));
    assert!(!h.state().ui.mask_target && !h.state().ui.vector_mask_target);
    if let Ok(path) = std::env::var("PHOTOCRAFT_THUMBNAIL_SELECTION_SCREENSHOT") {
        h.render().unwrap().save(path).unwrap();
    }
}

/// #3915: Clicking a shape with the Path Selection tool selects its layer.
#[test]
fn clicking_shape_with_path_selection_selects_its_layer() {
    let mut h = harness();
    let rect_layer =
        h.state_mut().run("shape.create", json!({"kind": "rect", "rect": [20, 20, 80, 60], "fill": "#ff0000"})).unwrap()["layer"].as_u64().unwrap();
    // Add a new empty layer above so rect is not the active layer.
    let empty_layer = h.state_mut().run("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    assert_eq!(h.state().session.active().unwrap().active_layer.unwrap().0, empty_layer);

    // Clicking inside the rectangle with Path Selection selects the rect layer.
    h.state_mut().ui.tool = Tool::PathSelection;
    crate::vector_ui::path_selection_finish(h.state_mut(), [50.0, 50.0], [50.0, 50.0]);
    assert_eq!(h.state().session.active().unwrap().active_layer.unwrap().0, rect_layer);

    // Clicking outside does not change active layer.
    crate::vector_ui::path_selection_finish(h.state_mut(), [150.0, 90.0], [150.0, 90.0]);
    assert_eq!(h.state().session.active().unwrap().active_layer.unwrap().0, rect_layer);
}

/// Work paths remain visible (outline) regardless of the active tool. Before the fix,
/// `draw_overlay` was gated on `vector_tool` so paths disappeared when switching to Brush, Move,
/// etc. This test renders the canvas with a Brush tool active and a work path present, then
/// confirms the path's outline pixels are painted in the accent colour, proving the path is drawn.
#[test]
fn path_overlay_drawn_for_non_vector_tools() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    // Place a work path whose top edge runs exactly along y=10 in document space.
    app.run("path.set", json!({"name": "work", "path": rect_path(50.0, 10.0, 100.0, 80.0)})).unwrap();
    // Target the row in the Paths panel, as Photoshop does: the outline follows the panel's
    // active path, not the mere existence of the data.
    app.ui.selected_path = Some("work".into());
    // Switch to a non-vector tool — before the fix this hid the path outline.
    app.ui.tool = Tool::Brush;

    let mut h = Harness::builder().with_size(vec2(900.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(6);
    // Fit the view so document coordinates map predictably to screen pixels.
    {
        let v = &mut h.state_mut().ui.views[0];
        v.zoom = 2.0; // 2 px/doc-px → the 200×200 doc fills 400×400 screen pixels
        v.center = [100.0, 100.0];
        v.fit_pending = false;
    }
    h.run_steps(3);

    // Get the accent colour so we know what to look for.
    let accent = crate::theme::Tokens::get(&h.ctx).accent;

    let xf = crate::canvas::ViewXform::active(h.state()).expect("active document view");
    let ppp = h.ctx.pixels_per_point();
    let img = h.render().unwrap();

    // Sample the rendered outline at points derived from the same transform the canvas used. The
    // top edge is doc-y=10; x=70..130 stays away from the rectangle's anchor squares. A 1 px
    // egui stroke is anti-aliased over the white canvas, so accept visible blends of the accent.
    let accent_r = accent.r();
    let accent_g = accent.g();
    let accent_b = accent.b();
    let is_accent_stroke = |pixel: [u8; 4]| {
        let denom = f32::from(255u8.saturating_sub(accent_r)).max(1.0);
        let coverage = f32::from(255u8.saturating_sub(pixel[0])) / denom;
        if !(0.2..=1.05).contains(&coverage) {
            return false;
        }
        let blended = |channel: u8| 255.0 + coverage * (f32::from(channel) - 255.0);
        (f32::from(pixel[1]) - blended(accent_g)).abs() <= 12.0 && (f32::from(pixel[2]) - blended(accent_b)).abs() <= 12.0
    };

    let has_accent_near = |at: egui::Pos2| {
        let cx = (at.x * ppp).round() as i32;
        let cy = (at.y * ppp).round() as i32;
        let radius = (2.0 * ppp).ceil() as i32;
        for y in (cy - radius)..=(cy + radius) {
            for x in (cx - radius)..=(cx + radius) {
                if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
                    let pixel = img.get_pixel(x as u32, y as u32);
                    if is_accent_stroke([pixel[0], pixel[1], pixel[2], pixel[3]]) {
                        return true;
                    }
                }
            }
        }
        false
    };
    let hits = (70..=130).step_by(5).filter(|x| has_accent_near(xf.to_screen(*x as f32, 10.0))).count();
    assert!(
        hits >= 10,
        "expected the work path outline to be visible with Brush tool active, \
         but found accent-coloured pixels at only {hits} sample point(s) along the transformed top edge \
         (accent = #{accent_r:02x}{accent_g:02x}{accent_b:02x}); \
         the path must be drawn regardless of the active tool",
    );
}

/// Is anything targeted as the canvas's path overlay (a Paths panel row, a targeted vector mask,
/// or the active shape layer's path)?
fn path_outline_target(h: &Harness<'_, PhotocraftApp>) -> Option<photocraft_doc::vector::Path> {
    let app = h.state();
    let doc = app.session.active().unwrap().doc.clone();
    crate::vector_ui::visible_target_path(app, &doc)
}

/// Make Selection (⌘↩) spends a converted Work Path, as in Photoshop: the marching ants are
/// all that stays, the Work Path row is gone with its data (a saved path would survive), and
/// one undo step brings both back. Layer via Copy (⌘J) then copies the selection — one
/// undoable step, selection gone, new layer — with no path left behind anywhere, and switching
/// layers brings no stale outline.
#[test]
fn make_selection_consumes_the_work_path_and_layer_via_copy_leaves_nothing() {
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(10.0, 10.0, 50.0, 40.0)})).unwrap();
    h.state_mut().ui.selected_path = Some("work".into());
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(2);
    assert!(path_outline_target(&h).is_some(), "the targeted work path shows");

    press(&mut h, Key::Enter, command());
    let app = h.state();
    let st = app.session.active().unwrap();
    assert!(st.doc.selection.is_some(), "{}", app.ui.status);
    assert!(st.doc.work_path.is_none(), "converting the Work Path spends it, as in Photoshop");
    assert!(app.ui.selected_path.is_none(), "the row is gone, so nothing is targeted");
    assert!(path_outline_target(&h).is_none(), "no path outline: the ant border is the selection's");

    press(&mut h, Key::J, command());
    let app = h.state();
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layers.len(), 2, "Layer via Copy made the raster layer");
    assert!(st.doc.selection.is_none(), "#1096: the selection is gone, in the same step");
    assert!(st.doc.work_path.is_none(), "nothing of the path is left after Ctrl+J");
    assert!(app.ui.selected_path.is_none());
    assert!(path_outline_target(&h).is_none(), "no stale target after Ctrl+J");

    // Switching layers brings no overlay back either: there is no path left to draw.
    let background = st.doc.layers[0].id.0;
    h.state_mut().run("layer.select", json!({"layer": background})).unwrap();
    h.run_steps(2);
    assert!(path_outline_target(&h).is_none(), "no stale outline after switching layers");

    // The first undo only takes Layer Via Copy apart; the second restores the Work Path with
    // the selection-time state. Re-targeting the restored row shows the outline again (the
    // phase drawing model applies to a live path).
    h.state_mut().run("edit.undo", json!({})).unwrap();
    let st = h.state().session.active().unwrap();
    assert!(st.doc.work_path.is_none() && st.doc.selection.is_some(), "the first undo only removes the copy layer");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    let st = h.state().session.active().unwrap();
    assert!(st.doc.work_path.is_some(), "the second undo restores the Work Path");
    assert!(st.doc.selection.is_none());
    h.state_mut().ui.selected_path = Some("work".into());
    h.run_steps(2);
    assert!(path_outline_target(&h).is_some(), "explicitly re-targeted: visible again");
}

/// The whole Pen route: a draft finished by ⌘↩ becomes the Work Path, its conversion spends it
/// as in Photoshop, and ⌘J cuts the seam — nothing of the path is left behind.
#[test]
fn pen_to_selection_to_layer_via_copy_ends_with_the_path_spent() {
    let mut h = harness();
    h.state_mut().ui.tool = Tool::Pen;
    let p = |x: f64, y: f64| [[x, y], [x, y], [x, y]];
    h.state_mut().ui.pen = Some(PenPath { knots: vec![p(20.0, 20.0), p(120.0, 20.0), p(120.0, 80.0)], ..Default::default() });
    h.run_steps(2);

    press(&mut h, Key::Enter, command());
    let app = h.state();
    let st = app.session.active().unwrap();
    assert!(app.ui.pen.is_none(), "the draft is finished");
    assert!(st.doc.work_path.is_none(), "into the work path, and its conversion spends it, as in Photoshop");
    assert!(st.doc.selection.is_some(), "{}", app.ui.status);
    assert!(app.ui.selected_path.is_none(), "no row is left to target");
    assert!(path_outline_target(&h).is_none(), "only the ants show");
    assert!(app.ui.pen.is_none(), "no draft outline lingers either");

    press(&mut h, Key::J, command());
    let st = h.state().session.active().unwrap();
    assert_eq!(st.doc.layers.len(), 2, "the copy layer");
    assert!(st.doc.selection.is_none() && st.doc.work_path.is_none(), "nothing of the workflow is left on the canvas");
    assert_eq!(h.state().ui.selected_path, None);
}

/// A selection made from a row that the Paths panel is not targeting leaves the target alone
/// (converting another row's path must not re-point the panel).
#[test]
fn make_selection_from_another_row_keeps_the_target() {
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(0.0, 0.0, 30.0, 40.0)})).unwrap();
    h.state_mut().run("path.set", json!({"name": "Saved", "path": rect_path(100.0, 40.0, 60.0, 50.0)})).unwrap();
    h.state_mut().ui.selected_path = Some("work".into());
    h.state_mut().run("path.toSelection", json!({"name": "Saved"})).unwrap();
    assert!(h.state().session.active().unwrap().doc.selection.is_some());
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("work"), "the work path stays targeted");
    assert!(path_outline_target(&h).is_some(), "and its outline keeps showing");
    // The same conversion on the targeted row spends it, as in Photoshop's Work Path.
    h.state_mut().run("select.deselect", json!({})).unwrap();
    h.state_mut().run("path.toSelection", json!({"name": "work"})).unwrap();
    assert!(h.state().ui.selected_path.is_none(), "the work path's own conversion deselects it");
    assert!(h.state().session.active().unwrap().doc.work_path.is_none(), "and spends it");
}

/// Saving (renaming) the Work Path keeps it the panel's target under its new name, as Photoshop's
/// Save Path does. Deleting a targeted path keeps the click with its (now missing) row: the stale
/// name draws and edits nothing, and Edit › Undo turns it back into the target when it brings the
/// path back — so ⌫ → ↩ → ⌫ keeps working (#5702).
#[test]
fn save_and_delete_keep_the_paths_panel_target_accurate() {
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(10.0, 10.0, 50.0, 40.0)})).unwrap();
    h.state_mut().ui.selected_path = Some("work".into());
    h.state_mut().run("path.rename", json!({"name": "work", "to": "My Path"})).unwrap();
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("My Path"), "the saved path is the target now");
    assert!(h.state().session.active().unwrap().doc.work_path.is_none());
    let doc = h.state().session.active().unwrap().doc.clone();
    assert!(crate::vector_ui::visible_target_path(h.state(), &doc).is_some(), "the outline follows the row");

    h.state_mut().run("path.delete", json!({"name": "My Path"})).unwrap();
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("My Path"), "the click stays with the deleted row");
    let doc = h.state().session.active().unwrap().doc.clone();
    assert!(crate::vector_ui::visible_target_path(h.state(), &doc).is_none(), "the deleted path draws nothing");

    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(10.0, 10.0, 50.0, 40.0)})).unwrap();
    h.state_mut().ui.selected_path = Some("work".into());
    h.state_mut().run("path.delete", json!({"name": "work"})).unwrap();
    let doc = h.state().session.active().unwrap().doc.clone();
    assert!(crate::vector_ui::visible_target_path(h.state(), &doc).is_none(), "the deleted work path draws nothing");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    let doc = h.state().session.active().unwrap().doc.clone();
    assert!(crate::vector_ui::visible_target_path(h.state(), &doc).is_some(), "undo restores the path and its target");
}

/// Path Selection edits exactly the one path the canvas draws. A work path that exists while
/// the panel targets nothing draws nothing — and then a drag, ⌫/Delete or ⌘T must leave it
/// alone. The old fallback went on moving that invisible path: the tool looked dead ("nothing
/// draws") while it moved something the canvas didn't show. Selecting the Work Path row again
/// restores the outline and the edits. Make Selection (⌘↩) spends the converted Work Path
/// (Photoshop), so the deselected state here arises the ordinary way — a path set, panel not
/// targeting it.
#[test]
fn path_selection_only_edits_the_path_that_draws() {
    let anchors = |p: &photocraft_doc::vector::Path| p.subpaths[0].knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect::<Vec<_>>();
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(10.0, 10.0, 50.0, 40.0)})).unwrap();
    h.state_mut().ui.selected_path = Some("work".into());
    h.state_mut().ui.tool = Tool::PathSelection;
    h.run_steps(2);
    let targeted = h.state().session.active().unwrap().doc.work_path.clone().unwrap();
    assert!(crate::vector_ui::delete_key_deletes_work_path(h.state()), "⌫ deletes the drawn work path (#5702)");

    press(&mut h, Key::Enter, command());
    assert!(selection(&h).is_some(), "⌘↩ made a selection: {}", h.state().ui.status);
    assert!(h.state().ui.selected_path.is_none(), "and spent the work path");
    assert!(h.state().session.active().unwrap().doc.work_path.is_none(), "nothing of it is left to reach");

    // The deselected-but-existing state arises the ordinary way now: a work path the panel
    // does not target. It draws nothing, and Path Selection reaches nothing.
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(10.0, 10.0, 50.0, 40.0)})).unwrap();
    let deselected = h.state().session.active().unwrap().doc.work_path.clone().unwrap();
    let doc = h.state().session.active().unwrap().doc.clone();
    assert_eq!(deselected.subpaths[0].knots.len(), 4, "the data stays while the path hides");
    assert!(crate::vector_ui::visible_target_path(h.state(), &doc).is_none(), "the canvas draws nothing");

    // The tried-to-move anchors, for contrast: a drag of (+40, +30) would give these.
    let moved: Vec<_> = anchors(&deselected).into_iter().map(|[x, y]| [x + 40.0, y + 30.0]).collect();
    crate::vector_ui::path_selection_finish(h.state_mut(), [30.0, 30.0], [70.0, 60.0]);
    let after = h.state().session.active().unwrap().doc.work_path.clone().unwrap();
    assert_eq!(anchors(&after), anchors(&deselected), "the drag left the deselected path alone");
    assert_ne!(anchors(&deselected), moved, "the drag would have moved it: the test bites");
    assert!(!crate::vector_ui::delete_key_deletes_work_path(h.state()), "⌫ reaches Edit › Clear again, as in Photoshop");
    assert!(crate::vector_ui::free_transform_path(h.state()).is_none(), "⌘T is the pixel transform again");

    // Selecting the Work Path row in the Paths panel gives the tool its path back.
    h.state_mut().ui.selected_path = Some("work".into());
    h.run_steps(2);
    let doc = h.state().session.active().unwrap().doc.clone();
    assert!(crate::vector_ui::visible_target_path(h.state(), &doc).is_some(), "the outline is back");
    crate::vector_ui::path_selection_finish(h.state_mut(), [30.0, 30.0], [50.0, 10.0]);
    let retargeted = h.state().session.active().unwrap().doc.work_path.clone().unwrap();
    let want: Vec<_> = anchors(&targeted).into_iter().map(|[x, y]| [x + 20.0, y - 20.0]).collect();
    assert_eq!(anchors(&retargeted), want, "the targeted path moved with the drag");
    assert!(crate::vector_ui::delete_key_deletes_work_path(h.state()), "⌫ deletes it again");
    let tf = crate::vector_ui::free_transform_path(h.state());
    assert_eq!(tf.map(|(p, _)| p), Some(json!({"name": "work"})), "⌘T transforms the path again");
}

/// A saved path targeted in the Paths panel is what Path Selection drags and free transforms,
/// as in Photoshop. The old fallback moved the work path instead — a path other than the one
/// drawn — and moved nothing at all once the work path needed targeting.
#[test]
fn path_selection_moves_the_targeted_saved_path() {
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name": "Path 1", "path": rect_path(10.0, 10.0, 50.0, 40.0)})).unwrap();
    h.state_mut().ui.selected_path = Some("Path 1".into());
    h.state_mut().ui.tool = Tool::PathSelection;
    h.run_steps(2);
    let doc = h.state().session.active().unwrap().doc.clone();
    assert!(crate::vector_ui::visible_target_path(h.state(), &doc).is_some(), "the saved path draws");

    crate::vector_ui::path_selection_finish(h.state_mut(), [30.0, 30.0], [60.0, 50.0]);
    let saved = h.state().session.active().unwrap().doc.paths.iter().find(|p| p.name == "Path 1").map(|p| p.path.clone()).unwrap();
    let anchors: Vec<[f64; 2]> = saved.subpaths[0].knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect();
    assert_eq!(anchors, vec![[40.0, 30.0], [90.0, 30.0], [90.0, 70.0], [40.0, 70.0]], "the saved path moved");
    assert!(h.state().session.active().unwrap().doc.work_path.is_none(), "the work path was never created or touched");

    let tf = crate::vector_ui::free_transform_path(h.state());
    assert_eq!(tf.map(|(p, _)| p), Some(json!({"name": "Path 1"})), "⌘T transforms the saved path");
}

/// The Path Selection options bar's hint tells the truth per state: a drawn path is what a drag
/// edits, the deselected work path sends the user to the Paths panel, and with no paths at all
/// there is nothing to promise (Photoshop shows nothing).
#[test]
fn path_selection_hint_follows_the_panel_row() {
    let mut h = harness();
    h.state_mut().ui.tool = Tool::PathSelection;
    h.run_steps(1);
    assert!(crate::vector_ui::path_selection_hint(h.state()).is_none(), "no paths, no hint");
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(10.0, 10.0, 50.0, 40.0)})).unwrap();
    h.state_mut().ui.selected_path = Some("work".into());
    let hint = crate::vector_ui::path_selection_hint(h.state()).unwrap();
    assert!(hint.contains("Drag to move"), "the targeted path is the drag: {hint}");

    press(&mut h, Key::Enter, command());
    assert!(h.state().ui.selected_path.is_none(), "Make Selection spends the work path");
    assert!(crate::vector_ui::path_selection_hint(h.state()).is_none(), "no paths left, no hint");

    // A deselected-but-existing work path points to the Paths panel, as in Photoshop.
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(10.0, 10.0, 50.0, 40.0)})).unwrap();
    let hint = crate::vector_ui::path_selection_hint(h.state()).unwrap();
    assert!(hint.contains("Paths panel"), "the deselected work path points to the Paths panel: {hint}");

    h.state_mut().ui.selected_path = Some("work".into());
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(2);
    assert!(crate::vector_ui::path_selection_hint(h.state()).is_some(), "the path is back, targeted: {:#?}", crate::vector_ui::path_selection_hint(h.state()));
}
