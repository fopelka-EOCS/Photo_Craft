//! A layer seen through a live transform while its box is open (Free Transform and its modes,
//! Warp, Puppet Warp, Perspective Warp).
//!
//! The moving pixels are a [`LiveTransform`]: a source surface drawn through a triangle mesh
//! (document positions, source texel coordinates). It is composited in the layer's own place in
//! the stack, so the layer keeps its blend mode, opacity, fill, Blend If, unlinked masks,
//! clipping and style, and the layers above stay above it. The GPU compositor draws the mesh
//! itself (`photocraft-gpu`); [`apply`] gives the CPU compositor the same document with the layer's
//! pixels moved, and is the reference the GPU result is checked against.
//!
//! The mesh is drawn in index order, a later triangle replacing an earlier one where they overlap
//! (a warp folded over itself shows its front, as the tools order it). Inside a triangle the
//! source is sampled bilinearly, premultiplied by alpha, in the surface's own channels; texels
//! outside the source are transparent, so a mesh reaching a texel past the source's edge draws
//! the edge soft, as a resampled layer's is.

use std::sync::Arc;

use photocraft_doc::{Document, LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

/// Most triangles a mesh may have (a 24×24 grid is 1152; a fine warp a few thousand).
pub const MAX_TRIANGLES: usize = 1 << 20;
/// Positions and texel coordinates stay within this, so bounds arithmetic can't overflow.
const COORD_LIMIT: f32 = 1.0e7;

/// Triangles carrying the source onto the canvas.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LiveMesh {
    /// Vertex positions, in document pixels.
    pub pos: Vec<[f32; 2]>,
    /// The source point each vertex shows, in the source surface's pixel coordinates (texel
    /// centres at `.5`).
    pub uv: Vec<[f32; 2]>,
    /// Three vertex indices per triangle, drawn in this order.
    pub indices: Vec<u32>,
}

impl LiveMesh {
    /// The triangles that can be drawn: in range, finite, within [`COORD_LIMIT`] and not flat. A
    /// mesh with a bad vertex loses only the triangles that use it.
    pub fn triangles(&self) -> impl Iterator<Item = [([f32; 2], [f32; 2]); 3]> + '_ {
        let ok = |p: [f32; 2]| p.iter().all(|v| v.is_finite() && v.abs() <= COORD_LIMIT);
        self.indices.as_chunks::<3>().0.iter().take(MAX_TRIANGLES).filter_map(move |t| {
            let v = |i: u32| -> Option<([f32; 2], [f32; 2])> {
                let (p, q) = (*self.pos.get(i as usize)?, *self.uv.get(i as usize)?);
                (ok(p) && ok(q)).then_some((p, q))
            };
            let t = [v(*t.first()?)?, v(*t.get(1)?)?, v(*t.get(2)?)?];
            let [a, b, c] = t.map(|v| v.0);
            // A triangle flattened onto a line covers nothing.
            let area = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
            (area.is_finite() && area.abs() >= 1e-9).then_some(t)
        })
    }

    /// The document pixels the mesh can cover (rounded out); empty without a drawable triangle.
    pub fn bounds(&self) -> Rect {
        let mut lo = [f32::MAX; 2];
        let mut hi = [f32::MIN; 2];
        for tri in self.triangles() {
            for (p, _) in tri {
                lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                hi = [hi[0].max(p[0]), hi[1].max(p[1])];
            }
        }
        if lo[0] > hi[0] {
            return Rect::EMPTY;
        }
        Rect::new(lo[0].floor() as i32, lo[1].floor() as i32, hi[0].ceil() as i32, hi[1].ceil() as i32)
    }
}

/// A layer's pixels moving through a live transform.
#[derive(Clone, Debug)]
pub struct LiveTransform {
    /// The moving layer.
    pub layer: LayerId,
    /// The moving pixels, with the layer's linked masks applied (they move with the pixels), in
    /// the layer's format. For a layer that isn't plain pixels (type, shape, smart object, fill,
    /// isolated group), its content rendered alone, without its masks, style, opacity or fill.
    pub source: Arc<Surface>,
    /// With a selection, the layer's pixels that stay where they are (the moving ones are drawn
    /// over them); `None` when the whole layer moves.
    pub rest: Option<Arc<Surface>>,
    pub mesh: Arc<LiveMesh>,
}

impl LiveTransform {
    /// The document pixels the moving layer can change: where the mesh lands, plus where the
    /// layer was (its pixels leave from there).
    pub fn damage(&self, doc: &Document) -> Rect {
        let to = self.mesh.bounds();
        // A layer without pixels of its own (type, shape, group) may have shown anywhere.
        let from = doc
            .layer(self.layer)
            .map_or(Rect::EMPTY, |l| l.surface().filter(|_| matches!(l.content, LayerContent::Raster(_))).map_or(doc.bounds(), Surface::content_bounds));
        if to.is_empty() {
            from
        } else if from.is_empty() {
            to
        } else {
            to.union(&from)
        }
    }
}

/// `live`'s moving pixels over `rect`, in the source's format: the mesh drawn over transparency
/// (or over `live.rest` there). Native channels, straight alpha, as a surface region.
pub fn rasterize(live: &LiveTransform, rect: Rect) -> Vec<f32> {
    let fmt = live.source.format();
    let n = fmt.channels();
    let width = rect.width() as usize;
    let mut out = match &live.rest {
        Some(rest) if rest.format() == fmt => rest.read_region(rect),
        _ => vec![0.0; width * rect.height() as usize * n],
    };
    if rect.is_empty() || !fmt.alpha || n == 0 {
        return out;
    }
    let tris: Vec<_> = live.mesh.triangles().filter_map(|t| Tri::new(t, rect)).collect();
    if tris.is_empty() {
        return out;
    }
    // The source texels the mesh reads, premultiplied, with a transparent texel around them.
    let reach = tris.iter().fold(Rect::EMPTY, |r, t| if r.is_empty() { t.texels } else { r.union(&t.texels) });
    let src = Texels::read(&live.source, reach.intersect(&live.source.tile_bounds()));
    let row = |y: i32, line: &mut [f32]| {
        let py = y as f32 + 0.5;
        // The mesh over transparency first (a later triangle replaces an earlier one), then over
        // what the line holds (the pixels that stay), as the commit lays the moved pixels down.
        let mut moved = vec![0.0f32; width * n];
        let mut hit = vec![false; width];
        for t in tris.iter().filter(|t| t.rows.0 <= y && y < t.rows.1) {
            for x in t.cols.0..t.cols.1 {
                let Some(uv) = t.at(x as f32 + 0.5, py) else { continue };
                let i = (x - rect.x0) as usize;
                if let (Some(o), Some(h)) = (moved.get_mut(i * n..i * n + n), hit.get_mut(i)) {
                    src.sample(uv, o);
                    *h = true;
                }
            }
        }
        for (i, _) in hit.iter().enumerate().filter(|(_, h)| **h) {
            if let (Some(m), Some(o)) = (moved.get(i * n..i * n + n), line.get_mut(i * n..i * n + n)) {
                over(m, o);
            }
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        out.par_chunks_mut(width * n).enumerate().for_each(|(i, line)| row(rect.y0 + i as i32, line));
    }
    #[cfg(target_arch = "wasm32")]
    for (i, line) in out.chunks_mut(width * n).enumerate() {
        row(rect.y0 + i as i32, line);
    }
    out
}

/// `doc` ready for `live` to be drawn into it: the moving layer holds only the pixels that stay
/// (`live.rest`, else none) as a raster layer in the source's format, and its linked masks are
/// left out (the source carries them). Everything else about it and the document is unchanged.
/// The GPU compositor draws the mesh into this layer's content; [`apply`] does it on the CPU.
pub fn prepare(doc: &Document, live: &LiveTransform) -> Document {
    let mut d = doc.clone();
    let Some(l) = d.layer_mut(live.layer) else { return d };
    let rest = live.rest.as_ref().filter(|r| r.format() == live.source.format()).map_or_else(|| Surface::new(live.source.format()), |r| (**r).clone());
    l.content = LayerContent::Raster(rest);
    l.fill_cache = None;
    if l.mask.as_ref().is_some_and(|m| m.linked) {
        l.mask = None;
    }
    if l.vector_mask.as_ref().is_some_and(|m| m.linked) {
        l.vector_mask = None;
    }
    d
}

/// `doc` as the CPU compositor sees it with `live` applied: [`prepare`]d, with the moving pixels
/// drawn into the layer where the mesh puts them.
pub fn apply(doc: &Document, live: &LiveTransform) -> Document {
    let mut d = prepare(doc, live);
    let r = live.mesh.bounds().intersect(&doc.bounds().inflate(1));
    if let Some(s) = d.layer_mut(live.layer).and_then(|l| l.surface_mut())
        && !r.is_empty()
    {
        s.write_region(r, &rasterize(live, r));
        s.prune();
    }
    d
}

/// A triangle prepared for scan conversion within a target rect.
struct Tri {
    p: [[f32; 2]; 3],
    uv: [[f32; 2]; 3],
    /// Twice the signed area.
    area: f32,
    /// Rows `y0..y1` and columns `x0..x1` of the target it can cover.
    rows: (i32, i32),
    cols: (i32, i32),
    /// The source texels it reads (bilinear footprint).
    texels: Rect,
}

impl Tri {
    fn new(t: [([f32; 2], [f32; 2]); 3], rect: Rect) -> Option<Self> {
        let p = t.map(|v| v.0);
        let uv = t.map(|v| v.1);
        let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
        if !area.is_finite() || area.abs() < 1e-9 {
            return None;
        }
        let span = |i: usize| (p.iter().map(|v| v[i]).fold(f32::MAX, f32::min), p.iter().map(|v| v[i]).fold(f32::MIN, f32::max));
        let ((x0, x1), (y0, y1)) = (span(0), span(1));
        // Pixel centres inside: from the first centre at or past the low edge.
        let cols = (((x0 - 0.5).ceil() as i32).max(rect.x0), (((x1 - 0.5).floor() as i32) + 1).min(rect.x1));
        let rows = (((y0 - 0.5).ceil() as i32).max(rect.y0), (((y1 - 0.5).floor() as i32) + 1).min(rect.y1));
        if cols.0 >= cols.1 || rows.0 >= rows.1 {
            return None;
        }
        let ((u0, u1), (v0, v1)) = (
            (uv.iter().map(|v| v[0]).fold(f32::MAX, f32::min), uv.iter().map(|v| v[0]).fold(f32::MIN, f32::max)),
            (uv.iter().map(|v| v[1]).fold(f32::MAX, f32::min), uv.iter().map(|v| v[1]).fold(f32::MIN, f32::max)),
        );
        let texels = Rect::new((u0 - 0.5).floor() as i32, (v0 - 0.5).floor() as i32, (u1 - 0.5).floor() as i32 + 2, (v1 - 0.5).floor() as i32 + 2);
        Some(Tri { p, uv, area, rows, cols, texels })
    }

    /// The source point at document point (`x`, `y`), when it is inside. Edges shared by two
    /// triangles count for one of them (the top-left rule), so a seam is drawn once.
    fn at(&self, x: f32, y: f32) -> Option<[f32; 2]> {
        let s = self.area.signum();
        let edge = |a: [f32; 2], b: [f32; 2]| -> Option<f32> {
            let e = ((b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0])) * s;
            // Top-left rule: on an edge, only the top and left edges are in.
            let (dx, dy) = ((b[0] - a[0]) * s, (b[1] - a[1]) * s);
            let top_left = (dy == 0.0 && dx < 0.0) || dy > 0.0;
            (e > 0.0 || (e == 0.0 && top_left)).then_some(e)
        };
        let [a, b, c] = self.p;
        let w0 = edge(b, c)?;
        let w1 = edge(c, a)?;
        let w2 = edge(a, b)?;
        let k = 1.0 / (self.area * s);
        let (w0, w1, w2) = (w0 * k, w1 * k, w2 * k);
        let [ua, ub, uc] = self.uv;
        Some([ua[0] * w0 + ub[0] * w1 + uc[0] * w2, ua[1] * w0 + ub[1] * w1 + uc[1] * w2])
    }
}

/// Source texels over a rect, premultiplied by alpha, transparent outside.
struct Texels {
    rect: Rect,
    n: usize,
    px: Vec<f32>,
}

impl Texels {
    fn read(s: &Surface, rect: Rect) -> Self {
        let n = s.format().channels();
        let mut px = if rect.is_empty() { Vec::new() } else { s.read_region(rect) };
        if n > 0 {
            for p in px.chunks_exact_mut(n) {
                let a = p[n - 1];
                for c in &mut p[..n - 1] {
                    *c *= a;
                }
            }
        }
        Texels { rect, n, px }
    }

    fn texel(&self, x: i32, y: i32) -> Option<&[f32]> {
        if !self.rect.contains(x, y) {
            return None;
        }
        let i = ((y - self.rect.y0) as usize * self.rect.width() as usize + (x - self.rect.x0) as usize) * self.n;
        self.px.get(i..i + self.n)
    }

    /// Bilinear sample at source point `uv` (texel centres at `.5`), premultiplied, into `out`.
    fn sample(&self, uv: [f32; 2], out: &mut [f32]) {
        out.fill(0.0);
        let (fx, fy) = (uv[0] - 0.5, uv[1] - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (ax, ay) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as i32, y0 as i32);
        for (dx, dy, w) in [(0, 0, (1.0 - ax) * (1.0 - ay)), (1, 0, ax * (1.0 - ay)), (0, 1, (1.0 - ax) * ay), (1, 1, ax * ay)] {
            if w <= 0.0 {
                continue;
            }
            if let Some(t) = self.texel(x0 + dx, y0 + dy) {
                for (o, v) in out.iter_mut().zip(t) {
                    *o += v * w;
                }
            }
        }
    }
}

/// Premultiplied `m` over the straight-alpha pixel `out`, left straight in `out`.
fn over(m: &[f32], out: &mut [f32]) {
    let n = m.len();
    let (Some(&ma), Some(&oa)) = (m.last(), out.last()) else { return };
    let (ma, oa) = (ma.clamp(0.0, 1.0), oa.clamp(0.0, 1.0));
    let a = ma + oa * (1.0 - ma);
    for (i, (o, v)) in out.iter_mut().zip(m).enumerate() {
        *o = if i + 1 == n {
            a
        } else if a > 1e-6 {
            (v + *o * oa * (1.0 - ma)) / a
        } else {
            0.0
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;

    /// Two triangles covering `r` (document) showing source rect `s`.
    fn quad_mesh(r: [f32; 4], s: [f32; 4]) -> LiveMesh {
        LiveMesh {
            pos: vec![[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]],
            uv: vec![[s[0], s[1]], [s[2], s[1]], [s[2], s[3]], [s[0], s[3]]],
            indices: vec![0, 1, 2, 0, 2, 3],
        }
    }

    fn red_square(fmt: PixelFormat) -> Surface {
        let mut s = Surface::new(fmt);
        s.fill_rect(Rect::new(2, 2, 6, 6), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
        s
    }

    /// An identity mesh reproduces the source; a shifted one moves it, at every depth.
    #[test]
    fn identity_and_shift_reproduce_the_source() {
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
            let src = Arc::new(red_square(fmt));
            let live =
                LiveTransform { layer: LayerId(1), source: src.clone(), rest: None, mesh: Arc::new(quad_mesh([0.0, 0.0, 8.0, 8.0], [0.0, 0.0, 8.0, 8.0])) };
            let r = Rect::new(0, 0, 8, 8);
            let got = rasterize(&live, r);
            assert!(got.iter().zip(src.read_region(r)).all(|(a, b)| (a - b).abs() < 1e-4), "{fmt:?} identity");
            let shifted = LiveTransform { mesh: Arc::new(quad_mesh([10.0, 1.0, 18.0, 9.0], [0.0, 0.0, 8.0, 8.0])), ..live };
            let at = |x: i32, y: i32| {
                let v = rasterize(&shifted, Rect::new(x, y, x + 1, y + 1));
                photocraft_raster::to_rgba(&fmt, &v)
            };
            assert!((at(13, 4)[0] - 1.0).abs() < 1e-4 && (at(13, 4)[3] - 1.0).abs() < 1e-4, "{fmt:?} moved");
            assert_eq!(at(3, 3)[3], 0.0, "{fmt:?} left behind");
        }
    }

    /// Scaling up interpolates between texels; the edge fades out over the transparent border.
    #[test]
    fn scaling_up_is_bilinear_with_a_soft_edge() {
        let fmt = PixelFormat::RGBA8;
        let live = LiveTransform {
            layer: LayerId(1),
            source: Arc::new(red_square(fmt)),
            rest: None,
            mesh: Arc::new(quad_mesh([0.0, 0.0, 32.0, 32.0], [0.0, 0.0, 8.0, 8.0])),
        };
        let px = rasterize(&live, Rect::new(0, 16, 32, 17));
        let alpha: Vec<f32> = px.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
        assert!(alpha[16] > 0.99, "inside");
        assert!(alpha[2] < 0.01, "outside");
        assert!(alpha[7] > 0.2 && alpha[7] < 0.8, "the edge is soft: {}", alpha[7]);
        assert!(px.as_chunks::<4>().0.iter().filter(|p| p[3] > 0.01).all(|p| (p[0] - 1.0).abs() < 0.01), "straight colour stays red at the soft edge");
    }

    /// `apply` moves the layer's pixels and keeps everything else about it; linked masks go, an
    /// unlinked one stays.
    #[test]
    fn apply_moves_the_pixels_and_keeps_the_layer() {
        let size = photocraft_geom::Size { width: 16, height: 16 };
        let mut doc = Document::new("D", size, photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let mut l = photocraft_doc::Layer::new("L", LayerContent::Raster(red_square(PixelFormat::RGBA8)));
        l.blend = photocraft_color::BlendMode::Difference;
        l.opacity = 0.5;
        let id = l.id;
        doc.layers.push(l);
        let live = LiveTransform {
            layer: id,
            source: Arc::new(red_square(PixelFormat::RGBA8)),
            rest: None,
            mesh: Arc::new(quad_mesh([8.0, 8.0, 16.0, 16.0], [0.0, 0.0, 8.0, 8.0])),
        };
        let d = apply(&doc, &live);
        let l = d.layer(id).unwrap();
        assert_eq!((l.blend, l.opacity), (photocraft_color::BlendMode::Difference, 0.5));
        let s = l.surface().unwrap();
        assert_eq!(s.rgba(12, 12), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(s.rgba(3, 3)[3], 0.0);
        assert_eq!(live.damage(&doc), Rect::new(2, 2, 16, 16));
    }

    /// Hostile meshes (indices out of range, NaN, huge coordinates, degenerate triangles) draw
    /// nothing for the bad triangles and never panic.
    #[test]
    fn hostile_meshes_are_ignored() {
        let src = Arc::new(red_square(PixelFormat::RGBA8));
        for mesh in [
            LiveMesh { pos: vec![[0.0, 0.0]], uv: vec![[0.0, 0.0]], indices: vec![0, 1, 2] },
            LiveMesh { pos: vec![[f32::NAN, 0.0], [1.0, 0.0], [0.0, 1.0]], uv: vec![[0.0; 2]; 3], indices: vec![0, 1, 2] },
            LiveMesh { pos: vec![[0.0, 0.0], [1e30, 0.0], [0.0, 1e30]], uv: vec![[0.0; 2]; 3], indices: vec![0, 1, 2] },
            LiveMesh { pos: vec![[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]], uv: vec![[0.0; 2]; 3], indices: vec![0, 1, 2, 7] },
            LiveMesh::default(),
        ] {
            let live = LiveTransform { layer: LayerId(1), source: src.clone(), rest: None, mesh: Arc::new(mesh) };
            assert!(rasterize(&live, Rect::new(0, 0, 4, 4)).iter().all(|v| *v == 0.0));
            assert!(live.mesh.bounds().is_empty());
        }
    }
}
