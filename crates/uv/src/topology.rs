//! UV shell connectivity and edge protection masks.

use crate::{Error, MAX_PIXELS, Result, Scene, filled};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct Shell {
    pub material: usize,
    pub triangles: Vec<usize>,
    pub neighbors: BTreeSet<usize>,
}

#[derive(Clone, Debug)]
pub struct Boundary {
    pub shell: usize,
    pub uv: [[f64; 2]; 2],
    pub shared: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Topology {
    pub shells: Vec<Shell>,
    pub triangle_shell: Vec<usize>,
    pub boundaries: Vec<Boundary>,
    pub non_manifold_edges: usize,
}

#[derive(Clone, Copy)]
struct HalfEdge {
    triangle: usize,
    uv: [[f64; 2]; 2],
}

fn root(parents: &[usize], mut i: usize) -> usize {
    while let Some(&p) = parents.get(i) {
        if i == p {
            break;
        }
        i = p;
    }
    i
}

fn continuous(a: &HalfEdge, b: &HalfEdge) -> bool {
    a.uv.iter().flatten().zip(b.uv.iter().flatten()).all(|(x, y)| (x - y).abs() < 1e-7)
}

impl Topology {
    pub fn build(scene: &Scene) -> Result<Self> {
        scene.validate()?;
        let mut parents: Vec<usize> = (0..scene.triangles.len()).collect();
        let mut sizes = filled(scene.triangles.len(), 1usize)?;
        let mut edges: BTreeMap<(usize, usize), Vec<HalfEdge>> = BTreeMap::new();
        for (i, t) in scene.triangles.iter().enumerate() {
            for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                let (Some(a), Some(b)) = (t.corners.get(a), t.corners.get(b)) else { continue };
                if a.vertex == b.vertex {
                    continue;
                }
                let (key, uv) = if a.vertex < b.vertex { ((a.vertex, b.vertex), [a.uv, b.uv]) } else { ((b.vertex, a.vertex), [b.uv, a.uv]) };
                edges.entry(key).or_default().push(HalfEdge { triangle: i, uv });
            }
        }
        for list in edges.values() {
            if let [a, b] = list.as_slice()
                && continuous(a, b)
                && scene.triangles.get(a.triangle).map(|t| t.material) == scene.triangles.get(b.triangle).map(|t| t.material)
            {
                let (mut a, mut b) = (root(&parents, a.triangle), root(&parents, b.triangle));
                if a == b {
                    continue;
                }
                if sizes.get(a) < sizes.get(b) {
                    std::mem::swap(&mut a, &mut b);
                }
                let total = sizes.get(a).copied().unwrap_or(1).saturating_add(sizes.get(b).copied().unwrap_or(1));
                if let Some(p) = parents.get_mut(b) {
                    *p = a;
                }
                if let Some(size) = sizes.get_mut(a) {
                    *size = total;
                }
            }
        }
        let mut result = Self::default();
        let mut roots = BTreeMap::new();
        for (i, t) in scene.triangles.iter().enumerate() {
            let r = root(&parents, i);
            let shell = *roots.entry(r).or_insert_with(|| {
                let id = result.shells.len();
                result.shells.push(Shell { material: t.material, triangles: Vec::new(), neighbors: BTreeSet::new() });
                id
            });
            result.triangle_shell.push(shell);
            if let Some(s) = result.shells.get_mut(shell) {
                s.triangles.push(i);
            }
        }
        for list in edges.values() {
            if list.len() > 2 {
                result.non_manifold_edges += 1;
            }
            let same_uv = matches!(list.as_slice(), [a,b] if continuous(a,b));
            for a in list {
                let Some(&shell) = result.triangle_shell.get(a.triangle) else { continue };
                let peers: BTreeSet<usize> = list.iter().filter_map(|b| result.triangle_shell.get(b.triangle).copied()).filter(|s| *s != shell).collect();
                if let Some(s) = result.shells.get_mut(shell) {
                    s.neighbors.extend(peers.iter().copied());
                }
                if list.len() != 2 || !same_uv || !peers.is_empty() {
                    result.boundaries.push(Boundary { shell, uv: a.uv, shared: list.len() > 1 });
                }
            }
        }
        Ok(result)
    }

    /// All shells under a UV point, so stacked/mirrored UVs are never silently ambiguous.
    pub fn pick_uv(&self, scene: &Scene, uv: [f64; 2], material: usize) -> Vec<usize> {
        let mut shells = BTreeSet::new();
        for (i, t) in scene.triangles.iter().enumerate().filter(|(_, t)| t.material == material) {
            if barycentric(uv, t.corners.map(|c| c.uv)).is_some()
                && let Some(&s) = self.triangle_shell.get(i)
            {
                shells.insert(s);
            }
        }
        shells.into_iter().collect()
    }

    /// Rasterize the union first, then remove protected bands. Other selected islands cannot
    /// re-enable a protected seam. Distances use the actual width AND height of the texture.
    pub fn mask(&self, scene: &Scene, options: &MaskOptions) -> Result<Vec<f32>> {
        scene.validate()?;
        let MaskOptions { width, height, material, tile, selected, guard, horizontal, vertical, feather, open_borders } = options;
        let (w, h) = (*width as usize, *height as usize);
        let n = w.checked_mul(h).filter(|n| *n > 0 && *n <= MAX_PIXELS).ok_or_else(|| Error("UV mask is limited to 16M pixels".into()))?;
        if *material >= scene.materials.len() || selected.iter().any(|i| self.shells.get(*i).is_none_or(|s| s.material != *material)) {
            return Err(Error("Selected shell does not belong to this material".into()));
        }
        if [*guard, *horizontal, *vertical, *feather].iter().any(|v| !v.is_finite() || !(0.0..=1024.0).contains(v)) {
            return Err(Error("Margins and inward feather must be in 0..1024 pixels".into()));
        }
        let point = |uv: [f64; 2]| [(uv[0] - f64::from(tile[0])) * w as f64, (1.0 - (uv[1] - f64::from(tile[1]))) * h as f64];
        let mut samples = filled(n, 0u8)?;
        for shell in selected.iter().filter_map(|i| self.shells.get(*i)) {
            for t in shell.triangles.iter().filter_map(|i| scene.triangles.get(*i)) {
                let pts = t.corners.map(|c| point(c.uv));
                let (x0, y0, x1, y1) = bounds(&pts, 0.0, w, h);
                for y in y0..y1 {
                    for x in x0..x1 {
                        // Four samples give subpixel shells a useful, antialiased boundary.
                        let coverage = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)]
                            .iter()
                            .enumerate()
                            .fold(0u8, |bits, (i, (dx, dy))| if barycentric([x as f64 + dx, y as f64 + dy], pts).is_some() { bits | (1 << i) } else { bits });
                        if let Some(v) = samples.get_mut(y * w + x) {
                            *v |= coverage;
                        }
                    }
                }
            }
        }
        let mut mask = filled(n, 0.0f32)?;
        for (value, bits) in mask.iter_mut().zip(samples) {
            *value = bits.count_ones() as f32 * 0.25;
        }
        // Tiny conservative pad covers the entire square texel, not just its center.
        let pad = std::f64::consts::FRAC_1_SQRT_2;
        let radii = [*guard + *horizontal + pad, *guard + *vertical + pad];
        let radius = radii[0].max(radii[1]) + *feather;
        if *guard > 0.0 || *horizontal > 0.0 || *vertical > 0.0 || *feather > 0.0 {
            for e in self.boundaries.iter().filter(|e| selected.contains(&e.shell) && (e.shared || *open_borders)) {
                let pts = e.uv.map(point);
                let (x0, y0, x1, y1) = bounds(&pts, radius, w, h);
                let scaled = |p: [f64; 2]| [p[0] / radii[0], p[1] / radii[1]];
                let scaled_pts = pts.map(scaled);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let delta = segment_delta(scaled([x as f64 + 0.5, y as f64 + 0.5]), scaled_pts);
                        let unit = delta[0].hypot(delta[1]);
                        // Exact elliptical hard margin; feather is a radial pixel distance outside it.
                        // Equal axes reduce to the previous isotropic distance, including texel padding.
                        let d = if unit <= 1.0 { 0.0 } else { (delta[0] * radii[0]).hypot(delta[1] * radii[1]) * (1.0 - 1.0 / unit) };
                        let alpha = if d <= 0.0 {
                            0.0
                        } else if *feather > 0.0 {
                            (d / *feather).clamp(0.0, 1.0) as f32
                        } else {
                            1.0
                        };
                        if let Some(v) = mask.get_mut(y * w + x) {
                            *v = v.min(alpha);
                        }
                    }
                }
            }
        }
        Ok(mask)
    }
}

pub struct MaskOptions {
    pub width: u32,
    pub height: u32,
    pub material: usize,
    pub tile: [i32; 2],
    pub selected: BTreeSet<usize>,
    pub guard: f64,
    pub horizontal: f64,
    pub vertical: f64,
    pub feather: f64,
    pub open_borders: bool,
}

pub fn barycentric(p: [f64; 2], t: [[f64; 2]; 3]) -> Option<[f64; 3]> {
    let [a, b, c] = t;
    let cross = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    let area = cross(a, b, c);
    if area.abs() < 1e-15 {
        return None;
    }
    let v = cross(a, p, c) / area;
    let w = cross(a, b, p) / area;
    let u = 1.0 - v - w;
    if u >= -1e-9 && v >= -1e-9 && w >= -1e-9 { Some([u, v, w]) } else { None }
}

fn bounds<const N: usize>(pts: &[[f64; 2]; N], pad: f64, w: usize, h: usize) -> (usize, usize, usize, usize) {
    let lo = pts.iter().fold([f64::INFINITY; 2], |a, p| [a[0].min(p[0]), a[1].min(p[1])]);
    let hi = pts.iter().fold([f64::NEG_INFINITY; 2], |a, p| [a[0].max(p[0]), a[1].max(p[1])]);
    (
        (lo[0] - pad).floor().clamp(0.0, w as f64) as usize,
        (lo[1] - pad).floor().clamp(0.0, h as f64) as usize,
        (hi[0] + pad).ceil().clamp(0.0, w as f64) as usize,
        (hi[1] + pad).ceil().clamp(0.0, h as f64) as usize,
    )
}

fn segment_delta(p: [f64; 2], [a, b]: [[f64; 2]; 2]) -> [f64; 2] {
    let d = [b[0] - a[0], b[1] - a[1]];
    let len = d[0] * d[0] + d[1] * d[1];
    let t = if len > 1e-15 { (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / len).clamp(0.0, 1.0) } else { 0.0 };
    [p[0] - a[0] - t * d[0], p[1] - a[1] - t * d[1]]
}
