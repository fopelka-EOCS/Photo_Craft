//! Dialogs in their own OS window while document windows float.
//!
//! Dialogs are drawn in the main window. With a document floating in its own window (Window ›
//! Arrange › Float in Window), that window stays above the main window, so a dialog drawn there
//! would sit behind it. As in Photoshop, the dialogs then get an OS window of their own: above
//! the document windows, the main window still at the bottom. It is sized to the dialogs, has no
//! decorations (the dialog's own title band moves it), opens centred on the window of the active
//! document, and takes the focus back while a dialog is up. Without floating documents, or with
//! embedded viewports (the web build, tests), dialogs stay in the main window.

use crate::PhotocraftApp;

/// The dialogs' size before they were laid out once.
const FIRST_SIZE: egui::Vec2 = egui::vec2(480.0, 360.0);

/// Whether this frame's dialogs are drawn in their own window.
pub(crate) fn hosts(app: &PhotocraftApp, ctx: &egui::Context) -> bool {
    !ctx.embed_viewports() && !app.ui.dialogs.is_empty() && app.ui.windows.iter().any(|w| w.open)
}

/// The window the dialogs open centred on: the active document's floating window, else the main
/// window (screen points).
fn anchor(app: &PhotocraftApp, ctx: &egui::Context) -> Option<egui::Rect> {
    let active = app.session.active_index();
    let floating = app.ui.windows.iter().find(|w| w.open && w.floating && Some(w.document) == active);
    match floating {
        Some(w) => ctx.input_for(egui::ViewportId::from_hash_of(("docwin", w.id)), |i| i.viewport().outer_rect),
        None => None,
    }
    .or_else(|| ctx.input(|i| i.viewport().outer_rect))
}

/// The window's size for dialogs that last measured `extent` (none yet: a first guess), kept
/// within the main window's area `screen`.
fn window_size(extent: Option<egui::Vec2>, screen: egui::Rect) -> egui::Vec2 {
    let size = extent.filter(|e| e.x.is_finite() && e.y.is_finite() && e.x > 1.0 && e.y > 1.0).unwrap_or(FIRST_SIZE);
    let max = screen.size().max(FIRST_SIZE);
    egui::vec2(size.x.ceil(), size.y.ceil()).clamp(egui::vec2(120.0, 80.0), max)
}

/// Draw the dialogs in their own window (see the module docs). Call instead of `dialogs::show`
/// when [`hosts`] says so.
pub(crate) fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(first) = app.ui.dialogs.first().map(|d| d.id) else { return };
    let title = app.ui.dialogs.last().map(crate::dialogs::display_title).unwrap_or_default();
    let screen = ctx.content_rect();
    let size = window_size(crate::dialogs::hosted_extent(ctx), screen);
    // Placed once per stack of dialogs (keyed by the first): moving it by its title band then
    // sticks, and the next dialogs open centred again.
    let pos_id = egui::Id::new("pc-dialog-window-pos");
    let pos = match ctx.data(|d| d.get_temp::<(u64, egui::Pos2)>(pos_id)) {
        Some((id, p)) if id == first => p,
        _ => {
            let centre = anchor(app, ctx).map_or(egui::pos2(200.0, 200.0), |r| r.center());
            let p = centre - size / 2.0;
            ctx.data_mut(|d| d.insert_temp(pos_id, (first, p)));
            p
        }
    };
    let builder = egui::ViewportBuilder::default()
        .with_title(title)
        .with_decorations(false)
        .with_resizable(false)
        .with_taskbar(false)
        .with_inner_size(size)
        .with_position(pos)
        .with_window_level(egui::WindowLevel::AlwaysOnTop);
    // PhotoCraft has the focus in its main window or a document window: the dialog takes it, as
    // a modal dialog does (never from another application).
    let app_focused =
        ctx.input(|i| i.focused) || app.ui.windows.iter().any(|w| ctx.data(|d| d.get_temp::<bool>(egui::Id::new(("docwin-focused", w.id)))).unwrap_or(false));
    let mut close = false;
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("pc-dialog-window"), builder, |ui, _class| {
        let vctx = ui.ctx().clone();
        if vctx.input(|i| i.viewport().close_requested()) {
            // Alt+F4 on the dialog's window is its Cancel.
            vctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            close = true;
        }
        let focused = vctx.input(|i| i.focused);
        if !focused && app_focused {
            vctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(crate::theme::Tokens::get(&vctx).chrome)).show(ui, |_| {});
        // Keys typed here (⌘Z in a dialog, Esc, Tab) are this window's, not the main window's.
        if focused {
            crate::edit_history::keys(app, &vctx);
        }
        crate::dialogs::set_hosted(&vctx, Some(screen));
        crate::dialogs::show(app, &vctx);
        crate::dialogs::set_hosted(&vctx, None);
    });
    if close && let Some(top) = app.ui.dialogs.last().map(|d| d.id) {
        let _ = crate::dialogs::cancel(app, top);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dialogs_get_their_own_window_only_beside_document_windows() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        app.sync_views();
        crate::menus::invoke(&mut app, &ctx, "edit.keyboardShortcuts", json!({})).unwrap();
        ctx.set_embed_viewports(false);
        assert!(!hosts(&app, &ctx), "no document window: the main window shows the dialog");
        crate::menus::invoke(&mut app, &ctx, "window.arrange.floatInWindow", json!({})).unwrap();
        assert!(hosts(&app, &ctx), "a document window floats: the dialog gets its own window");
        ctx.set_embed_viewports(true);
        assert!(!hosts(&app, &ctx), "embedded viewports (web, tests): the main window");
        ctx.set_embed_viewports(false);
        app.ui.dialogs.clear();
        assert!(!hosts(&app, &ctx), "no dialog, no window");
    }

    #[test]
    fn the_window_fits_the_dialogs_within_the_main_window() {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0));
        assert_eq!(window_size(None, screen), FIRST_SIZE);
        assert_eq!(window_size(Some(egui::vec2(412.3, 288.6)), screen), egui::vec2(413.0, 289.0));
        assert_eq!(window_size(Some(egui::vec2(3000.0, 2000.0)), screen), egui::vec2(1400.0, 900.0));
        for bad in [egui::vec2(f32::NAN, 10.0), egui::vec2(0.0, 0.0), egui::vec2(f32::INFINITY, 300.0)] {
            assert_eq!(window_size(Some(bad), screen), FIRST_SIZE, "{bad:?}");
        }
        let tiny = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 50.0));
        assert_eq!(window_size(Some(egui::vec2(600.0, 500.0)), tiny), egui::vec2(480.0, 360.0), "never smaller than the first guess");
    }
}
