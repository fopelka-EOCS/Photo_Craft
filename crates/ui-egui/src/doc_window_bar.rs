//! Floating document windows' tab bar, and dragging them, as Photoshop's document windows.
//!
//! A floating document window draws its own bar instead of the system title bar: a tab per
//! document in it (a click shows it, × closes it, the right-click menu sends it back to the main
//! window or into a window of its own), the window's caption buttons, and an empty stretch that
//! moves the window. Dragging that stretch hands the move to the system (smooth, snaps like any
//! window), and the press it ends with comes back to the window (winit posts the button release
//! when the system move ends), so PhotoCraft knows where it was dropped:
//!
//! - on the main window's tab strip (or the top of its document area when it has no tabs): the
//!   documents go back into the main window's tabs;
//! - on another floating window's bar: they become tabs of that window.
//!
//! While dragging, the place it would land is highlighted. Several documents in one window
//! share it ([`DocWindow::group`]); the window shows one at a time. With embedded viewports (the
//! web build, tests) the bar only switches and closes tabs: there is no system window to move.

use egui::{Color32, Id, Pos2, Rect, Sense, Stroke, Vec2, ViewportId, pos2, vec2};

use crate::PhotocraftApp;
use crate::state::DocWindow;
use crate::theme::Tokens;

/// The bar's height, in points.
pub(crate) const BAR_H: f32 = 30.0;
/// How far around the main window's tab strip a drop still docks, in points.
const DOCK_SLACK: f32 = 8.0;

/// The viewport of the OS window with key `key` ([`DocWindow::key`]).
pub(crate) fn viewport(key: u64) -> ViewportId {
    ViewportId::from_hash_of(("docwin", key))
}

/// The open windows grouped by the OS window they show in, in first-seen order: (key, indices
/// into `windows`).
pub(crate) fn groups(windows: &[DocWindow]) -> Vec<(u64, Vec<usize>)> {
    let mut out: Vec<(u64, Vec<usize>)> = Vec::new();
    for (i, w) in windows.iter().enumerate().filter(|(_, w)| w.open) {
        match out.iter_mut().find(|(k, _)| *k == w.key()) {
            Some((_, members)) => members.push(i),
            None => out.push((w.key(), vec![i])),
        }
    }
    out
}

/// Which of a window's documents it shows (an index into `members`): the one the unsaved-changes
/// prompt asks about, else the active document, else the tab picked last, else the first.
pub(crate) fn shown(members: &[DocWindow], active: Option<usize>, asked: Option<usize>, picked: Option<u64>) -> usize {
    let at = |doc: Option<usize>| doc.and_then(|d| members.iter().position(|w| w.document == d));
    at(asked).or_else(|| at(active)).or_else(|| picked.and_then(|id| members.iter().position(|w| w.id == id))).unwrap_or(0)
}

fn picked_id(key: u64) -> Id {
    Id::new(("docwin-picked", key))
}

/// The tab picked last in window `key`.
pub(crate) fn picked(ctx: &egui::Context, key: u64) -> Option<u64> {
    ctx.data(|d| d.get_temp(picked_id(key)))
}

/// Where a drag of a document window would land.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// The main window's tabs.
    Main,
    /// The floating window with this key, as tabs.
    Window(u64),
}

/// The target under `pointer` (screen points) for window `dragged`: another window's bar wins
/// over the main window's tab strip (it is drawn above it).
pub(crate) fn drop_target(pointer: Pos2, main_zone: Option<Rect>, bars: &[(u64, Rect)], dragged: u64) -> Option<Target> {
    if !pointer.is_finite() {
        return None;
    }
    bars.iter()
        .find(|(k, r)| *k != dragged && r.contains(pointer))
        .map(|(k, _)| Target::Window(*k))
        .or_else(|| main_zone.filter(|z| z.contains(pointer)).map(|_| Target::Main))
}

/// What the bars and drops ask for, applied once every window is drawn ([`apply`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// Show this window's document (a tab was clicked) and make it active.
    Pick { key: u64, window: u64 },
    /// Every document of window `key` goes back into the main window's tabs.
    DockAll(u64),
    /// Window `from`'s documents become tabs of window `into`.
    Merge { from: u64, into: u64 },
    /// This one document goes back into the main window's tabs.
    DockOne(u64),
    /// This one document gets a window of its own.
    Separate(u64),
}

/// Apply `action` to the windows (and the active document).
pub(crate) fn apply(app: &mut PhotocraftApp, ctx: &egui::Context, action: Action) {
    match action {
        Action::Pick { key, window } => {
            ctx.data_mut(|d| d.insert_temp(picked_id(key), window));
            if let Some(doc) = app.ui.windows.iter().find(|w| w.id == window).map(|w| w.document) {
                app.session.set_active(doc);
            }
        }
        Action::DockAll(key) => app.ui.windows.retain(|w| w.key() != key),
        Action::Merge { from, into } => {
            if from == into || !app.ui.windows.iter().any(|w| w.key() == into) {
                return;
            }
            let mut moved = None;
            for w in app.ui.windows.iter_mut().filter(|w| w.key() == from) {
                w.group = Some(into);
                moved = moved.or(Some(w.id));
            }
            if let Some(window) = moved {
                apply(app, ctx, Action::Pick { key: into, window });
            }
        }
        Action::DockOne(window) => app.ui.windows.retain(|w| w.id != window),
        Action::Separate(window) => {
            let alone = app.ui.windows.iter().find(|w| w.id == window).is_some_and(|w| app.ui.windows.iter().filter(|x| x.key() == w.key()).count() < 2);
            if alone {
                return;
            }
            // A new id is a new OS window; the others keep theirs (their key may be this
            // window's old id, which stays the key of their window).
            let id = app.ui.alloc_id();
            if let Some(w) = app.ui.windows.iter_mut().find(|w| w.id == window) {
                w.id = id;
                w.group = None;
                w.position = None;
            }
        }
    }
}

/// A drag of window `key`'s bar handed to the system: where on the window it was grabbed (from
/// the window's outer top-left), and the frame it started.
#[derive(Clone, Copy, Debug)]
struct Drag {
    key: u64,
    grab: Vec2,
    pass: u64,
}

fn drag_id() -> Id {
    Id::new("docwin-drag")
}

fn target_id() -> Id {
    Id::new("docwin-drop-target")
}

fn bars_id() -> Id {
    Id::new("docwin-bars")
}

/// The main window's dock zone in its own points: its tab strip, else the top of its document
/// area, grown by [`DOCK_SLACK`].
fn main_zone_local(app: &PhotocraftApp) -> Option<Rect> {
    let strip = app.tab_strip.as_ref().map(|s| s.rect);
    let top = app.drop_canvas_rect.map(|r| Rect::from_min_size(r.min, vec2(r.width(), 40.0_f32.min(r.height()))));
    strip.or(top).map(|r| r.expand(DOCK_SLACK))
}

/// Once a frame, in the main window's pass, before the document windows: where a dragged
/// window would land now, with the dock guide drawn over the main window's tabs when that is
/// where. The windows read the target from here ([`target`]).
pub(crate) fn track(app: &PhotocraftApp, ctx: &egui::Context) {
    let drag = ctx.data(|d| d.get_temp::<Drag>(drag_id()));
    let target = drag.and_then(|d| {
        let outer = ctx.input_for(viewport(d.key), |i| i.viewport().outer_rect)?;
        let root = ctx.input(|i| i.viewport().inner_rect)?;
        let zone = main_zone_local(app).map(|z| z.translate(root.min.to_vec2()));
        let bars: Vec<(u64, Rect)> = ctx.data(|m| m.get_temp(bars_id())).unwrap_or_default();
        drop_target(outer.min + d.grab, zone, &bars, d.key)
    });
    ctx.data_mut(|m| m.insert_temp(target_id(), target));
    if drag.is_some() {
        // The system moves the window: keep frames coming so the guide follows it.
        ctx.request_repaint();
    }
    if target == Some(Target::Main)
        && let Some(zone) = main_zone_local(app)
    {
        let t = Tokens::get(ctx);
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("docwin-dock-guide")));
        painter.rect_filled(zone, t.radius_sm, t.accent.gamma_multiply(0.18));
        painter.rect_stroke(zone, t.radius_sm, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
        painter.text(zone.center(), egui::Align2::CENTER_CENTER, tl!("Dock as tabs"), egui::FontId::proportional(13.0), t.text);
    }
}

fn target(ctx: &egui::Context) -> Option<Target> {
    ctx.data(|m| m.get_temp::<Option<Target>>(target_id())).flatten()
}

/// Forget the bars recorded last frame; each window records its own as it is drawn.
pub(crate) fn begin_frame(ctx: &egui::Context) {
    ctx.data_mut(|m| m.insert_temp(bars_id(), Vec::<(u64, Rect)>::new()));
}

/// Window `key`'s bar, drawn at the top of its viewport (`ui`): its tabs (`members`, the one at
/// `shown` in front), caption buttons and the stretch that moves the window. Returns what was
/// asked. `close` is set when the window's close button was clicked; a tab's × adds its document
/// to `close_docs` (closed through the guarded File › Close by the caller). `own_window`: a real
/// OS window (else embedded, with nothing to move, minimise or maximise).
#[allow(clippy::too_many_arguments)]
pub(crate) fn bar(
    app: &PhotocraftApp,
    ui: &mut egui::Ui,
    key: u64,
    members: &[DocWindow],
    shown: usize,
    own_window: bool,
    close: &mut bool,
    close_docs: &mut Vec<photocraft_doc::DocId>,
) -> Vec<Action> {
    let wctx = ui.ctx().clone();
    let t = Tokens::get(&wctx);
    let mut actions = Vec::new();
    let frame = egui::Frame::NONE.fill(t.tab_strip);
    egui::Panel::top(Id::new(("docwin-bar", key))).resizable(false).exact_size(BAR_H).frame(frame).show(ui, |ui| {
        let bar = ui.max_rect();
        let (left, right) = if own_window { crate::titlebar::caption_sides(app, &wctx) } else { (0.0, 0.0) };
        // Record the bar in screen points for the other windows' drags.
        if own_window && let Some(inner) = wctx.input(|i| i.viewport().inner_rect) {
            let on_screen = bar.translate(inner.min.to_vec2());
            wctx.data_mut(|m| m.get_temp_mut_or_default::<Vec<(u64, Rect)>>(bars_id()).push((key, on_screen)));
        }
        // The stretch that moves the window, under the tabs and buttons.
        let grip = Rect::from_min_max(pos2(bar.left() + left, bar.top()), pos2(bar.right() - right, bar.bottom()));
        let moving = ui.interact(grip, Id::new(("docwin-grip", key)), Sense::click_and_drag());
        if own_window && moving.drag_started_by(egui::PointerButton::Primary) {
            let at = wctx.input(|i| Some((i.viewport().outer_rect?, i.viewport().inner_rect?, i.pointer.press_origin()?)));
            if let Some((outer, inner, press)) = at {
                let grab = inner.min + press.to_vec2() - outer.min;
                let pass = wctx.cumulative_pass_nr();
                wctx.data_mut(|m| m.insert_temp(drag_id(), Drag { key, grab, pass }));
                wctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
        }
        if own_window && moving.double_clicked() {
            crate::titlebar::toggle_maximized(&wctx);
        }
        // Tabs.
        let font = egui::FontId::proportional(12.0);
        let mut x = bar.left() + left + 6.0;
        let room = (bar.right() - right - 6.0 - x).max(0.0);
        let width = (room / members.len().max(1) as f32).clamp(60.0, 220.0);
        for (i, w) in members.iter().enumerate() {
            let Some(st) = app.session.documents().get(w.document) else { continue };
            let r = Rect::from_min_size(pos2(x, bar.top() + 3.0), vec2(width, BAR_H - 3.0));
            x += width + 2.0;
            let front = i == shown;
            let resp = ui.interact(r, Id::new(("docwin-tab", w.id)), Sense::click());
            let name = crate::canvas::studio_tab_name(app, st);
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, front, &name));
            let fill = if front {
                t.chrome
            } else if resp.hovered() {
                t.hover
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect_filled(r, egui::CornerRadius { nw: t.radius_sm as u8, ne: t.radius_sm as u8, sw: 0, se: 0 }, fill);
            let xr = Rect::from_center_size(pos2(r.right() - 12.0, r.center().y), vec2(16.0, 16.0));
            let g = crate::tab_strip::elided(ui, &name, font.clone(), t.text, (r.width() - 34.0).max(1.0));
            ui.painter().galley_with_override_text_color(pos2(r.left() + 8.0, r.center().y - g.size().y / 2.0), g, if front { t.text } else { t.text_dim });
            let xresp = ui.interact(xr, Id::new(("docwin-tabx", w.id)), Sense::click());
            xresp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Close")));
            if xresp.hovered() {
                ui.painter().rect_filled(xr, 4.0, t.hover);
            }
            crate::icons::paint(ui, xr, "x", 10.0, if xresp.hovered() { t.text } else { t.text_faint });
            if xresp.clicked() {
                close_docs.push(st.doc.id);
            } else if resp.clicked() && !front {
                actions.push(Action::Pick { key, window: w.id });
            }
            resp.context_menu(|ui| {
                if ui.button(tl!("Move to Main Window")).clicked() {
                    actions.push(Action::DockOne(w.id));
                    ui.close();
                }
                if members.len() > 1 && ui.button(tl!("Move to New Window")).clicked() {
                    actions.push(Action::Separate(w.id));
                    ui.close();
                }
            });
        }
        if own_window && crate::titlebar::caption_row(app, ui, bar) {
            *close = true;
        }
        // The guide: this window's bar is where the dragged one would land.
        if target(&wctx) == Some(Target::Window(key)) {
            ui.painter().rect_filled(bar, 0.0, t.accent.gamma_multiply(0.18));
            ui.painter().rect_stroke(bar, 0.0, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
        }
    });
    // A drag of this window ended (the system gives the press back when the move ends): drop it.
    if let Some(d) = wctx.data(|m| m.get_temp::<Drag>(drag_id())).filter(|d| d.key == key)
        && d.pass + 1 < wctx.cumulative_pass_nr()
        && !wctx.input(|i| i.pointer.primary_down())
    {
        wctx.data_mut(|m| m.remove::<Drag>(drag_id()));
        match target(&wctx) {
            Some(Target::Main) => actions.push(Action::DockAll(key)),
            Some(Target::Window(into)) => actions.push(Action::Merge { from: key, into }),
            None => {}
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn win(id: u64, document: usize, group: Option<u64>) -> DocWindow {
        DocWindow { id, document, view: Default::default(), open: true, position: None, floating: true, group }
    }

    #[test]
    fn windows_group_by_the_os_window_they_show_in() {
        let mut ws = vec![win(1, 0, None), win(2, 1, Some(1)), win(3, 2, None), win(4, 3, Some(3))];
        ws[3].open = false;
        assert_eq!(groups(&ws), vec![(1, vec![0, 1]), (3, vec![2])]);
        assert!(groups(&[]).is_empty());
    }

    #[test]
    fn a_window_shows_the_asked_then_active_then_picked_document() {
        let ms = vec![win(1, 0, None), win(2, 5, Some(1)), win(3, 7, Some(1))];
        assert_eq!(shown(&ms, None, None, None), 0);
        assert_eq!(shown(&ms, None, None, Some(3)), 2, "the tab picked last");
        assert_eq!(shown(&ms, Some(5), None, Some(3)), 1, "the active document wins");
        assert_eq!(shown(&ms, Some(5), Some(7), None), 2, "the prompt's document wins");
        assert_eq!(shown(&ms, Some(99), None, Some(42)), 0, "unknown documents and windows: the first");
    }

    #[test]
    fn drops_land_on_another_bar_before_the_main_tabs() {
        let main = Some(Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 40.0)));
        let bars = vec![(7, Rect::from_min_max(pos2(100.0, 10.0), pos2(400.0, 40.0))), (9, Rect::from_min_max(pos2(600.0, 500.0), pos2(900.0, 530.0)))];
        assert_eq!(drop_target(pos2(200.0, 20.0), main, &bars, 9), Some(Target::Window(7)), "a bar over the main tabs wins");
        assert_eq!(drop_target(pos2(200.0, 20.0), main, &bars, 7), Some(Target::Main), "never onto its own bar");
        assert_eq!(drop_target(pos2(700.0, 510.0), main, &bars, 7), Some(Target::Window(9)));
        assert_eq!(drop_target(pos2(500.0, 300.0), main, &bars, 7), None, "anywhere else: just moved");
        assert_eq!(drop_target(pos2(500.0, 20.0), None, &bars, 7), None, "no main tabs to dock into");
        assert_eq!(drop_target(pos2(f32::NAN, 20.0), main, &bars, 7), None);
    }

    fn app_with_floating(n: usize) -> (PhotocraftApp, egui::Context) {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for i in 0..n {
            app.run("file.new", json!({"width": 8, "height": 8, "name": format!("d{i}")})).unwrap();
        }
        app.sync_views();
        for i in 0..n {
            crate::menus::invoke(&mut app, &ctx, "window.arrange.floatInWindow", json!({"document": i})).unwrap();
        }
        (app, ctx)
    }

    #[test]
    fn merging_docking_and_separating_windows() {
        let (mut app, ctx) = app_with_floating(3);
        let ids: Vec<u64> = app.ui.windows.iter().map(|w| w.id).collect();
        assert_eq!(groups(&app.ui.windows).len(), 3);
        // d1's window dropped on d0's bar: one window with two tabs, d1 in front.
        apply(&mut app, &ctx, Action::Merge { from: ids[1], into: ids[0] });
        assert_eq!(groups(&app.ui.windows), vec![(ids[0], vec![0, 1]), (ids[2], vec![2])]);
        assert_eq!(app.session.active_index(), Some(1));
        assert_eq!(picked(&ctx, ids[0]), Some(ids[1]));
        assert!((0..3).all(|d| app.ui.floats(d)), "merged documents still float");
        // A merge into a window that isn't there, or into itself, changes nothing.
        apply(&mut app, &ctx, Action::Merge { from: ids[2], into: 999 });
        apply(&mut app, &ctx, Action::Merge { from: ids[2], into: ids[2] });
        assert_eq!(groups(&app.ui.windows).len(), 2);
        // d1 back into a window of its own: a new OS window.
        apply(&mut app, &ctx, Action::Separate(ids[1]));
        assert_eq!(groups(&app.ui.windows).len(), 3);
        assert!(app.ui.windows.iter().all(|w| w.group.is_none()));
        assert!(!app.ui.windows.iter().any(|w| w.id == ids[1]), "separated under a new id");
        // Separating the only document of a window does nothing.
        let alone = app.ui.windows[2].id;
        apply(&mut app, &ctx, Action::Separate(alone));
        assert_eq!(app.ui.windows[2].id, alone);
        // Dropped on the main window's tabs: all its documents dock.
        apply(&mut app, &ctx, Action::Merge { from: ids[2], into: ids[0] });
        apply(&mut app, &ctx, Action::DockAll(ids[0]));
        assert!(!app.ui.floats(0) && !app.ui.floats(2), "d0 and d2 are tabs again");
        assert!(app.ui.floats(1));
        // One tab sent back from its window.
        let d1 = app.ui.windows.iter().find(|w| w.document == 1).map(|w| w.id).unwrap();
        apply(&mut app, &ctx, Action::DockOne(d1));
        assert!(app.ui.windows.is_empty());
        assert_eq!(app.session.documents().len(), 3, "docking never closes a document");
    }

    /// The bar of a window holding two documents shows both as tabs; a click on the back one
    /// brings it to the front and makes it active.
    #[test]
    fn the_bar_shows_a_tab_per_document_and_switches_them() {
        let (mut app, ctx) = app_with_floating(2);
        let ids: Vec<u64> = app.ui.windows.iter().map(|w| w.id).collect();
        apply(&mut app, &ctx, Action::Merge { from: ids[1], into: ids[0] });
        app.session.set_active(0);
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1200.0, 800.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let ctx = ui.ctx().clone();
                if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
                crate::canvas::extra_windows(app, &ctx);
            },
            app,
        );
        crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(6);
        use egui_kittest::kittest::Queryable;
        assert!(h.query_by_label("d0").is_some() && h.query_by_label("d1").is_some(), "both tabs");
        h.get_by_label("d1").click();
        h.run_steps(3);
        assert_eq!(h.state().session.active_index(), Some(1), "the clicked tab's document is active");
        assert_eq!(picked(&h.ctx, ids[0]), Some(ids[1]));
    }
}
