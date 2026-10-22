//! Window › Gradients, Patterns (tabs of the Color card, as in Photoshop's Essentials workspace),
//! Styles, Shapes, Tool Presets and Clone Source (floating panels).
//!
//! The preset panels share one browser: folders with disclosure triangles, a thumbnail grid (or a
//! list when the size slider is at its minimum), a rename field, a context menu (Rename, Delete,
//! Move to group) and the Photoshop footer (size slider · New Group · New · Delete). Clicking a
//! preset selects it, double-clicking or dragging it onto the canvas applies it. Every action is
//! an engine command (`gradient.presets.*`, `pattern.presets.*`, `style.presets.*`,
//! `shape.presets.*`, `tool.presets.*`, `cloneSource.*`); this module only draws and dispatches.

use std::collections::{BTreeMap, BTreeSet};

use egui::{Align2, Color32, ColorImage, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;
use crate::theme::Tokens;

/// Panel state (serde, so the control channel can read and drive it).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetUi {
    /// Floating panels.
    pub styles: bool,
    pub shapes: bool,
    pub tool_presets: bool,
    pub clone_source: bool,
    /// Thumbnail size per panel (px; the minimum shows a list).
    pub sizes: BTreeMap<String, f32>,
    /// Folders whose open state differs from the default (only the first folder starts open).
    pub toggled: BTreeSet<String>,
    /// Selected preset per panel (by name / pattern id).
    pub selected: BTreeMap<String, String>,
    /// Tool Presets: Current Tool Only.
    pub current_tool_only: bool,
    /// Custom Shape tool: the shape it draws.
    pub custom_shape: String,
    /// Folder/group order per panel, by name (#3658), like the dock's `tabOrder` (#2272). A panel
    /// with no entry keeps the library's order; unknown names are ignored and groups missing from
    /// a list follow it in library order. Saved with the UI state.
    #[serde(rename = "groupOrder", alias = "group_order", deserialize_with = "lenient_group_order", skip_serializing_if = "BTreeMap::is_empty")]
    pub group_order: BTreeMap<String, Vec<String>>,
    /// In-place rename: (panel, preset or "group:<name>", text).
    #[serde(skip)]
    pub renaming: Option<(String, String, String)>,
}

/// Read `groupOrder` keeping what makes sense: non-object values, non-list entries and non-string
/// names are dropped instead of failing the whole saved UI state.
const MAX_GROUP_ORDER: usize = 10_000;
fn lenient_group_order<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BTreeMap<String, Vec<String>>, D::Error> {
    let v = Value::deserialize(d)?;
    let mut out = BTreeMap::new();
    for (panel, names) in v.as_object().into_iter().flatten() {
        let Some(names) = names.as_array() else { continue };
        out.insert(panel.clone(), names.iter().filter_map(Value::as_str).take(MAX_GROUP_ORDER).map(str::to_owned).collect());
    }
    Ok(out)
}

impl PresetUi {
    pub fn shape(&self) -> &str {
        if self.custom_shape.is_empty() { tl!("Heart") } else { &self.custom_shape }
    }

    /// Display order of the folders for `panel`, as library indices: the stored order first
    /// (names that no longer exist are dropped), then groups missing from it in library order.
    /// Mirrors the dock's per-group tab order (#2272, #3658).
    pub(crate) fn display_order(&self, panel: &str, names: &[String]) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::with_capacity(names.len());
        for name in self.group_order.get(panel).into_iter().flatten() {
            if let Some(i) = names.iter().position(|n| n == name)
                && !out.contains(&i)
            {
                out.push(i);
            }
        }
        for i in 0..names.len() {
            if !out.contains(&i) {
                out.push(i);
            }
        }
        out
    }

    /// Move folder `from` so it is drawn before `before` (`None` = last) for `panel`. The
    /// library's own order is stored as no entry, as the dock does for its tabs.
    pub(crate) fn move_group(&mut self, panel: &str, names: &[String], from: &str, before: Option<&str>) {
        if before == Some(from) {
            return;
        }
        let mut order: Vec<String> = self.display_order(panel, names).into_iter().filter_map(|i| names.get(i).cloned()).collect();
        let Some(at) = order.iter().position(|n| n == from) else { return };
        let moved = order.remove(at);
        let to = before.and_then(|b| order.iter().position(|n| n == b)).unwrap_or(order.len());
        order.insert(to, moved);
        if order.as_slice() == names {
            self.group_order.remove(panel);
        } else {
            self.group_order.insert(panel.to_string(), order);
        }
    }
}

const PANELS: [&str; 6] =
    ["window.panel.gradients", "window.panel.patterns", "window.panel.styles", "window.panel.shapes", "window.panel.toolPresets", "window.panel.cloneSource"];
const MIN_THUMB: f32 = 20.0;

/// Window-menu ids handled here.
pub fn handles(id: &str) -> bool {
    PANELS.contains(&id)
}

/// Color card tab of a Window › Gradients / Patterns id.
fn color_tab(id: &str) -> Option<usize> {
    match id {
        "window.panel.gradients" => Some(2),
        "window.panel.patterns" => Some(3),
        _ => None,
    }
}

fn float_flag<'a>(p: &'a mut PresetUi, id: &str) -> Option<&'a mut bool> {
    Some(match id {
        "window.panel.styles" => &mut p.styles,
        "window.panel.shapes" => &mut p.shapes,
        "window.panel.toolPresets" => &mut p.tool_presets,
        "window.panel.cloneSource" => &mut p.clone_source,
        _ => return None,
    })
}

/// Window › <panel>: toggles the panel (Gradients and Patterns: the Color card tab).
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if !handles(id) {
        return None;
    }
    let want = params.get("show").and_then(Value::as_bool);
    if let Some(tab) = color_tab(id) {
        // A collapsed Color group counts as not showing: the item expands it (#129).
        let showing = app.ui.panels.color && app.ui.dock_tabs.color == tab && !app.ui.dock.is_collapsed(crate::dock::Group::Color);
        let show = want.unwrap_or(!showing);
        app.ui.panels.color = show || (app.ui.panels.color && app.ui.dock_tabs.color != tab);
        if show {
            app.ui.dock_tabs.color = tab;
            crate::dock::reveal(app, crate::dock::Group::Color);
        }
        return Some(Ok(json!({"visible": show})));
    }
    let flag = float_flag(&mut app.ui.presets_ui, id)?;
    *flag = want.unwrap_or(!*flag);
    Some(Ok(json!({"visible": *flag})))
}

pub fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    if let Some(tab) = color_tab(id) {
        return Some(app.ui.panels.color && app.ui.dock_tabs.color == tab);
    }
    let mut p = app.ui.presets_ui.clone();
    float_flag(&mut p, id).map(|f| *f)
}

pub(crate) fn run(app: &mut PhotocraftApp, id: &str, p: Value) -> Option<Value> {
    match app.run(id, p) {
        Ok(v) => Some(v),
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
            None
        }
    }
}

// ------------------------------------------------------------------ textures

fn cached_texture(ctx: &egui::Context, key: impl std::hash::Hash + std::fmt::Debug, make: impl FnOnce() -> ColorImage) -> TextureHandle {
    let id = egui::Id::new(("preset-tex", key));
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return t;
    }
    let tex = ctx.load_texture(format!("preset-{id:?}"), make(), TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

fn c32(c: [f32; 4]) -> Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(q(c[0]), q(c[1]), q(c[2]), q(c[3]))
}

/// A gradient swatch: stops as a horizontal mesh over a checkerboard where it's transparent.
pub fn paint_gradient(ui: &egui::Ui, rect: Rect, stops: &[(f32, [f32; 4])]) {
    let p = ui.painter_at(rect);
    if stops.iter().any(|s| s.1[3] < 0.999) {
        crate::widgets::checker(&p, rect, (rect.height() / 4.0).clamp(3.0, 8.0));
    }
    let mut mesh = egui::Mesh::default();
    let x = |t: f32| rect.left() + rect.width() * t.clamp(0.0, 1.0);
    let mut pts: Vec<(f32, [f32; 4])> = stops.to_vec();
    if pts.first().is_some_and(|s| s.0 > 0.0) {
        pts.insert(0, (0.0, pts[0].1));
    }
    if let Some(l) = pts.last().copied().filter(|s| s.0 < 1.0) {
        pts.push((1.0, l.1));
    }
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let i = mesh.vertices.len() as u32;
        let (ca, cb) = (c32(a.1), c32(b.1));
        mesh.colored_vertex(pos2(x(a.0), rect.top()), ca);
        mesh.colored_vertex(pos2(x(b.0), rect.top()), cb);
        mesh.colored_vertex(pos2(x(b.0), rect.bottom()), cb);
        mesh.colored_vertex(pos2(x(a.0), rect.bottom()), ca);
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
    }
    p.add(mesh);
}

fn pattern_texture(ctx: &egui::Context, pat: &photocraft_doc::Pattern) -> TextureHandle {
    cached_texture(ctx, ("pattern", pat.id.clone()), || {
        let (w, h) = (pat.width.max(1), pat.height.max(1));
        let fmt = pat.surface.format();
        let px = pat.surface.read_region(photocraft_geom::Rect::new(0, 0, w as i32, h as i32));
        let n = fmt.channels();
        // Tiles smaller than the swatch repeat; larger ones are shown shrunk.
        const S: u32 = 64;
        let step = (w.max(h) as f32 / S as f32).max(1.0);
        let mut img = ColorImage::new([S as usize, S as usize], vec![Color32::TRANSPARENT; (S * S) as usize]);
        for y in 0..S {
            for x in 0..S {
                let sx = ((x as f32 * step) as u32) % w;
                let sy = ((y as f32 * step) as u32) % h;
                let i = (sy * w + sx) as usize * n;
                img.pixels[(y * S + x) as usize] = c32(photocraft_raster::to_rgba(&fmt, &px[i..i + n]));
            }
        }
        img
    })
}

/// Photoshop's pattern picker: a swatch of the library pattern `selected` (an id) that opens a grid
/// of every library pattern's swatch, each named in its tooltip. Returns the id of a newly picked
/// pattern.
pub(crate) fn pattern_picker(app: &PhotocraftApp, ui: &mut egui::Ui, selected: &str) -> Option<String> {
    const SWATCH: f32 = 40.0;
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let pats = &app.session.patterns.items;
    let current = pats.iter().find(|p| p.id == selected);
    let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
    let (r, resp) = ui.allocate_exact_size(vec2(SWATCH, SWATCH), Sense::click());
    if let Some(p) = current {
        ui.painter().image(pattern_texture(&ctx, p).id(), r, uv, Color32::WHITE);
    }
    ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Outside);
    let name = current.map_or(String::new(), |p| p.display_name().to_string());
    let resp = resp.on_hover_text(&name);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Patterns")));
    let groups = pattern_group_views(pats, &app.session.presets.pattern_groups);
    let picked = egui::Popup::menu(&resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            // Preserve the former six-swatch-wide popup while sharing the grouped browser.
            ui.set_width(6.0 * (SWATCH + 4.0));
            compact_pattern_browser(ui, pats, &groups, selected, "compact-pattern-picker", 240.0)
        })
        .and_then(|r| r.inner);
    picked.filter(|id| id != selected)
}

/// The Patterns library rendered by the shared browser without the standard panel shell. Used by
/// compact pickers (Brush Settings and fill dialogs) that need selection but no footer or menus.
pub(crate) fn compact_pattern_browser(
    ui: &mut egui::Ui,
    pats: &[photocraft_doc::Pattern],
    groups: &[GroupView],
    selected: &str,
    panel: &'static str,
    max_h: f32,
) -> Option<String> {
    let ctx = ui.ctx().clone();
    let t = Tokens::get(&ctx);
    let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
    // Popup contents are rebuilt every frame. Keep the browser's expansion state in egui's
    // temporary data store, scoped to this popup instance and consumer.
    let state_id = egui::Id::new(("compact-pattern-browser", panel));
    let mut state = ui.data(|d| d.get_temp::<PresetUi>(state_id)).unwrap_or_else(|| {
        let mut state = PresetUi::default();
        // Compact pickers replace the old ungrouped swatch grid. Keep every group's items
        // immediately reachable on first open, while allowing users to collapse groups afterward.
        state.toggled.extend(groups.iter().skip(1).map(|g| format!("{panel}/{}", g.name)));
        state
    });
    state.selected.insert(panel.into(), selected.to_string());
    state.sizes.insert(panel.into(), 40.0);
    let mut adapter = BrowserAdapter {
        panel,
        groups,
        options: BrowserOptions { canvas: Rect::NOTHING, max_h, flush: false },
        capabilities: BrowserCapabilities { item_drag: false, group_drag: false, selection: true, activation: true },
        item_context_menu: None,
        group_context_menu: None,
        paint_group: None,
        paint_item: &mut |ui, r, gi, ii| {
            if let Some(p) = groups.get(gi).and_then(|g| g.items.get(ii)).and_then(|i| pats.iter().find(|p| p.id == i.key)) {
                ui.painter().image(pattern_texture(&ctx, p).id(), r, uv, Color32::WHITE);
                let on = p.id == selected;
                ui.painter().rect_stroke(
                    r,
                    0.0,
                    Stroke::new(if on { 2.0 } else { 1.0 }, if on { t.accent } else { t.field_border }),
                    egui::StrokeKind::Outside,
                );
            }
        },
    };
    let mut picked = None;
    for event in browser(ui, &mut state, &mut adapter, max_h) {
        if let Ev::Select(id) | Ev::Activate(id) = event {
            if id != selected {
                picked = Some(id);
            }
            ui.close();
        }
    }
    ui.data_mut(|d| d.insert_temp(state_id, state));
    picked
}

pub(crate) fn style_texture(app: &PhotocraftApp, ctx: &egui::Context, st: &photocraft_engine::presets::styles::StylePreset) -> TextureHandle {
    let key = ("style", st.name.clone(), st.effects.len(), format!("{:?}{:?}", st.blend, st.fill_opacity));
    cached_texture(ctx, key, || {
        const S: u32 = 64;
        let px = photocraft_engine::presets::styles::thumbnail(&app.session, st, S);
        ColorImage::from_rgba_unmultiplied([S as usize, S as usize], &px)
    })
}

fn shape_texture(ctx: &egui::Context, sh: &photocraft_engine::presets::shapes::ShapePreset) -> TextureHandle {
    let knots: usize = sh.path.subpaths.iter().map(|s| s.knots.len()).sum();
    cached_texture(ctx, ("shape", sh.name.clone(), knots), || {
        const S: u32 = 64;
        let cov = photocraft_engine::presets::shapes::thumbnail(&sh.path, S);
        ColorImage::new([S as usize, S as usize], cov.iter().map(|a| Color32::from_white_alpha((a.clamp(0.0, 1.0) * 255.0).round() as u8)).collect())
    })
}

// ------------------------------------------------------------------ the browser

/// Asset-neutral item identity and label consumed by the shared picker browser.
pub(crate) struct ItemView {
    /// Stable consumer-defined key used for selection and emitted events.
    pub(crate) key: String,
    /// Display label shown in the item card or list row.
    pub(crate) name: String,
}

/// Asset-neutral group and its ordered items, ready for rendering by the shared browser.
pub(crate) struct GroupView {
    /// Group label and identity used for collapse state and group actions.
    pub(crate) name: String,
    /// Items in the order they should appear within this group.
    pub(crate) items: Vec<ItemView>,
}

/// Build browser groups from the current library, including patterns not yet reconciled into the
/// saved group projection. Newly defined/imported patterns and reconstructed library ids must be
/// visible in compact pickers even if the main Patterns panel has not rendered first.
pub(crate) fn pattern_group_views(patterns: &[photocraft_doc::Pattern], groups: &[photocraft_engine::presets::Group<String>]) -> Vec<GroupView> {
    let mut assigned = BTreeSet::new();
    let mut views: Vec<GroupView> = groups
        .iter()
        .map(|g| GroupView {
            name: g.name.clone(),
            items: g
                .items
                .iter()
                .filter_map(|id| patterns.iter().find(|p| &p.id == id))
                .filter(|p| assigned.insert(p.id.clone()))
                .map(|p| ItemView { key: p.id.clone(), name: p.display_name().to_string() })
                .collect(),
        })
        .collect();
    let loose: Vec<ItemView> =
        patterns.iter().filter(|p| !assigned.contains(&p.id)).map(|p| ItemView { key: p.id.clone(), name: p.display_name().to_string() }).collect();
    if !loose.is_empty() {
        if let Some(group) = views.iter_mut().find(|g| g.name == "Patterns") {
            group.items.extend(loose);
        } else {
            views.push(GroupView { name: "Patterns".into(), items: loose });
        }
    }
    views
}

/// Reusable hierarchy projection for consumers whose items carry nested folder paths. The tree
/// preserves source order: a folder appears where its first descendant occurs and direct items keep
/// their positions relative to folders. Consumers retain their own row/card rendering and actions.
#[derive(Debug)]
pub enum HierarchyNode<'a, T> {
    /// An item that belongs directly at this level of the hierarchy.
    Preset(&'a T),
    /// A nested folder, kept in the position of its first descendant.
    Folder(HierarchyFolder<'a, T>),
}

/// A folder node in the shared, order-preserving hierarchy projection.
#[derive(Debug)]
pub struct HierarchyFolder<'a, T> {
    /// Full path from the item's group root to this folder.
    pub path: Vec<String>,
    /// Items and subfolders in their original library order.
    pub children: Vec<HierarchyNode<'a, T>>,
    /// Number of items in this folder subtree, including nested folders.
    pub count: usize,
}

/// Project ordered items and their folder paths into a hierarchy, truncating paths at `max_depth`.
///
/// Paths deeper than `max_depth` are truncated before grouping, so the view cannot recurse beyond
/// the consumer's supported nesting limit. Item references remain borrowed from the source library.
pub(crate) fn hierarchy_tree<'a, T>(items: Vec<(&'a [String], &'a T)>, max_depth: usize) -> Vec<HierarchyNode<'a, T>> {
    let bounded = items.into_iter().map(|(path, item)| (&path[..path.len().min(max_depth)], item)).collect();
    hierarchy_level(bounded, &[])
}

/// Shared ordered traversal for nested hierarchy views. Consumers provide their item-run renderer,
/// folder presentation/interaction, and collapse policy; the traversal itself owns the recursive
/// folder/item sequencing so specialised views do not grow independent hierarchy walkers.
///
/// Adjacent presets are batched into one `draw_items` call and flushed before each folder header;
/// children are traversed only when `is_open` returns true and `max_depth` has not been reached.
type HierarchyOpen<'a, 'b, T> = dyn FnMut(&HierarchyFolder<'a, T>) -> bool + 'b;
type HierarchyDrawFolder<'a, 'b, T> = dyn FnMut(&mut egui::Ui, &HierarchyFolder<'a, T>, bool, usize) + 'b;
type HierarchyDrawItems<'a, 'b, T> = dyn FnMut(&mut egui::Ui, &[&'a T], usize) + 'b;

pub(crate) fn draw_hierarchy<'a, 'b, T>(
    ui: &mut egui::Ui,
    nodes: &[HierarchyNode<'a, T>],
    depth: usize,
    max_depth: usize,
    is_open: &mut HierarchyOpen<'a, 'b, T>,
    draw_folder: &mut HierarchyDrawFolder<'a, 'b, T>,
    draw_items: &mut HierarchyDrawItems<'a, 'b, T>,
) {
    let mut run: Vec<&'a T> = Vec::new();
    let flush = |ui: &mut egui::Ui, run: &mut Vec<&'a T>, draw_items: &mut dyn FnMut(&mut egui::Ui, &[&'a T], usize)| {
        if !run.is_empty() {
            draw_items(ui, run, depth);
            run.clear();
        }
    };
    for node in nodes {
        match node {
            HierarchyNode::Preset(item) => run.push(*item),
            HierarchyNode::Folder(folder) => {
                flush(ui, &mut run, draw_items);
                let open = is_open(folder);
                draw_folder(ui, folder, open, depth);
                if open && depth < max_depth {
                    draw_hierarchy(ui, &folder.children, depth + 1, max_depth, is_open, draw_folder, draw_items);
                }
            }
        }
    }
    flush(ui, &mut run, draw_items);
}

type HierarchyEntry<'a, T> = (&'a [String], &'a T);

/// Build one level of the tree while retaining the first-seen order of items and folders.
fn hierarchy_level<'a, T>(items: Vec<HierarchyEntry<'a, T>>, prefix: &[String]) -> Vec<HierarchyNode<'a, T>> {
    enum Slot {
        Item(usize),
        Folder(usize),
    }
    let mut slots = Vec::new();
    let mut folders: Vec<(&String, Vec<HierarchyEntry<'a, T>>)> = Vec::new();
    for (i, (path, item)) in items.iter().enumerate() {
        match path.split_first() {
            None => slots.push(Slot::Item(i)),
            Some((name, rest)) => match folders.iter_mut().position(|(n, _)| *n == name) {
                Some(k) => {
                    if let Some((_, v)) = folders.get_mut(k) {
                        v.push((rest, *item));
                    }
                }
                None => {
                    slots.push(Slot::Folder(folders.len()));
                    folders.push((name, vec![(rest, *item)]));
                }
            },
        }
    }
    type NamedFolderEntries<'a, T> = (&'a String, Vec<HierarchyEntry<'a, T>>);
    let mut folders: Vec<Option<NamedFolderEntries<'a, T>>> = folders.into_iter().map(Some).collect();
    slots
        .into_iter()
        .filter_map(|slot| match slot {
            Slot::Item(i) => items.get(i).map(|(_, item)| HierarchyNode::Preset(*item)),
            Slot::Folder(i) => {
                let (name, sub) = folders.get_mut(i)?.take()?;
                let path: Vec<String> = prefix.iter().cloned().chain(std::iter::once(name.clone())).collect();
                let count = sub.len();
                let children = hierarchy_level(sub, &path);
                Some(HierarchyNode::Folder(HierarchyFolder { path, children, count }))
            }
        })
        .collect()
}

/// Generic browser interactions; each asset consumer translates these into its own commands.
pub(crate) enum Ev {
    /// Select an item without activating or dismissing its picker.
    Select(String),
    /// Activate an item, typically on a double-click.
    Activate(String),
    /// Drop an item onto a canvas position in screen coordinates.
    Drop(String, Pos2),
    /// Rename an item from its stable key to a new display name.
    Rename(String, String),
    /// Delete the item identified by its stable key.
    Delete(String),
    /// Move an item into the named group.
    Move(String, String),
    /// Rename a group from its current name to a new name.
    RenameGroup(String, String),
    /// Delete a group and its contents.
    DeleteGroup(String),
    /// Request creation of a new group.
    NewGroup,
    /// Request creation of a new item.
    New,
}

/// Initial expansion policy for a browser group, with explicit user toggles overriding the default.
fn group_open(st: &PresetUi, panel: &str, gi: usize, name: &str) -> bool {
    // Swatch groups are small and all start open (Photoshop); other panels open the first only.
    (gi == 0 || panel == crate::swatches_ui::PANEL) != st.toggled.contains(&format!("{panel}/{name}"))
}

/// The footer's height (gap, size slider and buttons) before it has been measured.
const FOOTER_GUESS: f32 = 32.0;

/// Browser-specific layout and presentation, supplied by each asset consumer.
/// `flush` lays the grid out as Photoshop's Swatches panel: the cells touch, square-cornered and
/// without a border of their own, so no screen is spent on gaps (#5103); gradients, patterns and
/// the other preset grids keep their [`GRID_GAP`] and card borders.
#[derive(Clone, Copy)]
pub(crate) struct BrowserOptions {
    /// Canvas hit region used to distinguish drops onto the canvas from drops into the browser.
    pub(crate) canvas: Rect,
    /// Consumer-specific cap; the shell reduces this after reserving its optional controls.
    pub(crate) max_h: f32,
    /// Use touching, square-cornered grid cells rather than spaced preset cards.
    pub(crate) flush: bool,
}

/// Features that can be enabled independently for a particular browser consumer.
#[derive(Clone, Copy)]
pub(crate) struct BrowserCapabilities {
    /// Whether presets can be dragged to the canvas or other browser targets.
    pub(crate) item_drag: bool,
    /// Whether group headers can be reordered by dragging.
    pub(crate) group_drag: bool,
    /// Whether clicking an item emits a selection event.
    pub(crate) selection: bool,
    /// Whether activating an item (for example, double-clicking) emits an activation event.
    pub(crate) activation: bool,
}

impl Default for BrowserCapabilities {
    fn default() -> Self {
        Self { item_drag: true, group_drag: true, selection: true, activation: true }
    }
}

/// Optional consumer-provided painter for a group row: UI, row rectangle, group, open, hovered.
pub(crate) type PaintGroup = dyn FnMut(&egui::Ui, Rect, &GroupView, bool, bool);

/// Consumer adapter for the generic browser. Each asset library projects its own identity and
/// labels into `GroupView`/`ItemView`, paints its own preview, and handles returned `Ev` values with
/// its own commands. The browser never knows about gradient, pattern, swatch, style, or brush data.
pub(crate) struct BrowserAdapter<'a> {
    /// Stable key for this consumer's selection, sizing, and folder-collapse state.
    pub(crate) panel: &'a str,
    /// Consumer-projected groups and ordered items to display.
    pub(crate) groups: &'a [GroupView],
    /// Grid geometry and consumer-specific layout options.
    pub(crate) options: BrowserOptions,
    /// Toggleable interactions supported by this consumer.
    pub(crate) capabilities: BrowserCapabilities,
    /// Consumers may replace the standard item menu or omit it entirely. The browser only owns
    /// the hit target; menu contents and emitted generic requests live behind this adapter seam.
    pub(crate) item_context_menu: Option<ItemContextMenu>,
    /// Consumer-specific group menu policy, attached to the shared group hit target.
    pub(crate) group_context_menu: Option<GroupContextMenu>,
    /// Optional override for group-row visuals; the default is a folder row with disclosure.
    pub(crate) paint_group: Option<&'a mut PaintGroup>,
    /// Consumer-specific item preview painter, given the UI, target rectangle, group index and item index.
    pub(crate) paint_item: &'a mut dyn FnMut(&egui::Ui, Rect, usize, usize),
}

/// Consumer callback that appends generic events for actions offered on an item.
pub(crate) type ItemContextMenu = fn(&mut egui::Ui, &mut PresetUi, &str, &ItemView, &GroupView, &[String], &mut Vec<Ev>);
/// Consumer callback that appends generic events for actions offered on a group header.
pub(crate) type GroupContextMenu = fn(&mut egui::Ui, &mut PresetUi, &str, &GroupView, &mut Vec<Ev>);

/// Actions shared by picker item and group context menus. Consumers translate these into their
/// own state updates or engine commands, while the menu labels and interaction stay consistent.
pub(crate) enum ContextMenuAction {
    /// Begin renaming the targeted item or group.
    Rename,
    /// Delete the targeted item or group.
    Delete,
    /// Move the targeted item into the named group.
    MoveTo(String),
}

/// Draw a configurable item context menu and return the chosen generic action, if any.
///
/// `move_to` supplies the submenu label, available group names, and current group; the current
/// group is omitted from the choices. Returns a generic action without mutating an asset library.
pub(crate) fn draw_item_context_menu(
    ui: &mut egui::Ui,
    rename_label: &str,
    delete_label: &str,
    move_to: Option<(&str, &[String], &str)>,
) -> Option<ContextMenuAction> {
    if ui.button(rename_label).clicked() {
        ui.close();
        return Some(ContextMenuAction::Rename);
    }
    if ui.button(delete_label).clicked() {
        ui.close();
        return Some(ContextMenuAction::Delete);
    }
    if let Some((label, group_names, current_group)) = move_to {
        let mut action = None;
        ui.menu_button(label, |ui| {
            for group in group_names.iter().filter(|name| name.as_str() != current_group) {
                if ui.button(group).clicked() {
                    action = Some(ContextMenuAction::MoveTo(group.clone()));
                    ui.close();
                }
            }
        });
        return action;
    }
    None
}

/// Draw a group's rename/delete context menu and return the chosen action, if any.
pub(crate) fn draw_group_context_menu(ui: &mut egui::Ui, rename_label: &str, delete_label: &str) -> Option<ContextMenuAction> {
    if ui.button(rename_label).clicked() {
        ui.close();
        Some(ContextMenuAction::Rename)
    } else if ui.button(delete_label).clicked() {
        ui.close();
        Some(ContextMenuAction::Delete)
    } else {
        None
    }
}

/// Default menu policy for the standard preset panels. Keeping this separate from the browser
/// lets specialised consumers supply their own operations without forking the shared hit targets.
pub(crate) fn standard_item_context_menu(
    ui: &mut egui::Ui,
    st: &mut PresetUi,
    panel: &str,
    item: &ItemView,
    group: &GroupView,
    group_names: &[String],
    ev: &mut Vec<Ev>,
) {
    match draw_item_context_menu(ui, tl!("Rename…"), tl!("Delete"), Some((tl!("Move to"), group_names, &group.name))) {
        Some(ContextMenuAction::Rename) => st.renaming = Some((panel.to_string(), item.key.clone(), item.name.clone())),
        Some(ContextMenuAction::Delete) => ev.push(Ev::Delete(item.key.clone())),
        Some(ContextMenuAction::MoveTo(group)) => ev.push(Ev::Move(item.key.clone(), group)),
        None => {}
    }
}

/// Apply the shared rename/delete policy for a standard browser group context menu.
pub(crate) fn standard_group_context_menu(ui: &mut egui::Ui, st: &mut PresetUi, panel: &str, group: &GroupView, ev: &mut Vec<Ev>) {
    match draw_group_context_menu(ui, tl!("Rename Group…"), tl!("Delete Group")) {
        Some(ContextMenuAction::Rename) => st.renaming = Some((panel.to_string(), format!("group:{}", group.name), group.name.clone())),
        Some(ContextMenuAction::Delete) => ev.push(Ev::DeleteGroup(group.name.clone())),
        _ => {}
    }
}

/// Shared context-menu hit target for specialised consumers that keep their own row/card renderer.
/// The caller provides the menu policy and generic action requests; this helper owns only egui's
/// response wiring, matching the standard browser's item/group targets.
pub(crate) fn context_menu_target(resp: egui::Response, menu: impl FnOnce(&mut egui::Ui)) {
    let _ = resp.context_menu(menu);
}

/// Optional standard-picker footer controls. The shell owns their placement; the browser only
/// emits the corresponding generic events, leaving each consumer to dispatch asset-specific work.
pub(crate) struct FooterOptions<'a> {
    pub(crate) new_tip: &'a str,
    pub(crate) size_slider: bool,
    pub(crate) new_group: bool,
    pub(crate) new_item: bool,
    pub(crate) delete: bool,
}

impl<'a> FooterOptions<'a> {
    /// Enable the standard new-item, new-group, delete, and size controls for a picker footer.
    pub(crate) fn standard(new_tip: &'a str) -> Self {
        Self { new_tip, size_slider: true, new_group: true, new_item: true, delete: true }
    }
}

/// Shell-level controls are optional so compact pickers can share the browser without inheriting
/// the standard footer or panel chrome. Callbacks draw consumer-specific controls; they never
/// dispatch browser item/group actions.
#[derive(Default)]
pub(crate) struct ShellOptions<'a> {
    /// Optional consumer-owned header content above the shared picker controls.
    pub(crate) header: Option<&'a mut dyn FnMut(&mut egui::Ui)>,
    /// Optional consumer-owned menu button or settings controls.
    pub(crate) panel_menu: Option<&'a mut dyn FnMut(&mut egui::Ui)>,
    /// Optional search control; the consumer owns its query state and filtering.
    pub(crate) search: Option<&'a mut dyn FnMut(&mut egui::Ui)>,
    /// Optional recent-items strip rendered above the browser.
    pub(crate) recents: Option<&'a mut dyn FnMut(&mut egui::Ui)>,
    /// Standard shared footer controls, when the picker needs them.
    pub(crate) footer: Option<FooterOptions<'a>>,
    /// Consumer-owned footer renderer plus the height it reserves in the browser viewport.
    pub(crate) custom_footer: Option<CustomFooter<'a>>,
}

type CustomFooter<'a> = (&'a mut dyn FnMut(&mut egui::Ui), f32);

/// Space between the cells of a preset grid, and the grid's inset from the panel's left edge.
const GRID_GAP: f32 = 4.0;
const GRID_INSET: f32 = 8.0;

/// Draw only the reusable library region. It owns groups/items, selection, drag/drop and item/group
/// context targets, but not the containing panel's optional controls or footer.
///
/// `allocated_height` is reserved by the caller or picker shell; the browser owns only the
/// scrolling library region, not search, recents, panel menus, footer controls, or command dispatch.
pub(crate) fn browser(ui: &mut egui::Ui, st: &mut PresetUi, adapter: &mut BrowserAdapter<'_>, allocated_height: f32) -> Vec<Ev> {
    let t = Tokens::get(ui.ctx());
    let panel = adapter.panel;
    let groups = adapter.groups;
    let BrowserOptions { canvas, flush, .. } = adapter.options;
    let capabilities = adapter.capabilities;
    let item_context_menu = adapter.item_context_menu;
    let group_context_menu = adapter.group_context_menu;
    let paint_group = &mut adapter.paint_group;
    let paint_item = &mut adapter.paint_item;
    let max_h = allocated_height;
    let mut ev = Vec::new();
    let size = *st.sizes.get(panel).unwrap_or(&40.0);
    let list = size <= MIN_THUMB + 0.5;
    let selected = st.selected.get(panel).cloned();
    let rename_row_top = ui.cursor().top();
    let has_rename_row = st.renaming.as_ref().is_some_and(|r| r.0 == panel);
    // Rename field (Photoshop opens a small dialog; inline keeps the panel modeless).
    if let Some((p, key, mut text)) = st.renaming.clone().filter(|r| r.0 == panel) {
        let mut done = None;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Name:")).color(t.text_dim).size(11.5));
            let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(ui.available_width() - 4.0));
            if !r.has_focus() && !r.lost_focus() {
                r.request_focus();
            }
            if r.lost_focus() {
                done = Some(ui.input(|i| i.key_pressed(egui::Key::Enter)));
            }
        });
        st.renaming = Some((p, key.clone(), text.clone()));
        if let Some(ok) = done {
            st.renaming = None;
            if ok && !text.trim().is_empty() {
                match key.strip_prefix("group:") {
                    Some(g) => ev.push(Ev::RenameGroup(g.to_string(), text.trim().to_string())),
                    None => ev.push(Ev::Rename(key, text.trim().to_string())),
                }
            }
        }
    }
    // The rename row is part of the browser's allocated height, not extra height above it. The
    // scroll area's minimum must exclude the row (and its layout spacing), or a short dock can
    // push the shell-owned footer below the panel.
    let rename_row_height = if has_rename_row { (ui.cursor().top() - rename_row_top).max(0.0) } else { 0.0 };
    let scroll_height = (max_h - rename_row_height).max(0.0);
    let mut headers: Vec<(usize, Rect, String)> = Vec::new();
    let mut dropped: Option<(String, Pos2)> = None;
    // Folder drag (#3658): the order the folders are drawn in, and what the pointer is doing this
    // frame (the insertion line and the drop are resolved once every header is known).
    let mut drag: Option<(usize, Pos2)> = None;
    let mut released: Option<(usize, Pos2)> = None;
    let group_names: Vec<String> = groups.iter().map(|g| g.name.clone()).collect();
    let display = st.display_order(panel, &group_names);
    // The browser is the flexible middle region of a picker panel. Keep it at the allocated
    // height even when the library is short; otherwise the footer floats up and each consumer's
    // panel geometry depends on how many presets happen to be installed.
    let scroll = egui::ScrollArea::vertical()
        .id_salt(("preset-scroll", panel))
        .max_height(scroll_height)
        .min_scrolled_height(scroll_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
            for &gi in &display {
                let Some(g) = groups.get(gi) else { continue };
                let open = group_open(st, panel, gi, &g.name);
                let (hr, hresp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click_and_drag());
                if let Some(paint) = paint_group.as_deref_mut() {
                    paint(ui, hr, g, open, hresp.hovered() || hresp.dragged());
                } else {
                    if hresp.hovered() || hresp.dragged() {
                        ui.painter().rect_filled(hr, 2.0, t.hover.gamma_multiply(0.5));
                    }
                    let tri = Rect::from_center_size(pos2(hr.left() + 9.0, hr.center().y), vec2(12.0, 12.0));
                    crate::icons::paint(ui, tri, if open { "chevron-down" } else { "chevron-right" }, 10.0, t.text_dim);
                    let fold = Rect::from_center_size(pos2(hr.left() + 25.0, hr.center().y), vec2(14.0, 14.0));
                    crate::icons::paint(ui, fold, if open { "folder-open" } else { "folder" }, 12.0, t.icon);
                    ui.painter().text(pos2(hr.left() + 37.0, hr.center().y), Align2::LEFT_CENTER, &g.name, FontId::proportional(11.5), t.text);
                }
                if hresp.clicked() {
                    let k = format!("{panel}/{}", g.name);
                    if !st.toggled.remove(&k) {
                        st.toggled.insert(k);
                    }
                }
                if let Some(menu) = group_context_menu {
                    context_menu_target(hresp.clone(), |ui| menu(ui, st, panel, g, &mut ev));
                }
                if capabilities.group_drag
                    && hresp.dragged()
                    && let Some(p) = ui.ctx().pointer_interact_pos()
                {
                    drag = Some((gi, p));
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                }
                if capabilities.group_drag
                    && hresp.drag_stopped()
                    && let Some(p) = ui.ctx().pointer_interact_pos()
                {
                    released = Some((gi, p));
                }
                headers.push((gi, hr, g.name.clone()));
                if !open {
                    continue;
                }
                let w = ui.available_width();
                let pad = if list { 0.0 } else { GRID_INSET };
                let gap = if list || flush { 0.0 } else { GRID_GAP };
                let (cell, cols) = if list {
                    (vec2(w, 24.0), 1)
                } else {
                    let cols = ((w - pad + gap) / (size + gap)).floor().max(1.0) as usize;
                    (vec2(size, size), cols)
                };
                let rows = g.items.len().div_ceil(cols);
                let (area, _) = ui.allocate_exact_size(vec2(w, rows as f32 * (cell.y + gap) + 2.0), Sense::hover());
                for (ii, it) in g.items.iter().enumerate() {
                    let (cx, cy) = ((ii % cols) as f32, (ii / cols) as f32);
                    let r = Rect::from_min_size(area.min + vec2(pad + cx * (cell.x + gap), cy * (cell.y + gap)), cell);
                    let resp = ui.interact(r, ui.id().with((panel, gi, ii)), Sense::click_and_drag());
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &it.name));
                    let is_sel = selected.as_deref() == Some(&it.key);
                    if list {
                        if is_sel {
                            ui.painter().rect_filled(r, 0.0, t.row_selected);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.5));
                        }
                        let tr = Rect::from_min_size(r.min + vec2(24.0, 2.0), vec2(20.0, 20.0));
                        paint_item(ui, tr, gi, ii);
                        ui.painter().text(pos2(tr.right() + 8.0, r.center().y), Align2::LEFT_CENTER, &it.name, FontId::proportional(11.5), t.text);
                    } else {
                        paint_item(ui, r, gi, ii);
                        // Touching cells draw no border of their own: only the selected and the
                        // hovered one are outlined, over their neighbours, as in Photoshop.
                        let stroke = if is_sel {
                            Some((Stroke::new(2.0, t.accent), egui::StrokeKind::Outside))
                        } else if resp.hovered() {
                            Some((Stroke::new(1.0, t.text_dim), egui::StrokeKind::Outside))
                        } else if flush {
                            None
                        } else {
                            Some((Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside))
                        };
                        if let Some((stroke, kind)) = stroke {
                            ui.painter().rect_stroke(r, CornerRadius::same(if flush { 0 } else { 2 }), stroke, kind);
                        }
                    }
                    let resp = if list { resp } else { resp.on_hover_text(&it.name) };
                    if capabilities.activation && resp.double_clicked() {
                        ev.push(Ev::Activate(it.key.clone()));
                    } else if capabilities.selection && resp.clicked() {
                        ev.push(Ev::Select(it.key.clone()));
                    }
                    if capabilities.item_drag
                        && resp.dragged()
                        && let Some(pp) = ui.ctx().pointer_interact_pos()
                    {
                        // A ghost of the swatch follows the pointer.
                        let lp = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("preset-drag")));
                        let gr = Rect::from_center_size(pp, vec2(28.0, 28.0));
                        lp.rect_filled(gr, 3.0, t.card.gamma_multiply(0.8));
                        lp.rect_stroke(gr, 3.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Outside);
                        lp.text(gr.center(), Align2::CENTER_CENTER, "+", FontId::proportional(16.0), t.accent);
                    }
                    if capabilities.item_drag
                        && resp.drag_stopped()
                        && let Some(pp) = ui.ctx().pointer_interact_pos()
                        && !r.contains(pp)
                    {
                        dropped = Some((it.key.clone(), pp));
                    }
                    if let Some(menu) = item_context_menu {
                        context_menu_target(resp.clone(), |ui| menu(ui, st, panel, it, g, &group_names, &mut ev));
                    }
                }
            }
            // Insertion line while a folder is dragged by its header.
            if let Some((from, p)) = drag {
                let before = group_drop_before(&headers, from, p.y);
                let anchor = match before {
                    Some(i) => headers.iter().find(|(x, _, _)| *x == i).map(|(_, r, _)| *r),
                    None => headers.last().map(|(_, r, _)| *r),
                };
                if let Some(r) = anchor {
                    let y = if before.is_some() { r.top() - 1.0 } else { r.bottom() + 1.0 };
                    ui.painter().line_segment([pos2(r.left(), y), pos2(r.right(), y)], Stroke::new(2.0, t.accent));
                }
            }
        });
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("preset-browser-rect", panel)), scroll.inner_rect));
    // Apply the drop now every header rect is known; the next frame draws the new order.
    if let Some((from, p)) = released {
        let before = group_drop_before(&headers, from, p.y);
        let from_name = groups.get(from).map(|g| g.name.clone());
        let before_name = before.and_then(|i| groups.get(i).map(|g| g.name.clone()));
        if let Some(from_name) = from_name {
            st.move_group(panel, &group_names, &from_name, before_name.as_deref());
        }
    }
    let recorded: Vec<(String, Rect)> = headers.iter().map(|(_, r, n)| (n.clone(), *r)).collect();
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("preset-group-headers", panel)), recorded));
    if let Some((key, pos)) = dropped {
        if let Some((_, _, g)) = headers.iter().find(|(_, r, _)| r.contains(pos)) {
            ev.push(Ev::Move(key, g.clone()));
        } else if canvas.contains(pos) {
            ev.push(Ev::Drop(key, pos));
        }
    }
    ev
}

/// General picker-panel shell. It lays out optional panel menu, search and recents controls, gives
/// the remaining vertical space to the browser, then places the optional footer. The browser's
/// item/group actions remain generic `Ev` values for the consumer to handle.
///
/// The shell reserves vertical space for enabled controls before drawing the browser, keeping the
/// library viewport stable when groups are collapsed or the library is empty. Asset commands remain
/// the responsibility of the caller that consumes the returned events.
pub(crate) fn picker_shell(ui: &mut egui::Ui, st: &mut PresetUi, adapter: &mut BrowserAdapter<'_>, mut shell: ShellOptions<'_>) -> Vec<Ev> {
    let panel = adapter.panel.to_string();
    shell_controls(ui, &mut shell);

    let footer_id = egui::Id::new(("preset-footer", panel.as_str()));
    let footer_h = if shell.footer.is_some() { ui.data(|d| d.get_temp::<f32>(footer_id)).unwrap_or(FOOTER_GUESS) } else { 0.0 };
    let allocated_h = adapter.options.max_h.min((ui.available_height() - footer_h).max(0.0));
    let mut events = browser(ui, st, adapter, allocated_h);

    if let Some(footer) = shell.footer {
        let footer_top = ui.min_rect().bottom();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if footer.size_slider {
                let size = *st.sizes.get(&panel).unwrap_or(&40.0);
                let mut next = size;
                ui.add_sized(vec2(90.0, 18.0), egui::Slider::new(&mut next, MIN_THUMB..=96.0).show_value(false))
                    .on_hover_text(tl!("Thumbnail size (smallest: list)"));
                if (next - size).abs() > 0.01 {
                    st.sizes.insert(panel.clone(), next);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if footer.delete
                    && crate::icons::button(ui, "trash", 24.0, false, tl!("Delete")).clicked()
                    && let Some(k) = st.selected.get(&panel).cloned()
                {
                    events.push(Ev::Delete(k));
                }
                if footer.new_item && crate::icons::button(ui, "plus", 24.0, false, footer.new_tip).clicked() {
                    events.push(Ev::New);
                }
                if footer.new_group && crate::icons::button(ui, "folder-plus", 24.0, false, tl!("Create new group")).clicked() {
                    events.push(Ev::NewGroup);
                }
            });
        });
        let measured = ui.min_rect().bottom() - footer_top;
        if measured.is_finite() && (measured - footer_h).abs() > 0.5 {
            ui.data_mut(|d| d.insert_temp(footer_id, measured));
            ui.ctx().request_repaint();
        }
    }
    events
}

/// Shared shell layout for consumers with a specialised browser renderer. The shell owns the
/// optional top controls and allocates the remaining height; the consumer keeps its own rendering
/// and action model until it can be represented by `BrowserAdapter`.
///
/// Unlike [`picker_shell`], this variant does not require `BrowserAdapter`; it passes the height
/// remaining after optional controls to `draw_browser`, allowing specialised views to retain their
/// own cards, nested-folder interactions, and rendering rules.
pub(crate) fn picker_shell_custom<R>(ui: &mut egui::Ui, max_h: f32, mut shell: ShellOptions<'_>, draw_browser: impl FnOnce(&mut egui::Ui, f32) -> R) -> R {
    debug_assert!(shell.footer.is_none(), "custom browser renderers use the custom footer slot");
    shell_controls(ui, &mut shell);
    let footer_h = shell.custom_footer.as_ref().map_or(0.0, |(_, h)| *h + 4.0);
    let allocated_h = max_h.min((ui.available_height() - footer_h).max(0.0));
    let result = draw_browser(ui, allocated_h);
    if let Some((footer, _)) = shell.custom_footer {
        ui.add_space(4.0);
        footer(ui);
    }
    result
}

/// Paint optional shell controls in their shared top-to-bottom order.
fn shell_controls(ui: &mut egui::Ui, shell: &mut ShellOptions<'_>) {
    if let Some(header) = shell.header.as_deref_mut() {
        ui.horizontal(|ui| header(ui));
    }
    if let Some(menu) = shell.panel_menu.as_deref_mut() {
        ui.horizontal(|ui| menu(ui));
    }
    if let Some(search) = shell.search.as_deref_mut() {
        search(ui);
    }
    if let Some(recents) = shell.recents.as_deref_mut() {
        recents(ui);
    }
}

/// The folder (by library index) a header dragged to `y` lands before (`None` = last). `headers`
/// is in display order; mirrors `dock::drop_before`.
fn group_drop_before(headers: &[(usize, Rect, String)], from: usize, y: f32) -> Option<usize> {
    let hit = headers.iter().find(|(_, r, _)| y < r.center().y).map(|(i, _, _)| *i);
    match hit {
        Some(i) if i == from => Some(from),
        Some(i) => {
            let after_self = headers.iter().position(|(x, _, _)| *x == from).zip(headers.iter().position(|(x, _, _)| *x == i)).is_some_and(|(a, b)| b == a + 1);
            if after_self { Some(from) } else { Some(i) }
        }
        None => None,
    }
}

/// Folder header rects drawn last frame (name → rect), in display order; for tests and automation.
pub fn last_group_headers(ctx: &egui::Context, panel: &str) -> Vec<(String, Rect)> {
    ctx.data(|d| d.get_temp(egui::Id::new(("preset-group-headers", panel)))).unwrap_or_default()
}

/// The allocated viewport from the last frame; useful to verify the shared browser's layout.
#[cfg(test)]
pub(crate) fn last_browser_rect(ctx: &egui::Context, panel: &str) -> Option<Rect> {
    ctx.data(|d| d.get_temp(egui::Id::new(("preset-browser-rect", panel))))
}

/// Shared handling of the generic events through `<prefix>.edit` / `.new`.
fn generic(app: &mut PhotocraftApp, panel: &str, prefix: &str, e: &Ev, new_params: Value) -> bool {
    let edit = format!("{prefix}.edit");
    match e {
        Ev::Rename(k, n) => {
            run(app, &edit, json!({"action": "rename", "preset": k, "name": n}));
            if app.ui.presets_ui.selected.get(panel) == Some(k) {
                app.ui.presets_ui.selected.insert(panel.into(), n.clone());
            }
        }
        Ev::Delete(k) => {
            if run(app, &edit, json!({"action": "delete", "preset": k})).is_some() {
                app.ui.presets_ui.selected.remove(panel);
            }
        }
        Ev::Move(k, g) => drop(run(app, &edit, json!({"action": "move", "preset": k, "to": g}))),
        Ev::RenameGroup(g, n) => drop(run(app, &edit, json!({"action": "renameGroup", "group": g, "name": n}))),
        Ev::DeleteGroup(g) => drop(run(app, &edit, json!({"action": "deleteGroup", "group": g}))),
        Ev::NewGroup => {
            if let Some(r) = run(app, &edit, json!({"action": "newGroup", "name": "Group"})) {
                let g = r["group"].as_str().unwrap_or_default().to_string();
                app.ui.presets_ui.renaming = Some((panel.into(), format!("group:{g}"), g));
            }
        }
        Ev::New => {
            if let Some(r) = run(app, &format!("{prefix}.new"), new_params) {
                let name = r.get("name").or(r.get("pattern")).and_then(Value::as_str).unwrap_or_default().to_string();
                app.ui.presets_ui.selected.insert(panel.into(), name.clone());
                if prefix != "pattern.presets" {
                    app.ui.presets_ui.renaming = Some((panel.into(), name.clone(), name));
                }
            }
        }
        _ => return false,
    }
    true
}

fn empty(ui: &mut egui::Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(s).color(t.text_faint).size(11.5));
}

/// Where drops onto the canvas are accepted: the canvas proper, rulers excluded (a drop on the
/// ruler strip is not a canvas drop).
fn canvas_drop_rect(app: &PhotocraftApp) -> egui::Rect {
    crate::rulers::content_rect(app, app.last_canvas_rect)
}

// ------------------------------------------------------------------ Gradients

pub fn gradients_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let (fg, bg) = (app.session.tools.foreground, app.session.tools.background);
    let groups_src = app.session.presets.gradients.clone();
    // Current gradient (what the Gradient tool paints), like the panel's top "recent" row.
    let cur = app.session.presets.gradient.clone();
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
    paint_gradient(ui, r, &cur.resolve(fg, bg));
    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, Tokens::get(ui.ctx()).card_border), egui::StrokeKind::Inside);
    ui.add_space(4.0);
    let groups: Vec<GroupView> = groups_src
        .iter()
        .map(|g| GroupView { name: g.name.clone(), items: g.items.iter().map(|i| ItemView { key: i.name.clone(), name: i.name.clone() }).collect() })
        .collect();
    let canvas = canvas_drop_rect(app);
    let mut st = std::mem::take(&mut app.ui.presets_ui);
    let max_h = (ui.available_height() - 40.0).clamp(90.0, 260.0);
    let mut adapter = BrowserAdapter {
        panel: "gradients",
        groups: &groups,
        options: BrowserOptions { canvas, max_h, flush: false },
        capabilities: BrowserCapabilities::default(),
        item_context_menu: Some(standard_item_context_menu),
        group_context_menu: Some(standard_group_context_menu),
        paint_group: None,
        paint_item: &mut |ui, r, gi, ii| {
            let stops = groups_src[gi].items[ii].resolve(fg, bg);
            paint_gradient(ui, r, &stops);
        },
    };
    let events =
        picker_shell(ui, &mut st, &mut adapter, ShellOptions { footer: Some(FooterOptions::standard(tl!("Create new gradient"))), ..Default::default() });
    app.ui.presets_ui = st;
    for e in events {
        match &e {
            Ev::Select(k) => {
                app.ui.presets_ui.selected.insert("gradients".into(), k.clone());
                run(app, "gradient.presets.select", json!({"preset": k}));
            }
            Ev::Activate(k) | Ev::Drop(k, _) => {
                app.ui.presets_ui.selected.insert("gradients".into(), k.clone());
                run(app, "gradient.presets.apply", json!({"preset": k}));
            }
            _ => {
                generic(app, "gradients", "gradient.presets", &e, json!({"name": "Custom"}));
            }
        }
    }
}

// ------------------------------------------------------------------ Patterns

pub fn patterns_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    photocraft_engine::presets::patterns::sync(&mut app.session);
    let lib = app.session.patterns.items.clone();
    let groups = pattern_group_views(&lib, &app.session.presets.pattern_groups);
    let canvas = canvas_drop_rect(app);
    let mut st = std::mem::take(&mut app.ui.presets_ui);
    let max_h = (ui.available_height() - 40.0).clamp(90.0, 280.0);
    let ctx = ui.ctx().clone();
    let mut adapter = BrowserAdapter {
        panel: "patterns",
        groups: &groups,
        options: BrowserOptions { canvas, max_h, flush: false },
        capabilities: BrowserCapabilities::default(),
        item_context_menu: Some(standard_item_context_menu),
        group_context_menu: Some(standard_group_context_menu),
        paint_group: None,
        paint_item: &mut |ui, r, gi, ii| {
            if let Some(pattern) = groups.get(gi).and_then(|g| g.items.get(ii)).and_then(|item| lib.iter().find(|p| p.id == item.key)) {
                let tex = pattern_texture(&ctx, pattern);
                ui.painter().image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
            }
        },
    };
    let events = picker_shell(
        ui,
        &mut st,
        &mut adapter,
        ShellOptions { footer: Some(FooterOptions::standard(tl!("Create new pattern from the selection"))), ..Default::default() },
    );
    app.ui.presets_ui = st;
    for e in events {
        match &e {
            Ev::Select(k) => {
                app.ui.presets_ui.selected.insert("patterns".into(), k.clone());
                run(app, "pattern.presets.select", json!({"pattern": k}));
            }
            Ev::Activate(k) | Ev::Drop(k, _) => {
                app.ui.presets_ui.selected.insert("patterns".into(), k.clone());
                run(app, "pattern.presets.apply", json!({"pattern": k}));
            }
            _ => {
                generic(app, "patterns", "pattern.presets", &e, json!({}));
            }
        }
    }
}

// ------------------------------------------------------------------ floating panels

fn float_window(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    key: &str,
    title: &str,
    width: f32,
    offset: f32,
    body: impl FnOnce(&mut PhotocraftApp, &mut egui::Ui),
) -> bool {
    let t = Tokens::get(ctx);
    let frame = egui::Frame::NONE
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(CornerRadius::same(t.radius_lg as u8))
        .shadow(egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: t.shadow })
        .inner_margin(egui::Margin::same(8));
    let canvas = canvas_drop_rect(app);
    let mut close = false;
    egui::Window::new(title)
        .id(egui::Id::new(("preset-window", key)))
        .title_bar(false)
        .resizable(false)
        .frame(frame)
        .default_pos(pos2((canvas.right() - width - 16.0 - offset).max(canvas.left() + 8.0), canvas.top() + 40.0))
        .show(ctx, |ui| {
            ui.set_width(width);
            // Photoshop-style tab header with a close box.
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!(&title)).color(t.text).size(12.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::icons::button(ui, "x", 20.0, false, tl!("Close")).clicked() {
                        close = true;
                    }
                });
            });
            crate::widgets::hairline(ui);
            ui.add_space(4.0);
            body(app, ui);
        });
    close
}

/// Draws the open floating panels (Styles, Shapes, Tool Presets, Clone Source).
pub fn windows(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let p = app.ui.presets_ui.clone();
    if p.styles && float_window(app, ctx, "styles", tl!("Styles"), 250.0, 0.0, styles_panel) {
        app.ui.presets_ui.styles = false;
    }
    if p.shapes && float_window(app, ctx, "shapes", tl!("Shapes"), 250.0, 280.0, shapes_panel) {
        app.ui.presets_ui.shapes = false;
    }
    if p.tool_presets && float_window(app, ctx, "toolPresets", tl!("Tool Presets"), 250.0, 560.0, tool_presets_panel) {
        app.ui.presets_ui.tool_presets = false;
    }
    if p.clone_source && float_window(app, ctx, "cloneSource", tl!("Clone Source"), 270.0, 840.0, clone_source_panel) {
        app.ui.presets_ui.clone_source = false;
    }
}

// ------------------------------------------------------------------ Styles

pub fn styles_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let src = app.session.presets.styles.clone();
    let groups: Vec<GroupView> = src
        .iter()
        .map(|g| GroupView { name: g.name.clone(), items: g.items.iter().map(|i| ItemView { key: i.name.clone(), name: i.name.clone() }).collect() })
        .collect();
    let canvas = canvas_drop_rect(app);
    let mut st = std::mem::take(&mut app.ui.presets_ui);
    let ctx = ui.ctx().clone();
    let t = Tokens::get(&ctx);
    let mut texes: Vec<Vec<TextureHandle>> = Vec::new();
    for g in &src {
        texes.push(g.items.iter().map(|s| style_texture(app, &ctx, s)).collect());
    }
    let mut adapter = BrowserAdapter {
        panel: "styles",
        groups: &groups,
        options: BrowserOptions { canvas, max_h: 300.0, flush: false },
        capabilities: BrowserCapabilities::default(),
        item_context_menu: Some(standard_item_context_menu),
        group_context_menu: Some(standard_group_context_menu),
        paint_group: None,
        paint_item: &mut |ui, r, gi, ii| {
            ui.painter().rect_filled(r, 2.0, t.field);
            ui.painter().image(texes[gi][ii].id(), r, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
        },
    };
    let events = picker_shell(
        ui,
        &mut st,
        &mut adapter,
        ShellOptions { footer: Some(FooterOptions::standard(tl!("Create new style from the selected layer"))), ..Default::default() },
    );
    app.ui.presets_ui = st;
    let shift = ui.input(|i| i.modifiers.shift);
    for e in events {
        match &e {
            Ev::Select(k) | Ev::Activate(k) => {
                app.ui.presets_ui.selected.insert("styles".into(), k.clone());
                // Clicking applies to the selected layers (⇧ adds to their effects).
                run(app, "style.presets.apply", json!({"preset": k, "add": shift}));
            }
            Ev::Drop(k, pos) => {
                // Dropping on the canvas styles the layer under the pointer's selection: the active layer.
                let _ = pos;
                run(app, "style.presets.apply", json!({"preset": k}));
            }
            _ => {
                generic(app, "styles", "style.presets", &e, json!({"name": "Style"}));
            }
        }
    }
}

// ------------------------------------------------------------------ Shapes

pub fn shapes_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let src = photocraft_engine::presets::shapes::all_groups(&app.session);
    let groups: Vec<GroupView> = src
        .iter()
        .map(|g| GroupView { name: g.name.clone(), items: g.items.iter().map(|i| ItemView { key: i.name.clone(), name: i.name.clone() }).collect() })
        .collect();
    let canvas = canvas_drop_rect(app);
    let mut st = std::mem::take(&mut app.ui.presets_ui);
    let ctx = ui.ctx().clone();
    let t = Tokens::get(&ctx);
    let mut adapter = BrowserAdapter {
        panel: "shapes",
        groups: &groups,
        options: BrowserOptions { canvas, max_h: 300.0, flush: false },
        capabilities: BrowserCapabilities::default(),
        item_context_menu: Some(standard_item_context_menu),
        group_context_menu: Some(standard_group_context_menu),
        paint_group: None,
        paint_item: &mut |ui, r, gi, ii| {
            let tex = shape_texture(&ctx, &src[gi].items[ii]);
            ui.painter().image(tex.id(), r.shrink(2.0), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), t.text);
        },
    };
    let events = picker_shell(
        ui,
        &mut st,
        &mut adapter,
        ShellOptions { footer: Some(FooterOptions::standard(tl!("Create new shape from the current path"))), ..Default::default() },
    );
    app.ui.presets_ui = st;
    let fill = shape_fill(app);
    for e in events {
        match &e {
            Ev::Select(k) => {
                // Selecting a shape picks it for the Custom Shape tool (and switches to it).
                app.ui.presets_ui.selected.insert("shapes".into(), k.clone());
                app.ui.presets_ui.custom_shape = k.clone();
                app.ui.tool = Tool::CustomShape;
            }
            Ev::Activate(k) => {
                app.ui.presets_ui.custom_shape = k.clone();
                run(app, "shape.presets.place", json!({"preset": k, "fill": fill}));
            }
            Ev::Drop(k, pos) => {
                app.ui.presets_ui.custom_shape = k.clone();
                let mut p = json!({"preset": k, "fill": fill});
                if let (Some(c), Some(d)) = (crate::canvas::doc_point_at(app, *pos), app.session.active()) {
                    let side = (d.doc.size.width.min(d.doc.size.height) as f64 * 0.3).max(8.0);
                    p["rect"] = json!([c[0] - side / 2.0, c[1] - side / 2.0, side, side]);
                }
                run(app, "shape.presets.place", p);
            }
            _ => {
                generic(app, "shapes", "shape.presets", &e, json!({"name": "Shape"}));
            }
        }
    }
}

fn shape_fill(app: &PhotocraftApp) -> Value {
    let f = app.session.tools.foreground;
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    if app.ui.tool_options.shape_fill { json!(format!("#{:02x}{:02x}{:02x}", q(f[0]), q(f[1]), q(f[2]))) } else { Value::Null }
}

/// Options-bar shape picker for the Custom Shape tool.
pub fn shape_picker(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!("Shape:")).color(t.text_dim).size(12.0));
    let groups = photocraft_engine::presets::shapes::all_groups(&app.session);
    let cur = app.ui.presets_ui.shape().to_string();
    let ctx = ui.ctx().clone();
    let (r, resp) = ui.allocate_exact_size(vec2(36.0, 22.0), Sense::click());
    ui.painter().rect_filled(r, 3.0, t.field);
    ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    if let Some(sh) = groups.iter().flat_map(|g| g.items.iter()).find(|s| s.name == cur) {
        let tex = shape_texture(&ctx, sh);
        ui.painter().image(
            tex.id(),
            Rect::from_center_size(r.center() - vec2(5.0, 0.0), vec2(18.0, 18.0)),
            Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
            t.text,
        );
    }
    crate::icons::paint(ui, Rect::from_center_size(pos2(r.right() - 8.0, r.center().y), vec2(10.0, 10.0)), "chevron-down", 9.0, t.text_dim);
    let resp = resp.on_hover_text(&cur);
    egui::Popup::menu(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_width(232.0);
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for g in &groups {
                ui.label(egui::RichText::new(&g.name).color(t.text_dim).size(11.0));
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
                    for sh in &g.items {
                        let (cr, cresp) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
                        let on = sh.name == cur;
                        ui.painter().rect_filled(
                            cr,
                            2.0,
                            if on {
                                t.accent_soft
                            } else if cresp.hovered() {
                                t.hover
                            } else {
                                t.field
                            },
                        );
                        let tex = shape_texture(&ctx, sh);
                        ui.painter().image(tex.id(), cr.shrink(4.0), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), t.text);
                        if cresp.on_hover_text(&sh.name).clicked() {
                            app.ui.presets_ui.custom_shape = sh.name.clone();
                            ui.close();
                        }
                    }
                });
            }
        });
    });
}

/// Custom Shape tool drag: places the selected shape in the dragged rect (⇧ keeps proportions).
pub fn finish_custom_shape(app: &mut PhotocraftApp, rect: [f64; 4], keep: bool, fill: Value, stroke: Value) {
    let name = app.ui.presets_ui.shape().to_string();
    let r = run(app, "shape.presets.place", json!({"preset": name, "rect": rect, "keepAspect": keep, "fill": fill, "stroke": stroke}));
    if r.is_none() && !photocraft_engine::presets::shapes::all_groups(&app.session).iter().any(|g| g.items.iter().any(|s| s.name == name)) {
        let template = "No custom shape \"{name}\": pick one in the options bar or Window › Shapes";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("name", &name)]);
    }
}

// ------------------------------------------------------------------ Tool Presets

fn tool_id(t: Tool) -> String {
    let s = format!("{t:?}");
    let mut c = s.chars();
    c.next().map(|f| f.to_ascii_lowercase().to_string() + c.as_str()).unwrap_or_default()
}

/// Apply a tool preset: the engine sets the brush/colour, the shell switches tool and options.
pub fn select_tool_preset(app: &mut PhotocraftApp, name: &str) {
    let before_ratio = crate::crop_ui::preset_ratio(app);
    let Some(r) = run(app, "tool.presets.select", json!({"preset": name})) else { return };
    if let Some(tool) = r["tool"].as_str().and_then(Tool::from_name) {
        crate::tool_memory::select(app, tool);
        // Each tool keeps its own brush (#218), so switch it in before the preset's brush lands
        // on it, and re-apply the brush the command just set for the old tool.
        crate::paint_mouse::sync_tool_brush(app);
        if let Some(b) = r["options"].get("brush").filter(|b| b.is_object()) {
            let _ = app.run("tools.setBrush", json!({"brush": b}));
        }
    }
    if let Some(o) = r["options"].get("toolOptions").and_then(Value::as_object) {
        let mut cur = serde_json::to_value(&app.ui.tool_options).unwrap_or_default();
        if let Some(c) = cur.as_object_mut() {
            for (k, v) in o {
                // An object option (`sample_all`, one per tool) takes only the entries the
                // preset stored, so the other tools' keep theirs (#5524).
                match (c.get_mut(k), v) {
                    (Some(Value::Object(have)), Value::Object(set)) => have.extend(set.clone()),
                    _ => {
                        c.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        if let Ok(opts) = serde_json::from_value(cur) {
            app.ui.tool_options = opts;
        }
    }
    // As the options bar does: a crop preset's ratio (or W x H) fits the frame at once.
    if crate::crop_ui::preset_ratio(app) != before_ratio {
        crate::crop_ui::apply_ratio(app);
    }
}

/// The `tool.presets.new` params for the + button: `tool`'s own options only, so choosing the
/// preset later leaves the other tools' settings alone.
fn new_tool_preset_params(app: &PhotocraftApp, tool: Tool) -> Value {
    let all = serde_json::to_value(&app.ui.tool_options).unwrap_or_default();
    let mut own: serde_json::Map<String, Value> =
        crate::tool_reset::option_keys(tool).iter().filter_map(|k| Some(((*k).to_string(), all.get(*k)?.clone()))).collect();
    // Sample All Layers is each tool's own (#5524): store `tool`'s alone.
    if let Some(k) = crate::tool_reset::sample_all_key(tool)
        && let Some(v) = all.get("sample_all").and_then(|m| m.get(&k))
    {
        own.insert("sample_all".into(), json!({ k: v }));
    }
    json!({"name": tool.label(), "tool": tool_id(tool), "options": {"toolOptions": own}})
}

pub fn tool_presets_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let only = app.ui.presets_ui.current_tool_only;
    let cur = app.ui.tool;
    let items: Vec<(String, String)> =
        app.session.presets.tool_presets.iter().filter(|p| !only || Tool::from_name(&p.tool) == Some(cur)).map(|p| (p.name.clone(), p.tool.clone())).collect();
    let selected = app.ui.presets_ui.selected.get("toolPresets").cloned();
    let mut action: Option<(String, Value)> = None;
    let mut pick: Option<String> = None;
    // Rename field.
    if let Some((_, key, mut text)) = app.ui.presets_ui.renaming.clone().filter(|r| r.0 == "toolPresets") {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Name:")).color(t.text_dim).size(11.5));
            let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(ui.available_width() - 4.0));
            if !r.has_focus() && !r.lost_focus() {
                r.request_focus();
            }
            if r.lost_focus() {
                app.ui.presets_ui.renaming = None;
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !text.trim().is_empty() {
                    action = Some(("tool.presets.edit".into(), json!({"action": "rename", "preset": key, "name": text.trim()})));
                }
            } else {
                app.ui.presets_ui.renaming = Some(("toolPresets".into(), key.clone(), text.clone()));
            }
        });
    }
    egui::ScrollArea::vertical().id_salt("tool-presets").max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if items.is_empty() {
            empty(ui, if only { tl!("No presets for the current tool.") } else { tl!("No tool presets.") });
        }
        for (name, tool) in &items {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
            if selected.as_deref() == Some(name) {
                ui.painter().rect_filled(r, 0.0, t.row_selected);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.5));
            }
            let icon = Tool::from_name(tool).map(crate::icons::tool_icon).unwrap_or("settings");
            crate::icons::paint(ui, Rect::from_center_size(pos2(r.left() + 14.0, r.center().y), vec2(16.0, 16.0)), icon, 13.0, t.icon);
            ui.painter().text(pos2(r.left() + 30.0, r.center().y), Align2::LEFT_CENTER, name, FontId::proportional(11.5), t.text);
            if resp.clicked() {
                pick = Some(name.clone());
            }
            resp.context_menu(|ui| {
                if ui.button(tl!("Rename Tool Preset…")).clicked() {
                    app.ui.presets_ui.renaming = Some(("toolPresets".into(), name.clone(), name.clone()));
                    ui.close();
                }
                if ui.button(tl!("Delete Tool Preset")).clicked() {
                    action = Some(("tool.presets.edit".into(), json!({"action": "delete", "preset": name})));
                    ui.close();
                }
            });
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        crate::widgets::checkbox(ui, &mut app.ui.presets_ui.current_tool_only, tl!("Current Tool Only"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::icons::button(ui, "trash", 24.0, false, tl!("Delete tool preset")).clicked()
                && let Some(n) = selected.clone()
            {
                action = Some(("tool.presets.edit".into(), json!({"action": "delete", "preset": n})));
            }
            if crate::icons::button(ui, "plus", 24.0, false, tl!("Create new tool preset")).clicked() {
                action = Some(("tool.presets.new".into(), new_tool_preset_params(app, cur)));
            }
        });
    });
    if let Some(n) = pick {
        app.ui.presets_ui.selected.insert("toolPresets".into(), n.clone());
        select_tool_preset(app, &n);
    }
    if let Some((cmd, p)) = action {
        let created = cmd == "tool.presets.new";
        if let Some(r) = run(app, &cmd, p)
            && created
        {
            let n = r["name"].as_str().unwrap_or_default().to_string();
            app.ui.presets_ui.selected.insert("toolPresets".into(), n.clone());
            app.ui.presets_ui.renaming = Some(("toolPresets".into(), n.clone(), n));
        }
    }
}

// ------------------------------------------------------------------ Clone Source

pub fn clone_source_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let cs = app.session.presets.clone.clone();
    let slot = cs.active().clone();
    let mut action: Option<Value> = None;
    // The five source buttons.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for i in 0..photocraft_engine::presets::clone_source::SLOTS {
            let set = cs.slots[i].source.is_some();
            let tip = match cs.slots[i].source {
                Some(s) => format!("Clone source {}: {:.0}, {:.0}", i + 1, s[0], s[1]),
                None => crate::i18n::fmt(
                    tl!("Clone source {n} ({key}-click with Clone Stamp to set)"),
                    &[("n", &(i + 1).to_string()), ("key", &crate::shortcuts::pretty("Alt"))],
                ),
            };
            let r = crate::icons::button(ui, "stamp", 30.0, cs.active == i, &tip);
            if set {
                let d = r.rect.right_bottom() - vec2(6.0, 6.0);
                ui.painter().circle_filled(d, 2.5, t.accent);
            }
            if r.clicked() {
                run(app, "cloneSource.select", json!({"index": i}));
            }
        }
    });
    ui.add_space(4.0);
    let src_line =
        match (slot.source, slot.layer.and_then(|l| app.session.active().and_then(|d| d.doc.layer(photocraft_doc::LayerId(l)).map(|x| x.name.clone())))) {
            (Some(_), Some(l)) => format!("Source: {} : {l}", app.session.active().map(|d| d.doc.name.clone()).unwrap_or_default()),
            (Some(_), None) => "Source: set".to_string(),
            _ => crate::i18n::fmt(tl!("Source: not set ({key}-click with Clone Stamp)"), &[("key", &crate::shortcuts::pretty("Alt"))]),
        };
    ui.label(egui::RichText::new(src_line).color(t.text_dim).size(11.0));
    ui.add_space(4.0);
    let lbl = |ui: &mut egui::Ui, s: &str| {
        ui.add_sized(vec2(18.0, 18.0), egui::Label::new(egui::RichText::new(s).color(t.text_dim).size(11.5)));
    };
    let off = slot.offset().unwrap_or([0.0, 0.0]);
    let (mut ox, mut oy) = (off[0] as f32, off[1] as f32);
    let (mut w, mut h) = (slot.scale[0] as f32, slot.scale[1] as f32);
    let mut rot = slot.rotation as f32;
    egui::Grid::new("clone-src-grid").num_columns(5).spacing(vec2(6.0, 4.0)).show(ui, |ui| {
        lbl(ui, "X:");
        if crate::widgets::value_field(ui, &mut ox, -30000.0..=30000.0, "px", 72.0).changed() && slot.source.is_some() {
            action = Some(json!({"offset": [ox, oy]}));
        }
        lbl(ui, "W:");
        if crate::widgets::value_field(ui, &mut w, -1000.0..=1000.0, "%", 64.0).changed() {
            action = Some(json!({"width": w}));
        }
        // The W/H chain: with it on, a new W or H scales the other by the same factor.
        let icon = if slot.linked { "link" } else { "unlink" };
        if crate::icons::button(ui, icon, 22.0, slot.linked, tl!("Maintain aspect ratio")).clicked() {
            action = Some(json!({"linked": !slot.linked}));
        }
        ui.end_row();
        lbl(ui, "Y:");
        if crate::widgets::value_field(ui, &mut oy, -30000.0..=30000.0, "px", 72.0).changed() && slot.source.is_some() {
            action = Some(json!({"offset": [ox, oy]}));
        }
        lbl(ui, "H:");
        if crate::widgets::value_field(ui, &mut h, -1000.0..=1000.0, "%", 64.0).changed() {
            action = Some(json!({"height": h}));
        }
        ui.end_row();
        lbl(ui, "A:");
        if crate::widgets::value_field(ui, &mut rot, -180.0..=180.0, "°", 72.0).changed() {
            action = Some(json!({"rotation": rot}));
        }
        ui.horizontal(|ui| {
            if crate::icons::button(ui, "arrow-left-right", 22.0, slot.flip_h, tl!("Flip Horizontal")).clicked() {
                action = Some(json!({"flipH": !slot.flip_h}));
            }
            if crate::icons::button(ui, "rotate-cw", 22.0, false, tl!("Reset Transform")).clicked() {
                run(app, "cloneSource.resetTransform", json!({}));
            }
        });
        ui.horizontal(|ui| {
            if crate::icons::button(ui, "scaling", 22.0, slot.flip_v, tl!("Flip Vertical")).clicked() {
                action = Some(json!({"flipV": !slot.flip_v}));
            }
        });
        ui.end_row();
    });
    if let Some(p) = action {
        run(app, "cloneSource.set", p);
    }
    ui.add_space(6.0);
    crate::widgets::hairline(ui);
    ui.add_space(4.0);
    // Overlay options.
    let mut o = cs.overlay.clone();
    let before = o.clone();
    crate::widgets::checkbox(ui, &mut o.show, tl!("Show Overlay"));
    ui.horizontal(|ui| {
        ui.add_space(18.0);
        ui.label(egui::RichText::new(tl!("Opacity:")).color(t.text_dim).size(11.5));
        crate::widgets::value_field(ui, &mut o.opacity, 0.0..=100.0, "%", 56.0);
        let opts = [
            ("normal".to_string(), tl!("Normal")),
            ("darken".to_string(), tl!("Darken")),
            ("lighten".to_string(), tl!("Lighten")),
            ("difference".to_string(), tl!("Difference")),
        ];
        crate::widgets::dropdown(ui, "clone-overlay-mode", &mut o.blend, &opts, 90.0);
    });
    ui.horizontal(|ui| {
        ui.add_space(18.0);
        crate::widgets::checkbox(ui, &mut o.clipped, tl!("Clipped"));
        crate::widgets::checkbox(ui, &mut o.auto_hide, tl!("Auto Hide"));
        crate::widgets::checkbox(ui, &mut o.invert, tl!("Invert"));
    });
    if o != before {
        run(
            app,
            "cloneSource.overlay",
            json!({"show": o.show, "opacity": o.opacity, "clipped": o.clipped, "autoHide": o.auto_hide, "invert": o.invert, "blend": o.blend}),
        );
    }
}

/// Where the active clone source samples for document point `at` (for the canvas marker).
/// The first stroke anchors at its press, so the marker follows before the stroke commits.
pub fn clone_sample_point(app: &PhotocraftApp, at: Option<[f64; 2]>) -> Option<[f64; 2]> {
    use photocraft_engine::presets::clone_source::{Mapping, transform_matrix};
    let s = app.session.presets.clone.active();
    let src = s.source?;
    let stroking = app.drag.as_ref().filter(|d| matches!(d.tool, Tool::CloneStamp | Tool::Healing)).map(|d| d.start);
    let anchor = if app.ui.tool_options.clone_aligned { s.anchor.or(stroking) } else { stroking };
    match (anchor, at) {
        (Some(a), Some(h)) => {
            let m = Mapping { source: (src[0], src[1]), anchor: (a[0], a[1]), m: transform_matrix(s.scale, s.rotation, s.flip_h, s.flip_v) };
            let (x, y) = m.map(h[0], h[1]);
            Some([x, y])
        }
        _ => Some(src),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menus::invoke;
    use crate::state::ToolOptions;

    fn app() -> (PhotocraftApp, egui::Context) {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.sync_views();
        (app, ctx)
    }

    #[test]
    fn window_menu_toggles_panels_and_color_tabs() {
        let (mut app, ctx) = app();
        for id in PANELS {
            assert!(crate::menus::is_live(id), "{id}");
        }
        invoke(&mut app, &ctx, "window.panel.gradients", Value::Null).unwrap();
        assert!(app.ui.panels.color && app.ui.dock_tabs.color == 2);
        assert_eq!(checked(&app, "window.panel.gradients"), Some(true));
        assert_eq!(checked(&app, "window.panel.patterns"), Some(false));
        invoke(&mut app, &ctx, "window.panel.patterns", Value::Null).unwrap();
        assert_eq!(app.ui.dock_tabs.color, 3);
        // Choosing it again hides the card.
        invoke(&mut app, &ctx, "window.panel.patterns", Value::Null).unwrap();
        assert!(!app.ui.panels.color);
        for (id, f) in [("window.panel.styles", 0), ("window.panel.shapes", 1), ("window.panel.toolPresets", 2), ("window.panel.cloneSource", 3)] {
            invoke(&mut app, &ctx, id, Value::Null).unwrap();
            let p = &app.ui.presets_ui;
            assert!([p.styles, p.shapes, p.tool_presets, p.clone_source][f], "{id}");
            assert_eq!(checked(&app, id), Some(true));
            invoke(&mut app, &ctx, id, json!({"show": false})).unwrap();
            assert_eq!(checked(&app, id), Some(false));
        }
    }

    #[test]
    fn tool_preset_switches_tool_and_options() {
        let (mut app, _) = app();
        app.ui.tool = Tool::Brush;
        select_tool_preset(&mut app, "Radial Gradient");
        assert_eq!(app.ui.tool, Tool::Gradient);
        assert_eq!(app.ui.tool_options.gradient_style, "radial");
        select_tool_preset(&mut app, "Soft Eraser 60 px");
        assert_eq!(app.ui.tool, Tool::Eraser);
        assert_eq!(app.session.tools.brush.size, 60.0);
        assert_eq!(tool_id(Tool::CloneStamp), "cloneStamp");
        assert_eq!(tool_id(Tool::PatternStamp), "patternStamp");
        assert_eq!(Tool::from_name(&tool_id(Tool::CustomShape)), Some(Tool::CustomShape));
    }

    /// A crop preset picked in Tool Presets fits the frame to its ratio at once, as the options
    /// bar's dropdown does.
    #[test]
    fn a_crop_tool_preset_fits_the_frame_to_its_ratio() {
        let (mut app, _) = app();
        app.ui.tool = Tool::Crop;
        crate::crop_ui::ensure_frame(&mut app);
        assert_eq!(app.ui.crop_rect, Some([0.0, 0.0, 64.0, 48.0]));
        app.run("tool.presets.new", json!({"name": "Square", "tool": "crop", "options": {"toolOptions": {"crop_ratio": "1:1"}}})).unwrap();
        select_tool_preset(&mut app, "Square");
        assert_eq!(app.ui.tool_options.crop_ratio, "1:1");
        assert_eq!(app.ui.crop_rect, Some([8.0, 0.0, 56.0, 48.0]));
    }

    /// #7356: a preset made with + keeps only its own tool's options, so choosing it doesn't
    /// reset the Crop tool's Delete Cropped Pixels or the Move tool's Auto-Select.
    #[test]
    fn new_tool_preset_stores_only_its_tools_options() {
        let (mut app, _) = app();
        app.ui.tool = Tool::RectMarquee;
        app.ui.tool_options.feather = 5.0;
        let p = new_tool_preset_params(&app, Tool::RectMarquee);
        let r = app.run("tool.presets.new", p).unwrap();
        let name = r["name"].as_str().unwrap().to_string();
        let stored = &app.session.presets.tool_presets.iter().find(|t| t.name == name).unwrap().options["toolOptions"];
        assert_eq!(stored["feather"], json!(5.0));
        assert!(stored.as_object().unwrap().keys().all(|k| !k.starts_with("crop_") && !k.starts_with("move_")), "{stored}");
        app.ui.tool_options.feather = 0.0;
        app.ui.tool_options.crop_delete = !ToolOptions::default().crop_delete;
        app.ui.tool_options.move_auto_select = !ToolOptions::default().move_auto_select;
        select_tool_preset(&mut app, &name);
        assert_eq!(app.ui.tool_options.feather, 5.0);
        assert_eq!(app.ui.tool_options.crop_delete, !ToolOptions::default().crop_delete);
        assert_eq!(app.ui.tool_options.move_auto_select, !ToolOptions::default().move_auto_select);
    }

    /// #5524 with #7356: a Blur preset keeps Blur's Sample All Layers and leaves the Magic Wand's
    /// alone, both when it is made and when it is chosen.
    #[test]
    fn new_tool_preset_stores_only_its_tools_sample_all_layers() {
        let (mut app, _) = app();
        app.ui.tool = Tool::Blur;
        app.ui.tool_options.sample_all.blur = true;
        let p = new_tool_preset_params(&app, Tool::Blur);
        assert_eq!(p["options"]["toolOptions"]["sample_all"], json!({"blur": true}));
        let name = app.run("tool.presets.new", p).unwrap()["name"].as_str().unwrap().to_string();
        app.ui.tool_options.sample_all.blur = false;
        app.ui.tool_options.sample_all.magic_wand = true;
        select_tool_preset(&mut app, &name);
        assert!(app.ui.tool_options.sample_all.blur);
        assert!(app.ui.tool_options.sample_all.magic_wand, "the Magic Wand's Sample All Layers is not Blur's");
    }

    #[test]
    fn custom_shape_tool_places_the_picked_shape() {
        let (mut app, _) = app();
        app.ui.presets_ui.custom_shape = "Star".into();
        finish_custom_shape(&mut app, [4.0, 4.0, 20.0, 20.0], true, json!("#ff0000"), Value::Null);
        let d = app.session.active().unwrap();
        let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
        assert_eq!(l.name, "Star");
        assert!(matches!(l.content, photocraft_doc::LayerContent::Shape(_)));
    }

    /// #3658: a group/folder drag reorders by name, stores only a real reorder, and falls back to
    /// the library's order when the saved order matches it or names a group that's gone.
    #[test]
    fn moving_a_folder_reorders_like_the_dock_tabs() {
        let names: Vec<String> = ["Grays", "Pastel", "Pure", "Dark"].iter().map(|s| s.to_string()).collect();
        let mut st = PresetUi::default();
        assert_eq!(st.display_order("swatches", &names), vec![0, 1, 2, 3]);
        st.move_group("swatches", &names, "Pastel", None);
        assert_eq!(st.group_order.get("swatches").unwrap(), &["Grays", "Pure", "Dark", "Pastel"]);
        st.move_group("swatches", &names, "Pastel", Some("Grays"));
        assert_eq!(st.group_order.get("swatches").unwrap(), &["Pastel", "Grays", "Pure", "Dark"]);
        assert_eq!(st.display_order("swatches", &names), vec![1, 0, 2, 3]);
        // Back to the library's own order: no entry, as the dock stores its default tab order.
        st.move_group("swatches", &names, "Pastel", Some("Pure"));
        assert!(!st.group_order.contains_key("swatches"), "a no-op order is not stored");
        // Out-of-range names, moves onto themselves and unknown `before` names change nothing bad.
        for (from, before) in [("Nope", None), ("Grays", Some("Grays"))] {
            let mut m = PresetUi::default();
            m.move_group("swatches", &names, from, before);
            assert!(m.group_order.is_empty(), "{from} {before:?}");
            assert_eq!(m.display_order("swatches", &names), vec![0, 1, 2, 3]);
        }
        // A missing group in a saved order follows the rest in library order.
        let mut m = PresetUi::default();
        m.group_order.insert("swatches".into(), vec!["Dark".into(), "Grays".into()]);
        assert_eq!(m.display_order("swatches", &names), vec![3, 0, 1, 2]);
    }

    #[test]
    fn short_preset_library_keeps_the_allocated_browser_height() {
        use egui_kittest::Harness;

        let mut st = PresetUi::default();
        let groups = Vec::<GroupView>::new();
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut adapter = BrowserAdapter {
            panel: "short-library",
            groups: &groups,
            options: BrowserOptions { canvas: Rect::NOTHING, max_h: 180.0, flush: false },
            capabilities: BrowserCapabilities::default(),
            item_context_menu: Some(standard_item_context_menu),
            group_context_menu: Some(standard_group_context_menu),
            paint_group: None,
            paint_item: &mut |_, _, _, _| {},
        };
        let mut h = Harness::builder().with_size(vec2(320.0, 300.0)).build_ui_state(
            |ui, _app: &mut PhotocraftApp| {
                picker_shell(ui, &mut st, &mut adapter, ShellOptions { footer: Some(FooterOptions::standard("Create preset")), ..Default::default() });
            },
            app,
        );
        h.run_steps(2);
        let rect = last_browser_rect(&h.ctx, "short-library");
        assert!(rect.is_some(), "browser viewport is recorded");
        assert!(rect.unwrap().height() >= 175.0, "short libraries should retain their allocated height: {rect:?}");
    }

    #[test]
    fn short_picker_keeps_the_footer_visible_while_renaming() {
        use std::{cell::Cell, rc::Rc};

        use egui_kittest::Harness;

        let mut st = PresetUi { renaming: Some(("short-rename".into(), "preset-key".into(), "Preset name".into())), ..Default::default() };
        let groups = Vec::<GroupView>::new();
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut adapter = BrowserAdapter {
            panel: "short-rename",
            groups: &groups,
            options: BrowserOptions { canvas: Rect::NOTHING, max_h: 180.0, flush: false },
            capabilities: BrowserCapabilities::default(),
            item_context_menu: Some(standard_item_context_menu),
            group_context_menu: Some(standard_group_context_menu),
            paint_group: None,
            paint_item: &mut |_, _, _, _| {},
        };
        let content_bottom = Rc::new(Cell::new(0.0_f32));
        let measured_bottom = Rc::clone(&content_bottom);
        let mut h = Harness::builder().with_size(vec2(320.0, 180.0)).build_ui_state(
            |ui, _app: &mut PhotocraftApp| {
                picker_shell(ui, &mut st, &mut adapter, ShellOptions { footer: Some(FooterOptions::standard("Create preset")), ..Default::default() });
                measured_bottom.set(ui.min_rect().bottom());
            },
            app,
        );
        h.run_steps(3);
        assert!(content_bottom.get() <= 180.0, "footer should stay inside the short panel: content bottom {}", content_bottom.get());
    }

    #[test]
    fn custom_picker_shell_renders_search_and_passes_remaining_height_to_renderer() {
        use std::{cell::Cell, rc::Rc};

        use egui_kittest::Harness;

        let calls = Rc::new(Cell::new(0));
        let observed_height = Rc::new(Cell::new(0.0_f32));
        let call_count = Rc::clone(&calls);
        let height = Rc::clone(&observed_height);
        let mut search = move |ui: &mut egui::Ui| {
            call_count.set(call_count.get() + 1);
            ui.label("Brush search");
        };
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut h = Harness::builder().with_size(vec2(320.0, 300.0)).build_ui_state(
            |ui, _app: &mut PhotocraftApp| {
                picker_shell_custom(ui, 260.0, ShellOptions { search: Some(&mut search), ..Default::default() }, |ui, allocated_h| {
                    height.set(allocated_h);
                    ui.allocate_exact_size(vec2(ui.available_width(), allocated_h.min(50.0)), Sense::hover());
                });
            },
            app,
        );
        h.run_steps(2);
        assert!(calls.get() > 0, "the shared shell renders enabled search controls");
        assert!(observed_height.get() > 100.0, "the custom renderer receives the space remaining after shell controls");
        assert!(observed_height.get() <= 260.0, "the custom renderer respects its consumer height cap");
    }

    #[test]
    fn picker_shell_controls_and_browser_capabilities_are_optional() {
        use std::{
            cell::{Cell, RefCell},
            rc::Rc,
        };

        use egui_kittest::{Harness, kittest::Queryable};

        let groups = vec![GroupView { name: "Only group".into(), items: vec![ItemView { key: "only".into(), name: "Only item".into() }] }];
        let mut st = PresetUi::default();
        let menu_calls = Rc::new(Cell::new(0));
        let search_calls = Rc::new(Cell::new(0));
        let recent_calls = Rc::new(Cell::new(0));
        let menu_count = Rc::clone(&menu_calls);
        let search_count = Rc::clone(&search_calls);
        let recent_count = Rc::clone(&recent_calls);
        let mut menu = move |ui: &mut egui::Ui| {
            menu_count.set(menu_count.get() + 1);
            ui.label("Panel menu");
        };
        let mut search = move |ui: &mut egui::Ui| {
            search_count.set(search_count.get() + 1);
            ui.label("Search");
        };
        let mut recents = move |ui: &mut egui::Ui| {
            recent_count.set(recent_count.get() + 1);
            ui.label("Recent");
        };
        let events = Rc::new(RefCell::new(Vec::<Ev>::new()));
        let captured_events = Rc::clone(&events);
        let mut adapter = BrowserAdapter {
            panel: "compact-library",
            groups: &groups,
            options: BrowserOptions { canvas: Rect::NOTHING, max_h: 260.0, flush: false },
            capabilities: BrowserCapabilities { item_drag: false, group_drag: false, selection: false, activation: false },
            item_context_menu: None,
            group_context_menu: None,
            paint_group: None,
            paint_item: &mut |_, _, _, _| {},
        };
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut h = Harness::builder().with_size(vec2(320.0, 300.0)).build_ui_state(
            |ui, _app: &mut PhotocraftApp| {
                let emitted = picker_shell(
                    ui,
                    &mut st,
                    &mut adapter,
                    ShellOptions { panel_menu: Some(&mut menu), search: Some(&mut search), recents: Some(&mut recents), footer: None, ..Default::default() },
                );
                captured_events.borrow_mut().extend(emitted);
            },
            app,
        );
        h.run_steps(3);
        assert!(menu_calls.get() > 0 && search_calls.get() > 0 && recent_calls.get() > 0, "enabled shell slots are rendered");
        let rect = last_browser_rect(&h.ctx, "compact-library").expect("browser viewport is recorded without a footer");
        assert!(rect.height() > 100.0, "the browser receives the space left by optional controls: {rect:?}");
        h.get_by_label("Only item").click();
        h.run_steps(2);
        assert!(events.borrow().is_empty(), "disabled browser capabilities do not emit consumer actions");
    }

    #[test]
    fn shared_hierarchy_preserves_order_and_bounds_nested_paths() {
        #[derive(Debug, PartialEq)]
        struct Item(&'static str);
        let a = Item("a");
        let b = Item("b");
        let c = Item("c");
        let d = Item("d");
        let path = |parts: &[&str]| parts.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let folder_path = path(&["Folder"]);
        let nested_path = path(&["Folder", "Nested"]);
        let tree = hierarchy_tree(vec![(&folder_path, &a), (&[], &b), (&nested_path, &c), (&folder_path, &d)], 4);
        assert!(matches!(tree.first(), Some(HierarchyNode::Folder(f)) if f.path == ["Folder"] && f.count == 3));
        assert!(matches!(tree.get(1), Some(HierarchyNode::Preset(item)) if item.0 == "b"));
        let Some(HierarchyNode::Folder(folder)) = tree.first() else { panic!("folder node") };
        assert!(matches!(folder.children.first(), Some(HierarchyNode::Preset(item)) if item.0 == "a"));
        assert!(matches!(folder.children.get(1), Some(HierarchyNode::Folder(nested)) if nested.path == ["Folder", "Nested"]));
        assert!(matches!(folder.children.get(2), Some(HierarchyNode::Preset(item)) if item.0 == "d"));

        let deep_path = path(&["Folder", "Deep", "TooDeep"]);
        let bounded = hierarchy_tree(vec![(&nested_path, &c), (&deep_path, &d)], 1);
        assert!(matches!(bounded.first(), Some(HierarchyNode::Folder(f)) if f.path == ["Folder"] && f.children.len() == 2));
    }

    #[test]
    fn compact_patterns_browser_selects_a_library_identity() {
        use egui_kittest::{Harness, kittest::Queryable};

        let (app, _) = app();
        let patterns = app.session.patterns.items.clone();
        let groups = pattern_group_views(&patterns, &app.session.presets.pattern_groups);
        let first = groups.iter().find_map(|g| g.items.first()).expect("built-in Patterns group has an item");
        let (id, name) = (first.key.clone(), first.name.clone());
        let mut h = Harness::builder().with_size(vec2(320.0, 240.0)).build_ui_state(
            move |ui, picked: &mut Option<String>| {
                if let Some(id) = compact_pattern_browser(ui, &patterns, &groups, "", "compact-test", 180.0) {
                    *picked = Some(id);
                }
            },
            None::<String>,
        );
        h.run_steps(3);
        h.get_by_label(&name).click();
        h.run_steps(2);
        assert_eq!(h.state().as_deref(), Some(id.as_str()));
        assert!(last_browser_rect(&h.ctx, "compact-test").is_some(), "compact consumer uses the shared browser viewport");
    }

    #[test]
    fn compact_patterns_browser_preserves_folder_disclosure_between_frames() {
        use egui_kittest::Harness;

        let (app, _) = app();
        let patterns = app.session.patterns.items.clone();
        let groups = pattern_group_views(&patterns, &app.session.presets.pattern_groups);
        assert!(groups.len() > 1, "the fixture needs a second group to toggle");
        let folder = groups[1].name.clone();
        let panel = "compact-disclosure-test";
        let state_id = egui::Id::new(("compact-pattern-browser", panel));
        let mut h = Harness::builder().with_size(vec2(320.0, 240.0)).build_ui_state(
            move |ui, _picked: &mut Option<String>| {
                let _ = compact_pattern_browser(ui, &patterns, &groups, "", panel, 180.0);
            },
            None::<String>,
        );

        h.run_steps(3);
        assert!(
            h.ctx.data(|d| d.get_temp::<PresetUi>(state_id).is_some_and(|s| s.toggled.contains(&format!("{panel}/{folder}")))),
            "later folders start collapsed"
        );

        let header =
            last_group_headers(&h.ctx, panel).iter().find(|(name, _)| name == &folder).map(|(_, rect)| rect.center()).expect("second group header is visible");
        h.event(egui::Event::PointerMoved(header));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: header, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.event(egui::Event::PointerButton { pos: header, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(2);

        let key = format!("{panel}/{folder}");
        assert!(!h.ctx.data(|d| d.get_temp::<PresetUi>(state_id).is_some_and(|s| s.toggled.contains(&key))), "clicking the folder expands it");
        h.run_steps(2);
        assert!(!h.ctx.data(|d| d.get_temp::<PresetUi>(state_id).is_some_and(|s| s.toggled.contains(&key))), "the next frame must retain the expanded state");
    }

    #[test]
    fn patterns_panel_dispatches_selection_to_the_patterns_command() {
        use egui_kittest::{Harness, kittest::Queryable};

        let (app, _) = app();
        let first = pattern_group_views(&app.session.patterns.items, &app.session.presets.pattern_groups)
            .iter()
            .find_map(|g| g.items.first().map(|p| (p.key.clone(), p.name.clone())))
            .expect("a built-in Patterns group");
        let (id, name) = first;
        let mut h = Harness::builder().with_size(vec2(340.0, 360.0)).build_ui_state(|ui, app: &mut PhotocraftApp| patterns_panel(app, ui), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(4);
        h.get_by_label(&name).click();
        h.run_steps(3);
        assert_eq!(h.state().session.presets.pattern.as_deref(), Some(id.as_str()));
        assert_eq!(h.state().session.journal.last().map(|(id, _)| id.as_str()), Some("pattern.presets.select"));
    }

    fn consumer_item_menu(ui: &mut egui::Ui, _st: &mut PresetUi, _panel: &str, item: &ItemView, _group: &GroupView, _names: &[String], ev: &mut Vec<Ev>) {
        if ui.button("Consumer item operation").clicked() {
            ev.push(Ev::Delete(item.key.clone()));
            ui.close();
        }
    }

    fn consumer_group_menu(ui: &mut egui::Ui, _st: &mut PresetUi, _panel: &str, group: &GroupView, ev: &mut Vec<Ev>) {
        if ui.button("Consumer group operation").clicked() {
            ev.push(Ev::DeleteGroup(group.name.clone()));
            ui.close();
        }
    }

    #[test]
    fn browser_keeps_context_hit_targets_but_delegates_menu_actions() {
        use egui_kittest::{Harness, kittest::Queryable};

        let groups = vec![GroupView { name: "Only group".into(), items: vec![ItemView { key: "only".into(), name: "Only item".into() }] }];
        let mut st = PresetUi::default();
        let mut adapter = BrowserAdapter {
            panel: "consumer-actions",
            groups: &groups,
            options: BrowserOptions { canvas: Rect::NOTHING, max_h: 180.0, flush: false },
            capabilities: BrowserCapabilities::default(),
            item_context_menu: Some(consumer_item_menu),
            group_context_menu: Some(consumer_group_menu),
            paint_group: None,
            paint_item: &mut |_, _, _, _| {},
        };
        let mut h = Harness::builder()
            .with_size(vec2(320.0, 240.0))
            .build_ui_state(|ui, events: &mut Vec<Ev>| events.extend(browser(ui, &mut st, &mut adapter, 180.0)), Vec::<Ev>::new());
        h.run_steps(3);
        h.get_by_label("Only item").click_secondary();
        h.run_steps(2);
        h.get_by_label("Consumer item operation").click();
        h.run_steps(2);
        assert!(matches!(h.state().last(), Some(Ev::Delete(key)) if key == "only"));

        let header = last_group_headers(&h.ctx, "consumer-actions").first().map(|(_, r)| r.center()).expect("group hit target recorded");
        h.event(egui::Event::PointerMoved(header));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: header, button: egui::PointerButton::Secondary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: header, button: egui::PointerButton::Secondary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(2);
        h.get_by_label("Consumer group operation").click();
        h.run_steps(2);
        assert!(matches!(h.state().last(), Some(Ev::DeleteGroup(group)) if group == "Only group"));
    }

    /// #3658: the folder order persists with the UI state and reads leniently (an old state has no
    /// field, and garbage in one never fails the whole state).
    #[test]
    fn folder_order_persists_and_reads_leniently() {
        let old: PresetUi = serde_json::from_value(json!({"sizes": {"swatches": 22.0}})).unwrap();
        assert!(old.group_order.is_empty(), "an older state without the field still loads");
        let mut st = PresetUi::default();
        st.group_order.insert("swatches".into(), vec!["Dark".into(), "Grays".into()]);
        let back: PresetUi = serde_json::from_value(serde_json::to_value(&st).unwrap()).unwrap();
        assert_eq!(back.group_order.get("swatches").unwrap(), &["Dark", "Grays"]);
        for bad in [json!("x"), json!(7), json!({"swatches": "paths"}), json!({"swatches": [1, null, {"a": 1}]})] {
            let back: PresetUi = serde_json::from_value(json!({"groupOrder": bad})).unwrap();
            assert!(back.group_order.get("swatches").is_none_or(Vec::is_empty), "{bad}");
        }
    }

    /// Regression: preset drops mapped the pointer through the raw widget rect, ignoring the
    /// rulers' 16 px inset — a dropped shape landed 8 screen px from the pointer, and a drop on
    /// the ruler strip counted as a canvas drop. Drops now map through, and target, the same
    /// content rect the canvas draws with.
    #[test]
    fn preset_drops_map_through_the_ruler_inset() {
        let (mut app, _) = app();
        app.last_canvas_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        app.ui.views[0] = crate::state::View { zoom: 2.0, center: [32.0, 24.0], fit_pending: false, fill_pending: false, doc_size: [64, 48], rotation: 0.0 };
        let p = egui::pos2(400.0, 300.0);
        let off = crate::canvas::doc_point_at(&app, p).unwrap();
        assert_eq!(off, [32.0, 24.0], "without rulers the widget rect is the whole mapping");
        // With rulers the content rect shrinks by the inset on the left and top: the same screen
        // point is 8 screen px (4 document px at zoom 2) up-left of where it was.
        app.ui.extras.rulers = true;
        let on = crate::canvas::doc_point_at(&app, p).unwrap();
        assert!((on[0] - (off[0] - 4.0)).abs() < 1e-3, "{on:?} vs {off:?}");
        assert!((on[1] - (off[1] - 4.0)).abs() < 1e-3, "{on:?} vs {off:?}");
        // The ruler strip is no longer a drop target; the canvas proper still is.
        assert!(!canvas_drop_rect(&app).contains(egui::pos2(5.0, 300.0)));
        assert!(canvas_drop_rect(&app).contains(p));
    }

    #[test]
    fn clone_marker_follows_the_engine_slot() {
        let (mut app, _) = app();
        assert_eq!(clone_sample_point(&app, Some([5.0, 5.0])), None);
        app.run("cloneSource.set", json!({"source": [10, 10]})).unwrap();
        assert_eq!(clone_sample_point(&app, Some([30.0, 30.0])), Some([10.0, 10.0]));
        app.drag = Some(crate::canvas::Drag::new(Tool::CloneStamp, [40.0, 40.0], vec![[50.0, 30.0, 1.0]], egui::Modifiers::NONE, false));
        assert_eq!(clone_sample_point(&app, Some([50.0, 30.0])), Some([20.0, 0.0]));
        app.drag = None;
        app.session.presets.clone.active_mut().anchor = Some([20.0, 20.0]);
        assert_eq!(clone_sample_point(&app, Some([30.0, 30.0])), Some([20.0, 20.0]));
    }

    #[test]
    fn clone_marker_shows_only_while_painting() {
        // #668: ⌥-click used to leave a "+" on the canvas until the next stroke.
        let (mut app, _) = app();
        app.ui.tool = Tool::CloneStamp;
        crate::retouch_ui::set_source(&mut app, 10.0, 10.0);
        app.hover_doc = Some([40.0, 40.0]);
        assert_eq!(crate::retouch_ui::source_marker_point(&app), None, "hovering after ⌥-click shows nothing");
        app.drag = Some(crate::canvas::Drag::new(Tool::CloneStamp, [40.0, 40.0], vec![[40.0, 40.0, 1.0]], egui::Modifiers::NONE, false));
        assert!(crate::retouch_ui::source_marker_point(&app).is_some(), "painting shows where it samples");
        app.drag = None;
        app.ui.tool = Tool::Brush;
        assert_eq!(crate::retouch_ui::source_marker_point(&app), None);
    }
}
