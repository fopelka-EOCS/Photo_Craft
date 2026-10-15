//! Free Transform's live preview, composited in the layer's place.
//!
//! While a box is open on a layer (Free Transform and its modes, Warp, ⌥⌘T, a placed file), the
//! layer's moving pixels are a [`LiveTransform`]: its pixels (with its linked masks, which move
//! with them) drawn through a mesh that follows the box. The canvas shows a document prepared for
//! it (`photocraft_compose::live::prepare`) and the GPU compositor draws the mesh into the layer's
//! content, so the layer keeps its place in the stack and its blend mode, opacity, fill, Blend
//! If, unlinked masks and clipping while it moves, and the layers above it stay above it: a patch
//! lined up in Difference shows in Difference. Where the GPU can't (a layer with a style, whose
//! style follows its new shape; a pass-through group), the CPU composites the same document with
//! the pixels moved (`photocraft_compose::live::apply`).
//!
//! The mesh is rebuilt only when the box changes, the prepared document only when the document
//! does; each new mesh is a new preview key, and the canvas redraws only where the layer was and
//! is (`damage`).

use std::collections::VecDeque;
use std::sync::Arc;

use photocraft_algo::transform::Homography;
use photocraft_compose::live::{self, LiveMesh, LiveTransform};
use photocraft_doc::{DocId, Document, Layer, LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

use crate::PhotocraftApp;
use crate::state::TransformSession;

/// Preview keys: `BASE + n`, one per mesh shown (see `canvas::display_doc`).
const BASE: u64 = 1 << 46;
/// Cells per side of the grid a box is drawn with: fine enough that a steep perspective stays
/// straight to the eye (each cell is affine).
const GRID: usize = 32;
/// Keys whose layer area is remembered for damage.
const KEEP: usize = 8;

/// A layer's live preview over a transform session.
pub(crate) struct LiveSession {
    layer: LayerId,
    /// The moving pixels and, with a selection, the pixels that stay (both with the linked masks
    /// applied).
    source: Arc<Surface>,
    rest: Option<Arc<Surface>>,
    /// The document prepared for the mesh, and the revision it was made from.
    doc: DocId,
    base: Arc<Document>,
    revision: u64,
    /// The last mesh and the box it was built from.
    mesh: Option<(String, Arc<LiveMesh>)>,
    serial: u64,
    /// Recent preview keys and the document pixels the moving layer covered in each.
    shown: VecDeque<(u64, Rect)>,
    /// Where the layer's pixels were before the session (they leave from there).
    origin: Rect,
    /// The CPU composite's document for the current key (`cpu_doc`).
    applied: Option<(u64, Arc<Document>)>,
}

impl LiveSession {
    /// The live preview of layer `id` of `doc` (at `revision`): `lifted` is the selection's
    /// pixels taken out of it, `rest` what stays. `None` for a layer that doesn't move as pixels
    /// (an adjustment layer moves only its mask).
    pub(crate) fn start(doc: &Document, revision: u64, id: LayerId, lifted: Option<&Surface>, rest: Option<&Surface>) -> Option<Self> {
        let layer = doc.layer(id)?;
        if matches!(layer.content, LayerContent::Adjustment(_)) {
            return None;
        }
        let source = match lifted {
            Some(l) => linked_masks(l, layer),
            None => linked_masks(&own_pixels(doc, layer)?, layer),
        };
        let rest = rest.map(|r| Arc::new(linked_masks(r, layer)));
        let origin = layer.surface().filter(|_| matches!(layer.content, LayerContent::Raster(_))).map_or(doc.bounds(), Surface::content_bounds);
        let mut s = LiveSession {
            layer: id,
            source: Arc::new(source),
            rest,
            doc: doc.id,
            base: Arc::new(doc.clone()),
            revision,
            mesh: None,
            serial: 0,
            shown: VecDeque::new(),
            origin,
            applied: None,
        };
        s.base = Arc::new(live::prepare(doc, &s.live(Arc::new(LiveMesh::default()))));
        Some(s)
    }

    /// Move layer `id` of `doc` from now on (Warp on editable type starts from the type without
    /// its Warp Text): its pixels as `doc` has them, the mesh rebuilt on the next frame.
    pub(crate) fn restart_from(&mut self, doc: &Document, id: LayerId) {
        if let Some(layer) = doc.layer(id)
            && let Some(px) = own_pixels(doc, layer)
        {
            self.source = Arc::new(linked_masks(&px, layer));
            self.mesh = None;
            self.applied = None;
        }
    }

    fn live(&self, mesh: Arc<LiveMesh>) -> LiveTransform {
        LiveTransform { layer: self.layer, source: self.source.clone(), rest: self.rest.clone(), mesh }
    }

    /// The live transform as the box has it now, if a mesh was built.
    pub(crate) fn current(&self) -> Option<(DocId, LiveTransform)> {
        let (_, m) = self.mesh.as_ref()?;
        Some((self.doc, self.live(m.clone())))
    }

    fn key(&self) -> u64 {
        BASE + self.serial
    }
}

/// The layer's own pixels as they show alone: a pixel layer's surface, else (type, shape, smart
/// object, fill, group) its content rendered by itself, without its style, masks, opacity, fill
/// or clipping, in the document's format.
fn own_pixels(doc: &Document, layer: &Layer) -> Option<Surface> {
    if let (LayerContent::Raster(s), true) = (&layer.content, layer.surface().is_some()) {
        return Some(s.clone());
    }
    let b = photocraft_engine::transform_cmds::transform_bounds(doc, layer).intersect(&doc.bounds().inflate(4096));
    if b.is_empty() {
        return None;
    }
    let mut solo = doc.clone();
    let mut l = layer.clone();
    l.visible = true;
    l.effects.enabled = false;
    l.opacity = 1.0;
    l.fill_opacity = 1.0;
    l.clipped = false;
    l.mask = None;
    l.vector_mask = None;
    if l.blend == photocraft_color::BlendMode::PassThrough || !l.is_group() {
        l.blend = photocraft_color::BlendMode::Normal;
    }
    solo.layers = vec![l];
    let buf = photocraft_compose::render(&solo, b);
    let fmt = doc.pixel_format();
    let n = fmt.channels();
    let mut flat = vec![0.0f32; buf.px.len() * n];
    for (p, o) in buf.px.iter().zip(flat.chunks_exact_mut(n)) {
        photocraft_raster::from_rgba_into(&fmt, *p, o);
    }
    let mut s = Surface::new(fmt);
    s.write_region(b, &flat);
    s.prune();
    Some(s)
}

/// `s` with `layer`'s enabled linked masks applied to its alpha: they move with the pixels.
fn linked_masks(s: &Surface, layer: &Layer) -> Surface {
    let mut moving = layer.clone();
    if moving.mask.as_ref().is_some_and(|m| !m.linked || !m.enabled) {
        moving.mask = None;
    }
    if moving.vector_mask.as_ref().is_some_and(|m| !m.linked || !m.enabled) {
        moving.vector_mask = None;
    }
    if moving.mask.is_none() && moving.vector_mask.is_none() {
        return s.clone();
    }
    let area = s.content_bounds();
    if area.is_empty() {
        return s.clone();
    }
    let (mask, density) = match photocraft_compose::masks::combined_mask(&moving, area) {
        Some(c) => (c, 1.0),
        None => match &moving.mask {
            Some(m) => (m.surface.clone(), m.density),
            None => return s.clone(),
        },
    };
    let fmt = s.format();
    let n = fmt.channels();
    if !fmt.alpha || n == 0 {
        return s.clone();
    }
    let mut px = s.read_region(area);
    let m = mask.read_region(area);
    for (p, v) in px.chunks_exact_mut(n).zip(m) {
        if let Some(a) = p.last_mut() {
            *a *= 1.0 - density * (1.0 - v);
        }
    }
    let mut out = s.clone();
    out.write_region(area, &px);
    out.prune();
    out
}

/// The mesh carrying the layer's pixels onto the box: a grid over the box's source rectangle
/// (grown by a texel, so the edge fades as the transformed layer's does), or the warp's mesh.
/// Vertex positions are document pixels; texel coordinates are the source's, which is where
/// the layer was when the box opened. Cells past a perspective's horizon are left out.
pub(crate) fn mesh(t: &TransformSession) -> LiveMesh {
    let r = t.rect;
    let (w, h) = (r[2] - r[0], r[3] - r[1]);
    let mut m = LiveMesh::default();
    if !(w > 0.0 && h > 0.0 && w.is_finite() && h.is_finite()) {
        return m;
    }
    if let Some(warp) = &t.warp {
        let bm = warp.to_mesh(1, 1);
        let (us, vs) = crate::warp_preview::samples(&bm, 1.0);
        let mut rest = Vec::with_capacity(us.len() * vs.len());
        for v in &vs {
            for u in &us {
                let (x, y) = (r[0] + u * w, r[1] + v * h);
                let (px, py) = warp.map(x, y);
                m.pos.push([px as f32, py as f32]);
                m.uv.push([x as f32, y as f32]);
                rest.push([x as f32, y as f32]);
            }
        }
        m.indices = crate::warp_preview::triangle_order(&m.pos, &rest, us.len());
        return m;
    }
    let Some(hu) = Homography::rect_to_quad([0.0, 0.0, 1.0, 1.0], t.quad) else { return m };
    let front = hu.front_side([0.0, 0.0, 1.0, 1.0]);
    let (du, dv) = (1.0 / w, 1.0 / h);
    let mut ahead = Vec::with_capacity((GRID + 1) * (GRID + 1));
    for j in 0..=GRID {
        for i in 0..=GRID {
            // From a texel before the source to a texel past it.
            let u = -du + (1.0 + 2.0 * du) * i as f64 / GRID as f64;
            let v = -dv + (1.0 + 2.0 * dv) * j as f64 / GRID as f64;
            let (x, y) = hu.apply(u, v);
            m.pos.push([x as f32, y as f32]);
            m.uv.push([(r[0] + u * w) as f32, (r[1] + v * h) as f32]);
            ahead.push(front * hu.w(u, v) > 1e-9);
        }
    }
    for j in 0..GRID {
        for i in 0..GRID {
            let a = (j * (GRID + 1) + i) as u32;
            let n = GRID as u32;
            for tri in [[a, a + 1, a + n + 2], [a, a + n + 2, a + n + 1]] {
                // A cell with a corner past the horizon would stretch across the canvas.
                if tri.iter().all(|&k| ahead.get(k as usize).copied().unwrap_or(false)) {
                    m.indices.extend_from_slice(&tri);
                }
            }
        }
    }
    m
}

/// What the box is now, as far as the mesh goes.
fn box_key(t: &TransformSession) -> String {
    format!("{:?}{:?}{}", t.rect, t.quad, t.warp.as_ref().map_or_else(String::new, |w| serde_json::to_string(w).unwrap_or_default()))
}

/// The document the canvas shows for document `idx` while a box is open on one of its layers:
/// prepared for the live transform, with a key per mesh. `None` without a live session.
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    let st = app.session.documents().get(idx)?;
    let (doc, revision) = (st.doc.clone(), st.revision);
    let t = app.ui.transform.as_ref()?;
    let key = box_key(t);
    let built = app.transform_preview.as_ref().and_then(|pv| pv.live.as_ref()).is_some_and(|s| s.shows(&key));
    let fresh = (!built).then(|| mesh(t));
    let s = app.transform_preview.as_mut()?.live.as_mut()?;
    show(s, &doc, revision, key, fresh)
}

/// The document the canvas shows for document `idx` while Puppet Warp or Perspective Warp
/// moves one of its layers (distort_ui.rs), like [`display_doc`].
pub(crate) fn distort_display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    if app.session.active_index() != Some(idx) {
        return None;
    }
    let st = app.session.documents().get(idx)?;
    let (doc, revision) = (st.doc.clone(), st.revision);
    if let Some(p) = app.distort.puppet.as_mut() {
        let key = p.live_key();
        let fresh = p.live.as_ref().is_some_and(|s| !s.shows(&key)).then(|| p.live_mesh());
        return show(p.live.as_mut()?, &doc, revision, key, fresh);
    }
    let p = app.distort.perspective.as_mut()?;
    let key = p.live_key();
    let fresh = p.live.as_ref().is_some_and(|s| !s.shows(&key)).then(|| p.live_mesh());
    show(p.live.as_mut()?, &doc, revision, key, fresh)
}

impl LiveSession {
    /// Is the mesh built from `key` the one shown?
    fn shows(&self, key: &str) -> bool {
        self.mesh.as_ref().is_some_and(|(k, _)| k == key)
    }
}

/// `s`'s document for `doc` at `revision`, with `fresh` as its mesh (built from `key`) when the
/// tool's mesh changed.
fn show(s: &mut LiveSession, doc: &Arc<Document>, revision: u64, key: String, fresh: Option<LiveMesh>) -> Option<(Arc<Document>, u64)> {
    if s.doc != doc.id {
        return None;
    }
    if s.revision != revision {
        // Edits while the tool is open (another layer, the layer's blend mode or style): the
        // moving pixels stay what they were when it opened.
        s.base = Arc::new(live::prepare(doc, &s.live(Arc::new(LiveMesh::default()))));
        s.revision = revision;
        s.applied = None;
    }
    if let Some(m) = fresh {
        s.serial += 1;
        let m = Arc::new(m);
        let area = m.bounds();
        s.mesh = Some((key, m));
        let k = s.key();
        s.shown.push_back((k, area));
        while s.shown.len() > KEEP {
            s.shown.pop_front();
        }
    }
    Some((s.base.clone(), s.key()))
}

/// The open live session, whichever tool holds it.
fn session(app: &PhotocraftApp) -> Option<&LiveSession> {
    app.transform_preview
        .as_ref()
        .and_then(|pv| pv.live.as_ref())
        .or_else(|| app.distort.puppet.as_ref().and_then(|p| p.live.as_ref()))
        .or_else(|| app.distort.perspective.as_ref().and_then(|p| p.live.as_ref()))
}

fn session_mut(app: &mut PhotocraftApp) -> Option<&mut LiveSession> {
    if let Some(s) = app.transform_preview.as_mut().and_then(|pv| pv.live.as_mut()) {
        return Some(s);
    }
    if let Some(s) = app.distort.puppet.as_mut().and_then(|p| p.live.as_mut()) {
        return Some(s);
    }
    app.distort.perspective.as_mut().and_then(|p| p.live.as_mut())
}

/// The live transform the GPU compositor should draw for document `doc`, if one is showing.
pub(crate) fn current(app: &PhotocraftApp, doc: DocId) -> Option<(DocId, LiveTransform)> {
    session(app)?.current().filter(|(d, _)| *d == doc)
}

/// `doc` (shown under preview `key`) as the CPU compositor draws it: with the moving pixels in
/// place when it is a live transform's document, else unchanged.
pub(crate) fn cpu_doc(app: &mut PhotocraftApp, doc: Arc<Document>, key: u64) -> Arc<Document> {
    let Some(s) = session_mut(app).filter(|s| s.doc == doc.id && s.key() == key) else { return doc };
    if let Some((k, d)) = &s.applied
        && *k == key
    {
        return d.clone();
    }
    let Some((_, live)) = s.current() else { return doc };
    let d = Arc::new(live::apply(&doc, &live));
    s.applied = Some((key, d.clone()));
    d
}

/// What changed on the canvas between preview keys `seen` and `now` of document `doc` (at
/// `revision`): where the moving layer was in each (its original place for the document itself).
/// `None` when either key isn't one of this session's recent ones.
pub(crate) fn damage(app: &PhotocraftApp, doc: DocId, revision: u64, seen: u64, now: u64) -> Option<Rect> {
    let s = session(app).filter(|s| s.doc == doc && s.revision == revision)?;
    let area = |k: u64| -> Option<Rect> {
        if k == 0 {
            return Some(s.origin);
        }
        s.shown.iter().find(|(key, _)| *key == k).map(|(_, r)| *r)
    };
    let (a, b) = (area(seen)?, area(now)?);
    Some(
        if a.is_empty() {
            b
        } else if b.is_empty() {
            a
        } else {
            a.union(&b)
        }
        .inflate(1),
    )
}

/// A short key for a list of points (a tool's mesh positions): it changes when any point does.
pub(crate) fn points_key(pts: &[[f64; 2]]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in pts {
        p[0].to_bits().hash(&mut h);
        p[1].to_bits().hash(&mut h);
    }
    format!("{}:{:x}", pts.len(), h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app_with_layer(blend: &str) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48, "background": "white"})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 16})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#ff8000"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        app.run("layer.setProps", json!({"blend": blend})).unwrap();
        app
    }

    /// The canvas shows the document prepared for the mesh, with the layer's blend mode, and the
    /// CPU composite of it is the transformed layer composited in Difference: what OK will make.
    #[test]
    fn the_preview_composites_in_the_layers_blend_mode_and_matches_the_commit() {
        let mut app = app_with_layer("Difference");
        let ctx = egui::Context::default();
        crate::transform_tool::begin(&mut app, &ctx).unwrap();
        let t = app.ui.transform.as_mut().unwrap();
        let d = [14.0, 9.0];
        t.quad = t.quad.map(|q| [q[0] + d[0], q[1] + d[1]]);
        let (shown, key) = crate::canvas::display_doc(&mut app, 0);
        assert!(key >= BASE, "a live preview key");
        let id = app.session.active().unwrap().active_layer.unwrap();
        assert_eq!(shown.layer(id).unwrap().blend, photocraft_color::BlendMode::Difference);
        assert!(current(&app, shown.id).is_some(), "the GPU gets the live transform");
        let cpu = cpu_doc(&mut app, shown, key);
        let r = Rect::new(0, 0, 64, 48);
        let preview = photocraft_compose::render(&cpu, r);
        crate::transform_tool::commit(&mut app);
        let committed = photocraft_compose::render(&app.session.active().unwrap().doc, r);
        let worst = preview.px.iter().zip(&committed.px).map(|(a, b)| (0..4).map(|k| (a[k] - b[k]).abs()).fold(0.0f32, f32::max)).fold(0.0f32, f32::max);
        assert!(worst < 2.0 / 255.0, "preview vs commit: {:.2}/255", worst * 255.0);
        // Difference over white: the moved orange shows as its inverse where it landed.
        let at = |x: i32, y: i32| preview.px[(y * 64 + x) as usize];
        assert!(at(30, 25)[2] > 0.9 && at(30, 25)[0] < 0.1, "{:?}", at(30, 25));
        assert!(at(12, 12)[0] > 0.99, "left behind: white again");
    }

    /// A new box makes a new key; the damage between keys covers where the layer was and is.
    #[test]
    fn each_box_is_a_new_key_with_damage_where_the_layer_was_and_is() {
        let mut app = app_with_layer("Multiply");
        let ctx = egui::Context::default();
        crate::transform_tool::begin(&mut app, &ctx).unwrap();
        let (_, k1) = crate::canvas::display_doc(&mut app, 0);
        let (_, again) = crate::canvas::display_doc(&mut app, 0);
        assert_eq!(k1, again, "an unchanged box keeps its key");
        let t = app.ui.transform.as_mut().unwrap();
        t.quad = t.quad.map(|q| [q[0] + 20.0, q[1]]);
        let (_, k2) = crate::canvas::display_doc(&mut app, 0);
        assert_ne!(k1, k2);
        let (doc, rev) = (app.session.active().unwrap().doc.id, app.session.active().unwrap().revision);
        let d = damage(&app, doc, rev, k1, k2).unwrap();
        assert!(d.contains(10, 10) && d.contains(49, 25) && !d.contains(60, 40), "{d:?}");
        let from_doc = damage(&app, doc, rev, 0, k2).unwrap();
        assert!(from_doc.contains(10, 10) && from_doc.contains(49, 25), "{from_doc:?}");
        assert!(damage(&app, doc, rev, 12345, k2).is_none());
    }

    /// A box past the horizon or a degenerate one draws only what's in front; a linked mask moves
    /// with the pixels (its moved copy is in the source), an unlinked one stays in the layer.
    #[test]
    fn masks_and_degenerate_boxes() {
        let mut app = app_with_layer("Normal");
        app.run("layer.layerMask.revealAll", json!({})).unwrap();
        let ctx = egui::Context::default();
        crate::transform_tool::begin(&mut app, &ctx).unwrap();
        let (shown, _) = crate::canvas::display_doc(&mut app, 0);
        let id = app.session.active().unwrap().active_layer.unwrap();
        assert!(shown.layer(id).unwrap().mask.is_none(), "the linked mask moves with the source");
        let t = app.ui.transform.as_mut().unwrap();
        t.quad = [[0.0, 0.0]; 4];
        let m = mesh(t);
        assert!(m.triangles().next().is_none());
        t.rect = [0.0, 0.0, f64::NAN, 1.0];
        assert!(mesh(t).indices.is_empty());
    }
}
