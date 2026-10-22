//! Move tool Auto-Select: pick the topmost visible, not fully locked layer with pixels at a canvas
//! point (Photoshop's options bar "Auto-Select: Layer | Group", ⌘-click with the Move tool, and the
//! canvas right-click layer list). Like Photoshop, Auto-Select hits a type layer anywhere inside its
//! text's bounds, so a click between letters picks the type rather than the layer under it (#2370).
//!
//! In Group mode a click drills into groups, as in Photoshop (#7914): the first click on a group
//! selects the group, a second click on the same spot selects the layer inside it, and further
//! clicks go one level deeper still, down to the pixels. The drill level is read off the last
//! canvas click, so it needs no state of its own beyond that click.

use photocraft_doc::{Document, LayerContent, LayerId};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document".into())
}

/// Layers with visible pixels at (x, y), topmost first (hidden layers and hidden groups skipped).
pub fn layers_at(doc: &Document, x: i32, y: i32) -> Vec<LayerId> {
    layers_hit(doc, x, y, false)
}

/// [`layers_at`], but with `type_bounds` a type layer is hit anywhere inside the bounds of its
/// rendered text (the gaps between glyphs included), as Photoshop's Auto-Select does.
fn layers_hit(doc: &Document, x: i32, y: i32, type_bounds: bool) -> Vec<LayerId> {
    // A 1×1 read at i32::MAX would be an empty rect (huge `x` params saturate there).
    if x.checked_add(1).is_none() || y.checked_add(1).is_none() {
        return Vec::new();
    }
    let rows = doc.walk();
    let mut out = Vec::new();
    for (path, _, l) in rows.iter().rev() {
        // The layer and every enclosing group must be visible.
        if !(1..=path.len()).all(|n| doc.layer_at(&path[..n]).is_some_and(|a| a.visible)) {
            continue;
        }
        // An artboard clips its layers to the board (#1531): pixels past its edge aren't shown,
        // so they can't be picked. The board itself is hit anywhere on it, below its layers
        // (the walk lists them first), as a Photoshop click on an empty spot selects the board.
        let board = path.first().and_then(|i| doc.layers.get(*i)).and_then(|t| t.artboard());
        if board.is_some_and(|a| !a.rect.contains(x, y)) {
            continue;
        }
        if l.artboard().is_some() {
            out.push(l.id);
            continue;
        }
        if matches!(l.content, LayerContent::Group(_) | LayerContent::Adjustment(_)) {
            continue;
        }
        let alpha = match &l.content {
            // Fill layers cover the canvas (their mask limits them).
            LayerContent::Fill(_) => 1.0,
            // The text's pixel bounds (in document space, so they follow the layer's transform).
            LayerContent::Text(t) if type_bounds && t.cache.as_ref().is_some_and(|c| c.content_bounds().contains(x, y)) => 1.0,
            _ => match l.surface() {
                Some(s) => {
                    let mut px = [[0.0f32; 4]; 1];
                    s.read_rgba_into(Rect::from_xywh(x, y, 1, 1), &mut px);
                    px[0][3]
                }
                None => 0.0,
            },
        };
        // As the compositor sees it: density applied, all ones when the mask is disabled.
        let mask = l.mask.as_ref().map_or(1.0, |m| m.value(x, y));
        if alpha * mask > 0.0 {
            out.push(l.id);
        }
    }
    out
}

/// The chain of layers a click drills through to reach `id`: the enclosing groups, outermost
/// first, then the layer itself. An artboard is not a group here (Group mode stops at the
/// outermost group on the board), so it is left out.
fn group_chain(doc: &Document, id: LayerId) -> Vec<LayerId> {
    let Some((path, _, _)) = doc.walk().into_iter().find(|(_, _, l)| l.id == id) else { return vec![id] };
    let mut chain = Vec::new();
    for n in 1..path.len() {
        if let Some(a) = doc.layer_at(&path[..n])
            && a.is_group()
            && a.artboard().is_none()
        {
            chain.push(a.id);
        }
    }
    chain.push(id);
    chain
}

/// The outermost group containing `id` (the layer itself when it isn't in a group). An artboard
/// is not a group here: Group mode stops at the outermost group on the board.
fn top_group(doc: &Document, id: LayerId) -> LayerId {
    group_chain(doc, id).first().copied().unwrap_or(id)
}

/// The layer a click picks in Group mode, drilling into the group the last click picked, as
/// Photoshop does (#7914): the first click on a group selects the group, a second click on the
/// same group selects the layer inside it, and another click goes one level deeper still, down to
/// the pixels.
///
/// `drilled` is the layer the previous click picked. A click on anything else starts again at
/// the outermost group, so a click never reaches past the group the user entered.
fn drill_target(doc: &Document, hit: LayerId, drilled: Option<LayerId>) -> LayerId {
    let chain = group_chain(doc, hit);
    let Some(p) = drilled else { return chain[0] };
    let Some(i) = chain.iter().position(|&c| c == p) else { return chain[0] };
    // One level deeper than the group we are in; the pixels themselves are the end of the road.
    chain.get(i + 1).copied().unwrap_or(hit)
}

/// The layer an Auto-Select click at (x, y) picks (with `group`, its outermost group, or one
/// level deeper when `drilled` says the user has already entered a group): the topmost hit, a
/// type layer by its text bounds. Like Photoshop, it clicks through a fully locked layer (its own
/// Lock All or a locked group's) to the layer under it (#1641); the right-click list still shows
/// it. The click (`layer.pickAt`) and the Move tool's rollover highlight both ask here, so the
/// outline always names the layer a click takes.
pub fn auto_select_target(doc: &Document, x: i32, y: i32, group: bool, drilled: Option<LayerId>) -> Option<LayerId> {
    let hit = layers_hit(doc, x, y, true).into_iter().find(|id| !doc.effective_locks(*id).all)?;
    Some(if group { drill_target(doc, hit, drilled) } else { hit })
}

fn pick(s: &mut Session, p: &Value) -> Result<Value> {
    let x = p.get("x").and_then(Value::as_f64).ok_or_else(|| EngineError::BadParams { cmd: "layer.pickAt".into(), msg: "missing `x`".into() })?.floor() as i32;
    let y = p.get("y").and_then(Value::as_f64).ok_or_else(|| EngineError::BadParams { cmd: "layer.pickAt".into(), msg: "missing `y`".into() })?.floor() as i32;
    let group = p.get("target").and_then(Value::as_str) == Some("group");
    let mode = p.get("mode").and_then(Value::as_str).unwrap_or("replace");
    let select = p.get("select").and_then(Value::as_bool).unwrap_or(true);
    // How deep the click reaches: the group the last canvas click picked, as long as nothing has
    // changed since (a selection change or an edit moves the revision on) (#7914). Only a
    // Group-mode click drills, so a Layer-mode click ends the drill. A ⇧-click adds to the
    // selection, so it never drills.
    let (doc, drilled) = {
        let st = s.active().ok_or(EngineError::NoDocument)?;
        let drilled = group.then(|| st.last_pick.filter(|(_, rev)| *rev == st.revision).map(|(id, _)| id)).flatten();
        (st.doc.clone(), drilled)
    };
    // The right-click list includes type layers between glyphs, like Auto-Select.
    if p.get("list").and_then(Value::as_bool).unwrap_or(false) {
        let hits = layers_hit(&doc, x, y, true);
        let names: Vec<Value> = hits.iter().filter_map(|id| doc.layer(*id)).map(|l| json!({"layer": l.id.0, "name": l.name})).collect();
        return Ok(json!({ "layers": names }));
    }
    let Some(target) = auto_select_target(&doc, x, y, group, drilled) else {
        // A click that hit no layer ends the drill: the next click starts at the group again.
        // The probe the Move tool makes before a press counts too: the press is the click.
        if let Some(st) = s.active_mut() {
            st.last_pick = None;
        }
        return Ok(json!({ "layer": null }));
    };
    if select {
        // Like Photoshop, a plain click on one of several selected layers keeps them all
        // selected, so the drag that follows moves the whole selection.
        let keeps = mode == "replace" && s.active().is_some_and(|st| st.is_layer_selected(target) && st.selected_layers().len() > 1);
        if !keeps {
            s.execute("layer.select", json!({"layer": target.0, "mode": mode}))?;
        }
        // Remember the pick, so the next click on the same group drills into it. Only a
        // Group-mode click drills, and one that kept a multi-selection did not enter a group.
        if mode == "replace"
            && let Some(st) = s.active_mut()
        {
            st.last_pick = (group && !keeps).then_some((target, st.revision));
        }
    }
    Ok(json!({ "layer": target.0 }))
}

/// Layers whose content touches document rect `r`, top to bottom: visible, not fully locked, not
/// the locked Background (as Select › All Layers). A layer on an artboard counts only where the
/// board shows it. With `group`, each counts as its outermost group (deduplicated).
pub fn layers_in_rect(doc: &Document, r: Rect, group: bool) -> Vec<LayerId> {
    let mut out = Vec::new();
    if r.is_empty() {
        return out;
    }
    let canvas = doc.bounds();
    for (path, _, l) in doc.walk().iter().rev() {
        if matches!(l.content, LayerContent::Group(_) | LayerContent::Adjustment(_))
            || doc.is_background(l.id)
            || doc.effective_locks(l.id).all
            || !(1..=path.len()).all(|n| doc.layer_at(&path[..n]).is_some_and(|a| a.visible))
        {
            continue;
        }
        let mut b = photocraft_compose::layer_bounds(l, canvas);
        if let Some(board) = path.first().and_then(|i| doc.layers.get(*i)).and_then(|t| t.artboard()) {
            b = b.intersect(&board.rect);
        }
        if b.is_empty() || b.intersect(&r).is_empty() {
            continue;
        }
        let id = if group { top_group(doc, l.id) } else { l.id };
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// `layer.selectInRect`: the Move tool's marquee with Auto-Select, dragged from an empty spot.
fn select_in_rect(s: &mut Session, p: &Value) -> Result<Value> {
    let num = |k: &str| {
        p.get(k)
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
            .ok_or_else(|| EngineError::BadParams { cmd: "layer.selectInRect".into(), msg: format!("missing or invalid `{k}`") })
    };
    let (x, y, w, h) = (num("x")?, num("y")?, num("width")?, num("height")?);
    let px = |v: f64| v.clamp(f64::from(i32::MIN / 2), f64::from(i32::MAX / 2)) as i32;
    // A rect dragged up or left has a negative size.
    let r = Rect::new(px(x.min(x + w).floor()), px(y.min(y + h).floor()), px(x.max(x + w).ceil()), px(y.max(y + h).ceil()));
    let doc = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let group = p.get("target").and_then(Value::as_str) == Some("group");
    let hits = layers_in_rect(&doc, r, group);
    let add = match p.get("mode").and_then(Value::as_str).unwrap_or("replace") {
        "replace" => false,
        "add" => true,
        other => return Err(EngineError::BadParams { cmd: "layer.selectInRect".into(), msg: format!("unknown mode `{other}`") }),
    };
    // Nothing touched leaves the layer selection as it is.
    if !hits.is_empty() {
        let mut ids = if add { s.active().map(|st| st.selected_layers()).unwrap_or_default() } else { Vec::new() };
        ids.extend(hits.iter().filter(|id| !ids.contains(id)).copied().collect::<Vec<_>>());
        crate::layer_multi_cmds::set_selection(s, ids, hits.first().copied(), hits.first().copied())?;
    }
    Ok(json!({ "layers": hits.iter().map(|id| id.0).collect::<Vec<_>>() }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "layer.selectInRect",
            label: "Select Layers in Marquee",
            menu: &[],
            shortcut: None,
            params: r##"{"x":px,"y":px,"width":px,"height":px,"target":"layer|group"="layer","mode":"replace|add"="replace"} → {layers} (the layers whose content touches the rect, topmost first; none leaves the selection as it is)"##,
            enabled: has_doc,
            journal: true,
            run: select_in_rect,
        },
        CommandSpec {
            id: "layer.pickAt",
            label: "Auto-Select Layer",
            menu: &[],
            shortcut: None,
            params: r##"{"x":px,"y":px,"target":"layer|group"="layer","select":bool=true,"mode":"replace|toggle|add"="replace","list":bool=false (return visible layers at the point, including text bounds, topmost first)} → {layer} | {layers} (in Group mode a click drills into the groups: the first click takes the group, the next the layer inside it)"##,
            enabled: has_doc,
            journal: false,
            run: pick,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_topmost_layer_with_pixels() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 40})).unwrap();
        let bg = s.active().unwrap().doc.layers[0].id;
        s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let a = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.new.layer", json!({"name": "B"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        // Empty B is skipped; A wins inside its square, the Background elsewhere.
        assert_eq!(s.execute("layer.pickAt", json!({"x": 5, "y": 5})).unwrap()["layer"], a.0);
        assert_eq!(s.active().unwrap().active_layer, Some(a));
        assert_eq!(s.execute("layer.pickAt", json!({"x": 30, "y": 30, "select": false})).unwrap()["layer"], bg.0);
        let list = s.execute("layer.pickAt", json!({"x": 5, "y": 5, "list": true})).unwrap();
        assert_eq!(list["layers"].as_array().unwrap().len(), 2);
        // Hidden layers are ignored.
        s.execute("layer.setProps", json!({"layer": a.0, "visible": false})).unwrap();
        assert_eq!(s.execute("layer.pickAt", json!({"x": 5, "y": 5, "select": false})).unwrap()["layer"], bg.0);
    }

    /// Two filled squares: A at (0..10), B at (20..30), both in their own layers.
    fn two_squares() -> (Session, LayerId, LayerId) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 40})).unwrap();
        let mut ids = Vec::new();
        for (name, x) in [("A", 0), ("B", 20)] {
            s.execute("layer.new.layer", json!({"name": name})).unwrap();
            s.execute("select.rect", json!({"x": x, "y": x, "width": 10, "height": 10})).unwrap();
            s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
            ids.push(s.active().unwrap().active_layer.unwrap());
        }
        s.execute("select.deselect", json!({})).unwrap();
        (s, ids[0], ids[1])
    }

    #[test]
    fn group_target_selects_the_outermost_group() {
        let (mut s, a, _) = two_squares();
        s.execute("layer.select", json!({"layer": a.0})).unwrap();
        let inner = s.execute("layer.new.groupFromLayers", json!({"name": "Inner"})).unwrap()["layer"].as_u64().unwrap();
        let outer = s.execute("layer.new.groupFromLayers", json!({"name": "Outer"})).unwrap()["layer"].as_u64().unwrap();
        assert_ne!(inner, outer);
        // Selecting the group in the Layers panel is not a canvas click, so the click still takes
        // the outermost group (#7914).
        assert_eq!(s.execute("layer.pickAt", json!({"x": 5, "y": 5, "target": "group"})).unwrap()["layer"], outer);
        assert_eq!(s.active().unwrap().active_layer, Some(LayerId(outer)));
        // Layer mode reaches through the groups to the pixels.
        assert_eq!(s.execute("layer.pickAt", json!({"x": 5, "y": 5, "target": "layer"})).unwrap()["layer"], a.0);
        // Hiding the outer group hides its layers from the pick.
        s.execute("layer.setProps", json!({"layer": outer, "visible": false})).unwrap();
        let bg = s.active().unwrap().doc.layers[0].id;
        assert_eq!(s.execute("layer.pickAt", json!({"x": 5, "y": 5, "select": false})).unwrap()["layer"], bg.0);
    }

    /// A layer with pixels at (0..10)², wrapped in two groups: "Inner" inside "Outer".
    fn nested_groups() -> (Session, LayerId, LayerId, LayerId) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 40})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Leaf"})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let leaf = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.select", json!({"layer": leaf.0})).unwrap();
        let inner = LayerId(s.execute("layer.new.groupFromLayers", json!({"name": "Inner"})).unwrap()["layer"].as_u64().unwrap());
        let outer = LayerId(s.execute("layer.new.groupFromLayers", json!({"name": "Outer"})).unwrap()["layer"].as_u64().unwrap());
        (s, inner, outer, leaf)
    }

    #[test]
    fn a_click_drills_into_the_groups_it_hits() {
        // #7914: a click on grouped content took the group and never the layer inside it.
        let (mut s, inner, outer, leaf) = nested_groups();
        let pick = |s: &mut Session| s.execute("layer.pickAt", json!({"x": 5, "y": 5, "target": "group"})).unwrap()["layer"].clone();
        // The first click takes the outermost group.
        assert_eq!(pick(&mut s), outer.0);
        assert_eq!(s.active().unwrap().active_layer, Some(outer));
        // The next click on the same spot takes the group inside it.
        assert_eq!(pick(&mut s), inner.0);
        assert_eq!(s.active().unwrap().active_layer, Some(inner));
        // And the next one the pixels themselves.
        assert_eq!(pick(&mut s), leaf.0);
        assert_eq!(s.active().unwrap().active_layer, Some(leaf));
        // Clicking again keeps the layer: there is nothing deeper.
        assert_eq!(pick(&mut s), leaf.0);
        assert_eq!(s.active().unwrap().active_layer, Some(leaf));
    }

    #[test]
    fn a_click_on_another_group_starts_at_that_group() {
        let (mut s, inner, outer, _) = nested_groups();
        // A second layer with pixels of its own, in a group of its own.
        s.execute("layer.new.layer", json!({"name": "Other"})).unwrap();
        s.execute("select.rect", json!({"x": 20, "y": 0, "width": 10, "height": 10})).unwrap();
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let other = s.active().unwrap().active_layer.unwrap();
        let other_group = LayerId(s.execute("layer.new.groupFromLayers", json!({"name": "Other G"})).unwrap()["layer"].as_u64().unwrap());
        let pick = |s: &mut Session, x: i32| s.execute("layer.pickAt", json!({"x": x, "y": 5, "target": "group"})).unwrap()["layer"].clone();
        // The first click on the other group takes that group, not a layer inside it.
        assert_eq!(pick(&mut s, 25), other_group.0);
        // The next click drills into it, not into the group we were in before.
        assert_eq!(pick(&mut s, 25), other.0);
        // A click on the first group starts at its outermost group again: the group we left is
        // not remembered, so the click does not reach the layer inside it.
        assert_eq!(pick(&mut s, 5), outer.0);
        // From there the drill works again, one level per click.
        assert_eq!(pick(&mut s, 5), inner.0);
    }

    #[test]
    fn a_selection_change_ends_the_drill() {
        // The drill follows the clicks on the canvas: picking a layer in the Layers panel is
        // not a click, so it starts the next click at the group again.
        let (mut s, inner, outer, leaf) = nested_groups();
        let pick = |s: &mut Session| s.execute("layer.pickAt", json!({"x": 5, "y": 5, "target": "group"})).unwrap()["layer"].clone();
        assert_eq!(pick(&mut s), outer.0);
        s.execute("layer.select", json!({"layer": leaf.0})).unwrap();
        assert_eq!(pick(&mut s), outer.0, "the group, not the layer the panel selected");
        // The drill works again from there: the group, the group inside it, then the pixels.
        assert_eq!(pick(&mut s), inner.0);
        assert_eq!(pick(&mut s), leaf.0);
    }

    #[test]
    fn a_click_that_hits_no_layer_ends_the_drill() {
        // An empty spot is not a group, so the next click starts at the group again.
        let (mut s, inner, outer, _) = nested_groups();
        let pick = |s: &mut Session, x: i32| s.execute("layer.pickAt", json!({"x": x, "y": 5, "target": "group"})).unwrap()["layer"].clone();
        assert_eq!(pick(&mut s, 5), outer.0);
        // Past the canvas there is no layer at all: the drill ends there.
        assert_eq!(pick(&mut s, 500), Value::Null);
        assert_eq!(pick(&mut s, 5), outer.0, "the group, not the layer inside it");
        assert_eq!(pick(&mut s, 5), inner.0, "and the drill works again from there");
    }

    #[test]
    fn the_move_tools_probe_ends_the_drill_too() {
        // The Move tool probes the point before a press (`select: false`); the press is the
        // click, so a probe that hits nothing ends the drill as well.
        let (mut s, _, outer, _) = nested_groups();
        let pick = |s: &mut Session| s.execute("layer.pickAt", json!({"x": 5, "y": 5, "target": "group"})).unwrap()["layer"].clone();
        assert_eq!(pick(&mut s), outer.0);
        assert_eq!(s.execute("layer.pickAt", json!({"x": 500, "y": 5, "target": "group", "select": false})).unwrap()["layer"], Value::Null);
        assert_eq!(pick(&mut s), outer.0, "the group, not the layer inside it");
    }

    #[test]
    fn a_layer_mode_click_ends_the_drill() {
        // The drill belongs to the Group mode: a Layer-mode click names the pixels, and the
        // Group mode that follows starts at the group again.
        let (mut s, _, outer, leaf) = nested_groups();
        assert_eq!(s.execute("layer.pickAt", json!({"x": 5, "y": 5, "target": "layer"})).unwrap()["layer"], leaf.0);
        assert_eq!(s.execute("layer.pickAt", json!({"x": 5, "y": 5, "target": "group"})).unwrap()["layer"], outer.0);
    }

    #[test]
    fn adding_to_the_selection_does_not_drill_into_the_groups() {
        let (mut s, _, outer, _) = nested_groups();
        // ⇧-click adds the layer under the pointer to the selection; it does not go deeper.
        s.execute("layer.select", json!({"layer": outer.0})).unwrap();
        let picked = s.execute("layer.pickAt", json!({"x": 5, "y": 5, "target": "group", "mode": "add"})).unwrap()["layer"].clone();
        assert_eq!(picked, outer.0);
        let st = s.active().unwrap();
        assert!(st.is_layer_selected(outer) && st.selected_layers().len() == 1, "the group is still the only selected layer");
    }

    #[test]
    fn a_click_that_keeps_the_multi_selection_does_not_drill() {
        // The click kept the selection instead of entering the group, so it remembers nothing:
        // the next click takes the group again.
        let (mut s, _, outer, _) = nested_groups();
        s.execute("layer.new.layer", json!({"name": "Other"})).unwrap();
        s.execute("select.rect", json!({"x": 20, "y": 0, "width": 10, "height": 10})).unwrap();
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let other = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.select", json!({"layer": outer.0})).unwrap();
        s.execute("layer.select", json!({"layer": other.0, "mode": "add"})).unwrap();
        let pick = |s: &mut Session| s.execute("layer.pickAt", json!({"x": 5, "y": 5, "target": "group"})).unwrap()["layer"].clone();
        assert_eq!(pick(&mut s), outer.0);
        assert_eq!(s.active().unwrap().selected_layers().len(), 2, "the multi-selection is kept");
        assert_eq!(pick(&mut s), outer.0, "and the click does not drill into the group");
    }

    #[test]
    fn layer_mode_reaches_the_pixels_without_drilling() {
        let (mut s, _, _, leaf) = nested_groups();
        // Layer mode names the pixels straight away, whatever was picked before.
        s.execute("layer.select", json!({"layer": leaf.0})).unwrap();
        assert_eq!(s.execute("layer.pickAt", json!({"x": 5, "y": 5, "target": "layer"})).unwrap()["layer"], leaf.0);
        let bg = s.active().unwrap().doc.layers[0].id;
        s.execute("layer.select", json!({"layer": bg.0})).unwrap();
        assert_eq!(s.execute("layer.pickAt", json!({"x": 5, "y": 5, "target": "layer"})).unwrap()["layer"], leaf.0);
    }

    #[test]
    fn a_click_on_a_selected_layer_keeps_the_multi_selection() {
        let (mut s, a, b) = two_squares();
        s.execute("layer.select", json!({"layer": a.0})).unwrap();
        s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
        assert_eq!(s.active().unwrap().selected_layers().len(), 2);
        // Clicking A (selected) keeps both so the drag moves both.
        s.execute("layer.pickAt", json!({"x": 5, "y": 5})).unwrap();
        let st = s.active().unwrap();
        assert!(st.is_layer_selected(a) && st.is_layer_selected(b));
        // Clicking the Background (not selected) replaces the selection.
        s.execute("layer.pickAt", json!({"x": 35, "y": 5})).unwrap();
        assert_eq!(s.active().unwrap().selected_layers().len(), 1);
        assert!(!s.active().unwrap().is_layer_selected(a));
        // Shift-click (add) builds a selection up again.
        s.execute("layer.pickAt", json!({"x": 5, "y": 5})).unwrap();
        s.execute("layer.pickAt", json!({"x": 25, "y": 25, "mode": "add"})).unwrap();
        let st = s.active().unwrap();
        assert!(st.is_layer_selected(a) && st.is_layer_selected(b));
    }

    #[test]
    fn a_click_on_an_artboard_picks_the_board_or_the_layers_on_it() {
        // #1531: an empty spot on a board picked the Background under it, so a Move-tool drag
        // there never moved the board.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 40})).unwrap();
        let bg = s.active().unwrap().doc.layers[0].id;
        s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let a = s.active().unwrap().active_layer.unwrap();
        let board = s.execute("layer.new.artboardFromLayers", json!({})).unwrap()["layer"].as_u64().unwrap();
        s.execute("layer.artboard.set", json!({"width": 30, "height": 30})).unwrap();
        // A's pixels past the board's edge are clipped away, so they can't be picked either.
        s.edit("paint outside", |doc, _| {
            doc.layer_mut(a).unwrap().surface_mut().unwrap().fill_rect(Rect::new(32, 32, 36, 36), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        let pick =
            |s: &mut Session, x: i32, y: i32, target: &str| s.execute("layer.pickAt", json!({"x": x, "y": y, "target": target})).unwrap()["layer"].clone();
        assert_eq!(pick(&mut s, 5, 5, "layer"), a.0, "the layer on the board");
        assert_eq!(pick(&mut s, 5, 5, "group"), a.0, "Group mode stops inside the board");
        assert_eq!(pick(&mut s, 20, 20, "layer"), board, "an empty spot on the board picks the board");
        assert_eq!(s.active().unwrap().active_layer, Some(LayerId(board)));
        assert_eq!(pick(&mut s, 33, 33, "layer"), bg.0, "off the board: the Background, not A's clipped pixels");
        let list = s.execute("layer.pickAt", json!({"x": 5, "y": 5, "list": true})).unwrap();
        let ids: Vec<u64> = list["layers"].as_array().unwrap().iter().map(|l| l["layer"].as_u64().unwrap()).collect();
        assert_eq!(ids, vec![a.0, board, bg.0]);
        // A hidden board is skipped with its layers.
        s.execute("layer.setProps", json!({"layer": board, "visible": false})).unwrap();
        assert_eq!(pick(&mut s, 5, 5, "layer"), bg.0);
    }

    #[test]
    fn a_fully_locked_layer_is_clicked_through() {
        // #1641: a locked layer on top was picked, so the drag couldn't reach the layer under it.
        let (mut s, a, _) = two_squares();
        s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let top = s.active().unwrap().active_layer.unwrap();
        let lock = |s: &mut Session, id: u64, locks: Value| s.execute("layer.setProps", json!({"layer": id, "locks": locks})).unwrap();
        let pick = |s: &mut Session| s.execute("layer.pickAt", json!({"x": 5, "y": 5})).unwrap()["layer"].clone();
        assert_eq!(pick(&mut s), top.0);
        // A position lock alone doesn't hide it from Auto-Select.
        lock(&mut s, top.0, json!({"position": true}));
        assert_eq!(pick(&mut s), top.0);
        lock(&mut s, top.0, json!({"all": true}));
        assert_eq!(pick(&mut s), a.0);
        assert_eq!(s.active().unwrap().active_layer, Some(a));
        // The right-click layer list still names it.
        let list = s.execute("layer.pickAt", json!({"x": 5, "y": 5, "list": true})).unwrap();
        assert_eq!(list["layers"][0]["layer"], top.0);
        // A locked group locks the layers inside it.
        lock(&mut s, top.0, json!({"all": false, "position": false}));
        s.execute("layer.select", json!({"layer": top.0})).unwrap();
        let g = s.execute("layer.new.groupFromLayers", json!({"name": "G"})).unwrap()["layer"].as_u64().unwrap();
        assert_eq!(pick(&mut s), top.0);
        lock(&mut s, g, json!({"all": true}));
        assert_eq!(pick(&mut s), a.0);
        // Nothing unlocked under the point: no pick.
        lock(&mut s, a.0, json!({"all": true}));
        let bg = s.active().unwrap().doc.layers[0].id;
        assert_eq!(pick(&mut s), bg.0);
        lock(&mut s, bg.0, json!({"all": true}));
        assert_eq!(pick(&mut s), Value::Null);
    }

    #[test]
    fn auto_select_hits_a_type_layer_between_its_letters() {
        // #2370: a click in the gap between two glyphs picked the layer under the type.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 40})).unwrap();
        let bg = s.active().unwrap().doc.layers[0].id;
        let mut text = None;
        s.edit("type", |doc, _| {
            // Two "glyphs", (10..14)×(10..20) and (20..24)×(10..20), with a gap between them.
            let mut cache = photocraft_raster::Surface::new(doc.pixel_format());
            for x in [10, 20] {
                cache.fill_rect(Rect::new(x, 10, x + 4, 20), &[0.0, 0.0, 0.0, 1.0]);
            }
            let l = photocraft_doc::Layer::new("T", LayerContent::Text(photocraft_doc::TextLayer { cache: Some(cache), ..Default::default() }));
            text = Some(l.id);
            doc.layers.push(l);
            Ok(())
        })
        .unwrap();
        let t = text.unwrap();
        let pick = |s: &mut Session, x: i32, y: i32| s.execute("layer.pickAt", json!({"x": x, "y": y, "select": false})).unwrap()["layer"].clone();
        assert_eq!(pick(&mut s, 12, 15), t.0, "on a glyph");
        assert_eq!(pick(&mut s, 17, 15), t.0, "between the glyphs: the type layer, not the one under it");
        assert_eq!(pick(&mut s, 17, 20), bg.0, "below the text bounds");
        // The context menu also reaches this type layer between the glyphs (#4259).
        let list = s.execute("layer.pickAt", json!({"x": 17, "y": 15, "list": true})).unwrap();
        assert_eq!(list["layers"][0]["layer"], t.0);
        assert_eq!(list["layers"][1]["layer"], bg.0);
    }

    #[test]
    fn auto_select_hits_typed_text_in_a_gap_between_letters() {
        // The same through the type engine: "I I" leaves a transparent gap inside its bounds.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
        let bg = s.active().unwrap().doc.layers[0].id;
        let t = s.execute("type.create", json!({"text": "I   I", "size": 48, "x": 20, "y": 70})).unwrap()["layer"].as_u64().unwrap();
        let doc = s.active().unwrap().doc.clone();
        let Some(LayerContent::Text(tl)) = doc.layer(LayerId(t)).map(|l| &l.content) else { panic!("a type layer") };
        let cache = tl.cache.as_ref().expect("rendered type");
        let b = cache.content_bounds();
        // A transparent point on the middle row inside the bounds (none: no font to render with).
        let y = (b.y0 + b.y1) / 2;
        let gap = (b.x0..b.x1).find(|&x| {
            let mut px = [[0.0f32; 4]; 1];
            cache.read_rgba_into(Rect::from_xywh(x, y, 1, 1), &mut px);
            px[0][3] == 0.0
        });
        let Some(x) = gap else { return };
        assert_eq!(layers_at(&doc, x, y), vec![bg], "no type pixels at the gap");
        assert_eq!(s.execute("layer.pickAt", json!({"x": x, "y": y, "select": false})).unwrap()["layer"], t);
        let menu = s.execute("layer.pickAt", json!({"x": x, "y": y, "list": true})).unwrap();
        assert_eq!(menu["layers"][0]["layer"], t, "type layer stays above its background in the menu");
        assert_eq!(menu["layers"][1]["layer"], bg.0);
    }

    #[test]
    fn a_marquee_selects_the_layers_it_touches() {
        let (mut s, a, b) = two_squares();
        s.execute("layer.new.layer", json!({"name": "Empty"})).unwrap();
        let sel = |s: &mut Session, r: [f64; 4], target: &str, mode: &str| {
            let p = json!({"x": r[0], "y": r[1], "width": r[2], "height": r[3], "target": target, "mode": mode});
            let hit: Vec<u64> = s.execute("layer.selectInRect", p).unwrap()["layers"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
            let mut now: Vec<u64> = s.active().unwrap().selected_layers().iter().map(|l| l.0).collect();
            now.sort_unstable();
            (hit, now)
        };
        // Touching A's corner only (dragged up and left); never the empty layer or the Background.
        assert_eq!(sel(&mut s, [-5.0, -5.0, 6.0, 6.0], "layer", "replace"), (vec![a.0], vec![a.0]));
        assert_eq!(sel(&mut s, [50.0, 50.0, -27.0, -27.0], "layer", "replace"), (vec![b.0], vec![b.0]));
        // ⇧ adds; a box over nothing leaves the selection alone.
        assert_eq!(sel(&mut s, [0.0, 0.0, 5.0, 5.0], "layer", "add"), (vec![a.0], vec![a.0, b.0]));
        assert_eq!(sel(&mut s, [11.0, 11.0, 8.0, 8.0], "layer", "replace"), (vec![], vec![a.0, b.0]));
        assert_eq!(sel(&mut s, [-10.0, -10.0, 60.0, 60.0], "layer", "replace"), (vec![b.0, a.0], vec![a.0, b.0]));
        // Group mode selects the outermost group; a hidden or fully locked layer is skipped.
        s.execute("layer.select", json!({"layer": a.0})).unwrap();
        let g = s.execute("layer.new.groupFromLayers", json!({"name": "G"})).unwrap()["layer"].as_u64().unwrap();
        assert_eq!(sel(&mut s, [0.0, 0.0, 40.0, 40.0], "group", "replace").0, vec![b.0, g]);
        s.execute("layer.setProps", json!({"layer": b.0, "locks": {"all": true}})).unwrap();
        s.execute("layer.setProps", json!({"layer": g, "visible": false})).unwrap();
        assert_eq!(sel(&mut s, [0.0, 0.0, 40.0, 40.0], "layer", "replace").0, Vec::<u64>::new());
        for bad in [json!({}), json!({"x": 0, "y": 0, "width": f64::NAN, "height": 1}), json!({"x": 0, "y": 0, "width": 1, "height": 1, "mode": "x"})] {
            assert!(s.execute("layer.selectInRect", bad).is_err());
        }
        assert!(s.execute("layer.selectInRect", json!({"x": -1e300, "y": 1e300, "width": 1e308, "height": -1e308})).is_ok());
    }

    #[test]
    fn hostile_params_are_errors_or_misses_not_panics() {
        let (mut s, _, _) = two_squares();
        assert!(s.execute("layer.pickAt", json!({})).is_err());
        assert!(s.execute("layer.pickAt", json!({"x": "5", "y": 5})).is_err());
        for (x, y) in [(-1e12, 5.0), (1e12, 1e12), (-0.5, -0.5), (40.0, 40.0)] {
            assert_eq!(s.execute("layer.pickAt", json!({"x": x, "y": y})).unwrap()["layer"], Value::Null, "({x}, {y})");
        }
        let mut empty = Session::new();
        assert!(empty.execute("layer.pickAt", json!({"x": 1, "y": 1})).is_err());
    }

    /// A hide-all mask at density 0 hides nothing (the compositor applies density through
    /// `LayerMask::value`), so the layer stays pickable; at full density it is clicked through.
    #[test]
    fn mask_density_is_honoured_when_picking() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 20, "height": 20})).unwrap();
        let bg = s.active().unwrap().doc.layers[0].id;
        s.execute("layer.new.layer", json!({"name": "Red"})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let red = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.layerMask.hideAll", json!({})).unwrap();
        // Full density: the mask hides the layer, so the click reaches the Background.
        assert_eq!(s.execute("layer.pickAt", json!({"x": 5, "y": 5, "select": false})).unwrap()["layer"], bg.0);
        s.execute("layer.layerMask.edit", json!({"density": 0})).unwrap();
        assert_eq!(s.execute("layer.pickAt", json!({"x": 5, "y": 5, "select": false})).unwrap()["layer"], red.0);
        let list = s.execute("layer.pickAt", json!({"x": 5, "y": 5, "list": true})).unwrap();
        let ids: Vec<_> = list["layers"].as_array().unwrap().iter().map(|l| l["layer"].clone()).collect();
        assert_eq!(ids, vec![json!(red.0), json!(bg.0)]);
    }
}
