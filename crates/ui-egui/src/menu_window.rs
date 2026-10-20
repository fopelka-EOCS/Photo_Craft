//! The menu bar's menus in OS windows of their own while documents float in theirs.
//!
//! The menus are popups drawn inside the main window. A floating document window stays above the
//! main window ([`crate::canvas::extra_windows`]), so a menu drawn there would be under it, and
//! raising the main window for it would put the main window over the documents. As with
//! Photoshop's (native) menus, each level of an open menu is then a borderless window of its own,
//! above the document windows: the main window at the bottom, the document windows over it, the
//! menu on top.
//!
//! The titles stay in the main window's bar ([`crate::menus::menu_bar`]): a press opens or closes
//! a menu, a moving pointer over another title switches to it, Alt / F10 and the mnemonics open
//! one ([`adopt_popups`]). The menu's windows show the same items as the in-window menus: labels,
//! check marks, shortcuts, menu colours and disabled rows. Pointing at a submenu opens it beside
//! its row; ↑ ↓ ← → ↩ Space and Esc work as in the in-window menus. A press elsewhere closes them.
//! The windows never take the focus when they open, so the keys stay with the main window.
//!
//! Only while a floating document window is open, and never with embedded viewports (the web
//! build, tests): otherwise the menus stay the main window's popups.

use egui::{Id, Key, Modifiers, Pos2, Rect, Vec2, ViewportId, pos2, vec2};
use serde_json::json;

use crate::PhotocraftApp;
use crate::menus::{MenuItem, TOP_MENUS};

/// The open menu.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Open {
    /// The open top-level menu (index into [`TOP_MENUS`]).
    pub top: usize,
    /// The open submenus below it, by name (English, as in the items' paths).
    pub path: Vec<String>,
    /// The highlighted row of each open level (the keyboard's, and the pointer's).
    pub hi: Vec<Option<usize>>,
    /// The open title's rect in screen points: the menu hangs below it.
    pub title: Rect,
    /// Per open submenu (level 1 on): its row in the level above, in screen points.
    pub anchors: Vec<Rect>,
}

impl Default for Open {
    fn default() -> Self {
        Self { top: 0, path: Vec::new(), hi: Vec::new(), title: Rect::NOTHING, anchors: Vec::new() }
    }
}

fn state_id() -> Id {
    Id::new("pc-menu-window")
}

/// Whether the menus open in their own windows now (see the module docs).
pub(crate) fn hosted(app: &PhotocraftApp, ctx: &egui::Context) -> bool {
    !ctx.embed_viewports() && app.ui.windows.iter().any(|w| w.open && w.floating)
}

pub(crate) fn current(ctx: &egui::Context) -> Option<Open> {
    ctx.data(|d| d.get_temp::<Option<Open>>(state_id())).flatten()
}

fn set(ctx: &egui::Context, open: Option<Open>) {
    ctx.data_mut(|d| d.insert_temp(state_id(), open));
    ctx.request_repaint();
}

/// Close the open menu, if any.
pub(crate) fn close(ctx: &egui::Context) {
    if current(ctx).is_some() {
        set(ctx, None);
    }
}

fn open_top(ctx: &egui::Context, top: usize, title: Rect) {
    set(ctx, Some(Open { top, title, ..Default::default() }));
}

/// The main window's position on screen (its content's top-left), to turn its points into
/// screen points.
fn main_origin(ctx: &egui::Context) -> Vec2 {
    ctx.input_for(ViewportId::ROOT, |i| i.viewport().inner_rect).map_or(Vec2::ZERO, |r| r.min.to_vec2())
}

/// A menu title of the main window's bar (`top` its index in [`TOP_MENUS`]): a press opens its
/// menu, or closes it when open; a pointer moving onto it while another menu is open switches.
pub(crate) fn title(ctx: &egui::Context, top: usize, resp: &egui::Response) {
    let on_screen = resp.rect.translate(main_origin(ctx));
    let open = current(ctx);
    let pressed = ctx.input(|i| i.pointer.primary_pressed()) && (resp.is_pointer_button_down_on() || resp.clicked());
    if pressed {
        if open.as_ref().is_some_and(|o| o.top == top) {
            close(ctx);
        } else {
            open_top(ctx, top, on_screen);
        }
        return;
    }
    match open {
        Some(o) if o.top == top => {
            if o.title != on_screen {
                set(ctx, Some(Open { title: on_screen, ..o }));
            }
        }
        Some(_) if resp.hovered() && ctx.input(|i| i.pointer.delta() != Vec2::ZERO) => open_top(ctx, top, on_screen),
        _ => {}
    }
}

/// The bar's keyboard (Alt / F10, mnemonics) opens a title's egui popup: show that menu in its
/// windows instead, and close the popup. `ids` and `titles` are in [`TOP_MENUS`] order.
pub(crate) fn adopt_popups(ctx: &egui::Context, ids: &[Id], titles: &[egui::Response]) {
    for (top, id) in ids.iter().enumerate() {
        if egui::Popup::is_id_open(ctx, *id) {
            egui::Popup::close_id(ctx, *id);
            if let Some(t) = titles.get(top) {
                let mut open = Open { top, title: t.rect.translate(main_origin(ctx)), ..Default::default() };
                // Opened from the keyboard: the first row is highlighted, as in the bar's popups.
                open.hi = vec![None];
                set(ctx, Some(open));
            }
        }
    }
}

/// One row of a menu level.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Row {
    Item {
        id: String,
        label: String,
        shortcut: Option<String>,
        enabled: bool,
        checked: bool,
        color: Option<String>,
    },
    /// A submenu: its name (English, a path segment) and label.
    Sub {
        name: String,
        label: String,
        enabled: bool,
    },
    Separator,
}

impl Row {
    fn actionable(&self) -> bool {
        match self {
            Row::Item { enabled, .. } | Row::Sub { enabled, .. } => *enabled,
            Row::Separator => false,
        }
    }
}

/// The rows of the level under `top` › `path`, in Photoshop's order as the in-window menus show
/// them ([`crate::menus`]): commands and separators at this depth, a submenu where its first
/// command is; no separator first, last or twice.
pub(crate) fn rows(items: &[MenuItem], top: &str, path: &[String], lang: crate::i18n::Lang) -> Vec<Row> {
    let depth = path.len() + 1;
    let mine: Vec<&MenuItem> =
        items.iter().filter(|it| it.path.first().map(String::as_str) == Some(top) && it.path.len() >= depth && it.path.get(1..depth) == Some(path)).collect();
    let mut out: Vec<Row> = Vec::new();
    let mut subs: Vec<&str> = Vec::new();
    for it in &mine {
        if it.path.len() == depth {
            if it.label == "---" {
                if !matches!(out.last(), None | Some(Row::Separator)) {
                    out.push(Row::Separator);
                }
                continue;
            }
            out.push(Row::Item {
                id: it.id.clone(),
                label: crate::menus::translated_menu_label(lang, it),
                shortcut: it.shortcut.as_deref().map(crate::shortcuts::pretty),
                enabled: it.enabled,
                checked: it.checked == Some(true),
                color: it.color.clone(),
            });
        } else if let Some(name) = it.path.get(depth).map(String::as_str)
            && !subs.contains(&name)
        {
            subs.push(name);
            let children: Vec<&MenuItem> = mine.iter().copied().filter(|c| c.path.get(depth).map(String::as_str) == Some(name)).collect();
            out.push(Row::Sub { name: name.to_string(), label: crate::i18n::tr(lang, name).to_string(), enabled: crate::menus::submenu_enabled(&children) });
        }
    }
    while matches!(out.last(), Some(Row::Separator)) {
        out.pop();
    }
    out
}

/// The next actionable row from `from` going down (or up), wrapping; `None` when there is none.
pub(crate) fn step(rows: &[Row], from: Option<usize>, down: bool) -> Option<usize> {
    let n = rows.len();
    if n == 0 {
        return None;
    }
    let start = match (from, down) {
        (None, true) => n - 1,
        (None, false) => 0,
        (Some(i), _) => i.min(n - 1),
    };
    (1..=n).map(|k| if down { (start + k) % n } else { (start + n - k % n) % n }).find(|&i| rows.get(i).is_some_and(Row::actionable))
}

fn rows_id(level: usize) -> Id {
    Id::new(("pc-menu-window-rows", level))
}

/// Where a level's window goes: below the title (level 0) or beside the row it hangs from, kept
/// on the monitor (`screen`, in points): a submenu with no room on the right opens on the left.
pub(crate) fn place(level: usize, open: &Open, size: Vec2, screen: Rect) -> Pos2 {
    let mut p = if level == 0 {
        open.title.left_bottom()
    } else {
        let a = open.anchors.get(level - 1).copied().unwrap_or(open.title);
        let right = pos2(a.right() + 2.0, a.top() - 4.0);
        if right.x + size.x > screen.right() { pos2(a.left() - 2.0 - size.x, right.y) } else { right }
    };
    p.x = p.x.min(screen.right() - size.x).max(screen.left());
    p.y = p.y.min(screen.bottom() - size.y).max(screen.top());
    p
}

/// What a level's window reported this frame.
#[derive(Default)]
struct LevelOut {
    /// The pointer moved onto this row.
    hovered: Option<usize>,
    /// This row was clicked (or the press that opened the menu was released on it).
    clicked: Option<usize>,
    /// The rows' rects in screen points.
    rects: Vec<Rect>,
}

/// Levels whose windows are kept (parked off screen) while the menus are hosted, so opening a
/// menu moves a window that already exists instead of creating one (which takes a moment and
/// can flash).
const PREWARM: usize = 2;

/// Where a window waits off screen. Parked, not hidden: egui hands the glyphs it rasterises to
/// the window whose pass ends next, and a hidden window isn't painted, so its glyphs never
/// reached the GPU and the menus' text was missing. A parked window is painted like any other.
const PARK: Pos2 = pos2(-20000.0, -20000.0);

/// A level's window: on screen and on top once its size is known (`known`), else parked at the
/// normal level while it is measured. The change of level as it comes on screen raises it over
/// the document windows (it may have been created before them) without taking the focus.
fn presentation(known: Option<Vec2>) -> (bool, egui::WindowLevel) {
    if known.is_some() { (true, egui::WindowLevel::AlwaysOnTop) } else { (false, egui::WindowLevel::Normal) }
}

fn viewport(depth: usize) -> ViewportId {
    ViewportId::from_hash_of(("pc-menu-window", depth))
}

/// The size a level of this menu measured last time (`None`: never shown yet).
fn size_key(open: &Open, depth: usize) -> Id {
    Id::new(("pc-menu-window-size", open.top, open.path.get(..depth).unwrap_or(&[])))
}

/// The open levels' rows for `open` (a submenu that no longer exists closes, with the ones below).
fn levels_of(items: &[MenuItem], open: &mut Open, lang: crate::i18n::Lang) -> Vec<Vec<Row>> {
    let Some(top) = TOP_MENUS.get(open.top).copied() else { return Vec::new() };
    let mut levels: Vec<Vec<Row>> = Vec::new();
    for depth in 0..=open.path.len() {
        let r = rows(items, top, open.path.get(..depth).unwrap_or(&[]), lang);
        if r.is_empty() && depth > 0 {
            open.path.truncate(depth - 1);
            break;
        }
        levels.push(r);
    }
    open.hi.resize(levels.len(), None);
    open.anchors.truncate(levels.len().saturating_sub(1));
    levels
}

/// Draw one level's rows as the in-window menus draw theirs (egui's menu style, the Spectrum
/// menu rows, a hovered look for the highlighted row and the open submenu's row). Returns what
/// happened, rows' rects in this window's points.
fn draw_rows(ui: &mut egui::Ui, level: &[Row], hi: Option<usize>, open_sub: Option<&str>, interact: bool) -> LevelOut {
    let mut out = LevelOut::default();
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
    crate::widgets::style_spectrum_popup_menu(ui);
    let moved = ui.input(|i| i.pointer.delta() != Vec2::ZERO);
    let released = ui.input(|i| i.pointer.primary_released());
    for (i, row) in level.iter().enumerate() {
        let lit = hi == Some(i) || matches!(row, Row::Sub { name, .. } if open_sub == Some(name.as_str()));
        let saved = (ui.visuals().widgets.inactive, ui.visuals().widgets.hovered);
        if lit {
            ui.visuals_mut().widgets.inactive = ui.visuals().widgets.hovered;
        }
        let resp = match row {
            Row::Separator => {
                out.rects.push(ui.add(egui::Separator::default().spacing(9.0)).rect);
                None
            }
            Row::Item { label, shortcut, enabled, checked, color, .. } => {
                let mut b = egui::Button::new(crate::menu_keys::menu_row_text(ui, *checked, label, None));
                if let Some(tint) = color.as_deref().and_then(crate::menus::menu_tint) {
                    b = b.fill(tint.gamma_multiply(0.55));
                }
                if let Some(sc) = shortcut {
                    b = b.shortcut_text(sc.as_str());
                }
                Some(ui.add_enabled(*enabled, b))
            }
            Row::Sub { label, enabled, .. } => {
                let arrow = if crate::direction::is_rtl() { "⏴" } else { egui::containers::menu::SubMenuButton::RIGHT_ARROW };
                let b = egui::Button::new(crate::menu_keys::menu_row_text(ui, false, label, None)).right_text(arrow);
                Some(ui.add_enabled(*enabled, b))
            }
        };
        (ui.visuals_mut().widgets.inactive, ui.visuals_mut().widgets.hovered) = saved;
        if let Some(r) = resp {
            out.rects.push(r.rect);
            if interact {
                if r.hovered() && moved {
                    out.hovered = Some(i);
                }
                if r.clicked() || (released && r.contains_pointer() && r.enabled()) {
                    out.clicked = Some(i);
                }
            }
        }
    }
    out
}

/// A level's rows in its window's panel, as egui's menus lay theirs out: at the rows' own width
/// (the longest label and shortcut, at least `min_width`), drawn full width, from the top.
fn level_ui(ui: &mut egui::Ui, level: &[Row], hi: Option<usize>, open_sub: Option<&str>, interact: bool, min_width: f32) -> egui::InnerResponse<LevelOut> {
    egui::containers::menu::menu_style(ui.style_mut());
    // Measured in a child that takes no room: `scope_builder` would have taken its height and
    // pushed the rows below the window's bottom (the menus showed no rows).
    let mut sizing = ui.new_child(egui::UiBuilder::new().id_salt("sizing").sizing_pass().invisible());
    draw_rows(&mut sizing, level, None, None, false);
    let width = sizing.min_rect().width().max(min_width);
    ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
        ui.set_width(width);
        draw_rows(ui, level, hi, open_sub, interact)
    })
}

/// Draw the menu's windows and drive them (see the module docs). Once a frame in the main
/// window's pass, after its menu bar, whether a menu is open or not: the first levels' windows
/// are kept hidden meanwhile ([`PREWARM`]).
pub(crate) fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !hosted(app, ctx) {
        // Not drawn: the windows close.
        close(ctx);
        return;
    }
    let mut open = current(ctx);
    // PhotoCraft lost the focus to another application: the menu closes, as native ones do.
    if let Some(o) = &open {
        let ours = ctx.input(|i| i.focused)
            || app.ui.windows.iter().any(|w| ctx.data(|d| d.get_temp::<bool>(Id::new(("docwin-focused", w.key())))).unwrap_or(false))
            || (0..=o.path.len()).any(|depth| ctx.input_for(viewport(depth), |i| i.focused));
        let lost = Id::new("pc-menu-window-unfocused-since");
        let now = ctx.input(|i| i.time);
        if ours {
            ctx.data_mut(|d| d.remove::<f64>(lost));
        } else {
            let since = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(lost, || now));
            if now - since > 0.3 {
                ctx.data_mut(|d| d.remove::<f64>(lost));
                close(ctx);
                open = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(350));
            }
        }
    }
    let lang = crate::i18n::current();
    let items = if open.is_some() { crate::menus::menu_items(app) } else { Vec::new() };
    let mut levels = open.as_mut().map(|o| levels_of(&items, o, lang)).unwrap_or_default();
    let mut run: Option<String> = None;

    // The keyboard, which stays with the main window (the menu windows never take the focus).
    if let Some(o) = open.as_mut() {
        let deepest = levels.len().saturating_sub(1);
        let key = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
        let n = TOP_MENUS.len();
        let rtl = crate::direction::is_rtl();
        let mut closed = false;
        if let Some(level) = levels.get(deepest) {
            let hi = o.hi.get(deepest).copied().flatten();
            if key(Key::ArrowDown) {
                if let Some(h) = o.hi.get_mut(deepest) {
                    *h = step(level, hi, true);
                }
            } else if key(Key::ArrowUp) {
                if let Some(h) = o.hi.get_mut(deepest) {
                    *h = step(level, hi, false);
                }
            } else if key(Key::ArrowRight) {
                match hi.and_then(|i| level.get(i)) {
                    Some(Row::Sub { name, enabled: true, .. }) => {
                        o.path.push(name.clone());
                        o.hi.push(None);
                    }
                    _ => {
                        *o = Open { top: if rtl { (o.top + n - 1) % n } else { (o.top + 1) % n }, hi: vec![None], title: o.title, ..Default::default() };
                    }
                }
            } else if key(Key::ArrowLeft) {
                if o.path.pop().is_some() {
                    o.hi.truncate(o.path.len() + 1);
                } else {
                    *o = Open { top: if rtl { (o.top + 1) % n } else { (o.top + n - 1) % n }, hi: vec![None], title: o.title, ..Default::default() };
                }
            } else if key(Key::Enter) || key(Key::Space) {
                match hi.and_then(|i| level.get(i)) {
                    Some(Row::Item { id, enabled: true, .. }) => run = Some(id.clone()),
                    Some(Row::Sub { name, enabled: true, .. }) => {
                        o.path.push(name.clone());
                        o.hi.push(None);
                    }
                    _ => {}
                }
            } else if key(Key::Escape) {
                if o.path.pop().is_some() {
                    o.hi.truncate(o.path.len() + 1);
                } else {
                    closed = true;
                }
            }
        }
        if closed {
            close(ctx);
            open = None;
            levels.clear();
        } else {
            // The keys may have moved to another menu or level: its rows.
            levels = levels_of(&items, o, lang);
        }
    }

    // The windows: the open levels, and at least the first ones hidden for the next menu.
    let screen = ctx.input(|i| i.viewport().monitor_size).map_or(Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1080.0)), |s| Rect::from_min_size(Pos2::ZERO, s));
    let title = open.as_ref().and_then(|o| TOP_MENUS.get(o.top)).map_or_else(String::new, |t| crate::i18n::tr(lang, t).to_string());
    let mut outs: Vec<LevelOut> = Vec::new();
    for depth in 0..levels.len().max(PREWARM) {
        let level = levels.get(depth);
        let known = open.as_ref().filter(|_| level.is_some()).and_then(|o| ctx.data(|d| d.get_temp::<Vec2>(size_key(o, depth))));
        let (visible, window_level) = presentation(known);
        let size = known.unwrap_or(vec2(240.0, 200.0));
        let at = match (open.as_ref(), level) {
            (Some(o), Some(_)) if visible => place(depth, o, size, screen),
            _ => PARK,
        };
        let builder = egui::ViewportBuilder::default()
            .with_title(title.clone())
            .with_decorations(false)
            .with_resizable(false)
            .with_taskbar(false)
            .with_active(false)
            .with_window_level(window_level)
            .with_inner_size(size)
            .with_position(at);
        let hi = open.as_ref().and_then(|o| o.hi.get(depth).copied().flatten());
        let open_sub = open.as_ref().and_then(|o| o.path.get(depth).cloned());
        let measure = open.as_ref().filter(|_| level.is_some()).map(|o| size_key(o, depth));
        let mut out = LevelOut::default();
        ctx.show_viewport_immediate(viewport(depth), builder, |ui, _class| {
            let vctx = ui.ctx().clone();
            let style = vctx.global_style();
            let frame = egui::Frame::popup(&style).corner_radius(0).shadow(egui::Shadow::NONE);
            egui::CentralPanel::default().frame(frame).show(ui, |ui| {
                let Some(level) = level else { return };
                let inner = level_ui(ui, level, hi, open_sub.as_deref(), visible, if depth == 0 { 220.0 } else { 120.0 });
                out = inner.inner;
                let origin = vctx.input(|i| i.viewport().inner_rect).map_or(Vec2::ZERO, |r| r.min.to_vec2());
                for r in &mut out.rects {
                    *r = r.translate(origin);
                }
                if let Some(id) = measure {
                    let want = inner.response.rect.size() + frame.total_margin().sum();
                    let want = vec2(want.x.ceil(), want.y.ceil()).min(screen.size());
                    if known != Some(want) {
                        vctx.data_mut(|d| d.insert_temp(id, want));
                        ctx.request_repaint();
                    }
                }
            });
        });
        if level.is_some() {
            ctx.data_mut(|d| d.insert_temp(rows_id(depth), out.rects.clone()));
            outs.push(out);
        }
    }
    let Some(mut open) = open else { return };

    // Pointer: the deepest level the pointer is on decides.
    for (depth, out) in outs.iter().enumerate() {
        let Some(level) = levels.get(depth) else { continue };
        if let Some(i) = out.clicked {
            match level.get(i) {
                Some(Row::Item { id, enabled: true, .. }) => run = Some(id.clone()),
                Some(Row::Sub { name, enabled: true, .. }) => {
                    open.path.truncate(depth);
                    open.path.push(name.clone());
                    open.anchors.truncate(depth);
                    if let Some(r) = out.rects.get(i) {
                        open.anchors.push(*r);
                    }
                }
                _ => {}
            }
        } else if let Some(i) = out.hovered {
            open.hi.truncate(depth + 1);
            if let Some(h) = open.hi.get_mut(depth) {
                *h = Some(i);
            }
            match level.get(i) {
                Some(Row::Sub { name, enabled: true, .. }) => {
                    if open.path.get(depth) != Some(name) {
                        open.path.truncate(depth);
                        open.path.push(name.clone());
                    }
                    open.anchors.truncate(depth);
                    if let Some(r) = out.rects.get(i) {
                        open.anchors.push(*r);
                    }
                }
                _ => {
                    open.path.truncate(depth);
                    open.anchors.truncate(depth);
                }
            }
        }
    }
    // A submenu opened from the keyboard hangs from its highlighted row.
    for depth in 0..open.path.len() {
        if open.anchors.len() <= depth {
            let rect =
                ctx.data(|d| d.get_temp::<Vec<Rect>>(rows_id(depth))).and_then(|rs| open.hi.get(depth).copied().flatten().and_then(|i| rs.get(i).copied()));
            open.anchors.push(rect.unwrap_or(open.title));
        }
    }
    open.hi.resize(open.path.len() + 1, None);

    if let Some(id) = run {
        close(ctx);
        let id = crate::menus::alt_click(id, ctx.input(|i| i.modifiers.alt));
        if let Err(e) = crate::menus::invoke(app, ctx, &id, json!({})) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        return;
    }
    if Some(&open) != current(ctx).as_ref() {
        set(ctx, Some(open));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, path: &[&str], label: &str) -> MenuItem {
        MenuItem {
            id: id.into(),
            label: label.into(),
            path: path.iter().map(|s| s.to_string()).collect(),
            shortcut: None,
            enabled: true,
            checked: None,
            color: None,
        }
    }

    fn sample() -> Vec<MenuItem> {
        vec![
            item("a.one", &["Image"], "One"),
            item("sep1", &["Image"], "---"),
            item("a.two", &["Image", "Mode"], "Two"),
            item("a.three", &["Image", "Mode"], "Three"),
            item("a.deep", &["Image", "Mode", "Bits"], "Deep"),
            item("sep2", &["Image"], "---"),
            item("sep3", &["Image"], "---"),
            item("a.four", &["Image"], "Four"),
            item("sep4", &["Image"], "---"),
            item("b.other", &["Layer"], "Other"),
        ]
    }

    #[test]
    fn rows_follow_the_in_window_menus() {
        let lang = crate::i18n::Lang::from_pref("en");
        let top = rows(&sample(), "Image", &[], lang);
        let kinds: Vec<String> = top
            .iter()
            .map(|r| match r {
                Row::Item { id, .. } => id.clone(),
                Row::Sub { name, .. } => format!("> {name}"),
                Row::Separator => "---".into(),
            })
            .collect();
        assert_eq!(kinds, ["a.one", "---", "> Mode", "---", "a.four"], "a submenu where its first command is; separators collapse and never end a menu");
        let mode = rows(&sample(), "Image", &["Mode".to_string()], lang);
        assert_eq!(mode.len(), 3, "Two, Three and the Bits submenu");
        assert!(matches!(&mode[2], Row::Sub { name, .. } if name == "Bits"));
        assert!(rows(&sample(), "Image", &["Nope".to_string()], lang).is_empty());
        assert!(rows(&sample(), "Help", &[], lang).is_empty());
    }

    #[test]
    fn arrows_skip_separators_and_disabled_rows_and_wrap() {
        let lang = crate::i18n::Lang::from_pref("en");
        let mut items = sample();
        items[0].enabled = false;
        let top = rows(&items, "Image", &[], lang);
        assert_eq!(step(&top, None, true), Some(2), "down from nothing: the first enabled row (One is disabled)");
        assert_eq!(step(&top, Some(2), true), Some(4));
        assert_eq!(step(&top, Some(4), true), Some(2), "wraps");
        assert_eq!(step(&top, None, false), Some(4), "up from nothing: the last");
        assert_eq!(step(&[], None, true), None);
        assert_eq!(step(&[Row::Separator], None, true), None);
    }

    #[test]
    fn levels_stay_on_the_monitor() {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0));
        let open = Open {
            top: 0,
            path: vec!["Mode".into()],
            hi: vec![None, None],
            title: Rect::from_min_size(pos2(100.0, 0.0), vec2(40.0, 24.0)),
            anchors: vec![Rect::from_min_size(pos2(100.0, 200.0), vec2(240.0, 22.0))],
        };
        assert_eq!(place(0, &open, vec2(240.0, 300.0), screen), pos2(100.0, 24.0), "below the title");
        assert_eq!(place(1, &open, vec2(200.0, 100.0), screen), pos2(342.0, 196.0), "right of its row");
        let far = Open { anchors: vec![Rect::from_min_size(pos2(700.0, 200.0), vec2(240.0, 22.0))], ..open.clone() };
        assert_eq!(place(1, &far, vec2(200.0, 100.0), screen).x, 498.0, "no room on the right: on the left");
        assert_eq!(place(0, &open, vec2(240.0, 900.0), screen).y, 0.0, "taller than the monitor: from its top");
        assert_eq!(place(1, &open, vec2(200.0, 700.0), screen).y, 100.0, "kept above the bottom");
    }

    #[test]
    fn menus_get_windows_only_beside_floating_documents() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        app.sync_views();
        ctx.set_embed_viewports(false);
        assert!(!hosted(&app, &ctx));
        crate::menus::invoke(&mut app, &ctx, "window.arrange.newWindowForDocument", json!({})).unwrap();
        assert!(!hosted(&app, &ctx), "a second view isn't floating");
        crate::menus::invoke(&mut app, &ctx, "window.arrange.floatInWindow", json!({})).unwrap();
        assert!(hosted(&app, &ctx));
        ctx.set_embed_viewports(true);
        assert!(!hosted(&app, &ctx), "embedded: the main window's popups");
    }

    /// A level's window is hidden at the normal level until measured, then shown on top; and the
    /// bar's open-menu mark (which keeps shortcuts off) never raises the main window, which would
    /// cover the document windows (the reported bug).
    #[test]
    fn menu_windows_show_on_top_once_measured_and_never_raise_the_main_window() {
        assert_eq!(presentation(None), (false, egui::WindowLevel::Normal));
        assert_eq!(presentation(Some(vec2(240.0, 300.0))), (true, egui::WindowLevel::AlwaysOnTop));
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        app.sync_views();
        crate::menus::invoke(&mut app, &ctx, "window.arrange.floatInWindow", json!({})).unwrap();
        set(&ctx, Some(Open { top: 0, title: Rect::from_min_size(pos2(10.0, 0.0), vec2(40.0, 24.0)), ..Default::default() }));
        let wins = app.ui.windows.clone();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            crate::menu_nav::mark_open(ui.ctx());
            crate::canvas::raise_main_window_for_menus(ui.ctx(), &wins);
        });
        out.textures_delta.clear();
        let raised = out
            .viewport_output
            .get(&ViewportId::ROOT)
            .is_some_and(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop))));
        assert!(!raised, "the main window stays under the document windows");
        // Without the menu's windows (the bar's own popup menu) it is raised, as before.
        close(&ctx);
        let mut out = ctx.run_ui(Default::default(), |ui| {
            crate::menu_nav::mark_open(ui.ctx());
            crate::canvas::raise_main_window_for_menus(ui.ctx(), &wins);
        });
        out.textures_delta.clear();
        let raised = out
            .viewport_output
            .get(&ViewportId::ROOT)
            .is_some_and(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop))));
        assert!(raised);
    }

    /// A level's rows start at the top of its window and fill exactly the size measured for it
    /// (the measuring took the rows' height once, pushing them out of the window).
    #[test]
    fn a_levels_rows_start_at_the_top_of_its_window() {
        let lang = crate::i18n::Lang::from_pref("en");
        let level = rows(&sample(), "Image", &[], lang);
        let ctx = egui::Context::default();
        let mut got = None;
        let mut out = ctx.run_ui(Default::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let top = ui.cursor().top();
                let inner = level_ui(ui, &level, None, None, true, 220.0);
                got = Some((top, inner.response.rect, inner.inner.rects));
            });
        });
        out.textures_delta.clear();
        let (top, rect, rects) = got.unwrap();
        assert_eq!(rect.top(), top, "no room taken above the rows");
        assert_eq!(rects.len(), level.len());
        assert!((rects[0].top() - top).abs() < 0.5, "the first row is at the top");
        let last = rects.last().unwrap();
        assert!(last.bottom() <= rect.bottom() + 0.5, "every row is inside the measured size");
        assert!(rect.width() >= 220.0);
    }

    /// Keys drive the open menu: ↓ highlights, → opens the submenu, ↩ runs, Esc closes.
    #[test]
    fn keys_drive_the_open_menu() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        app.sync_views();
        crate::menus::invoke(&mut app, &ctx, "window.arrange.floatInWindow", json!({})).unwrap();
        let view = TOP_MENUS.iter().position(|t| *t == "View").unwrap();
        let press = |app: &mut PhotocraftApp, k: Key| {
            let raw = egui::RawInput {
                events: vec![egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| show(app, ui.ctx()));
            out.textures_delta.clear();
        };
        set(&ctx, Some(Open { top: view, hi: vec![None], ..Default::default() }));
        press(&mut app, Key::ArrowDown);
        let o = current(&ctx).unwrap();
        assert!(o.hi[0].is_some(), "↓ highlights a row");
        // Walk down to the first submenu and open it.
        let lang = crate::i18n::current();
        let level = rows(&crate::menus::menu_items(&app), "View", &[], lang);
        let sub = level.iter().position(|r| matches!(r, Row::Sub { enabled: true, .. })).unwrap();
        set(&ctx, Some(Open { hi: vec![Some(sub)], ..o }));
        press(&mut app, Key::ArrowRight);
        let o = current(&ctx).unwrap();
        assert_eq!(o.path.len(), 1, "→ opens the submenu");
        press(&mut app, Key::ArrowLeft);
        assert!(current(&ctx).unwrap().path.is_empty(), "← closes it");
        press(&mut app, Key::Escape);
        assert!(current(&ctx).is_none(), "Esc closes the menu");
    }
}
