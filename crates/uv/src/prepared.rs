//! Immutable, centred geometry and a bounded BVH shared by GPU upload and click picking.
use crate::{
    Error, Result, Scene,
    preview::{Camera, Materials, sample},
    topology::Topology,
};
use glam::DVec3;

pub struct Triangle {
    pub positions: [[f32; 3]; 3],
    pub normals: [[f32; 3]; 3],
    pub uv: [[f32; 2]; 3],
    pub face: [f32; 3],
    pub tangent: [f32; 3],
    pub bitangent: [f32; 3],
    pub material: usize,
    pub shell: u32,
}
struct Node {
    lo: DVec3,
    hi: DVec3,
    range: std::ops::Range<usize>,
    children: Option<[usize; 2]>,
}
pub struct Geometry {
    pub triangles: Vec<Triangle>,
    pub center: [f64; 3],
    pub diameter: f64,
    nodes: Vec<Node>,
    order: Vec<usize>,
}
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub triangle: usize,
    pub uv: [f64; 2],
    /// Surface position in the source model coordinate system.
    pub position: [f64; 3],
}
fn v(p: [f32; 3]) -> DVec3 {
    DVec3::from_array(p.map(f64::from))
}
fn f(p: DVec3) -> [f32; 3] {
    p.to_array().map(|x| x as f32)
}
pub fn rotate(p: DVec3, camera: Camera) -> DVec3 {
    let (sy, cy) = camera.yaw.sin_cos();
    let (sp, cp) = camera.pitch.sin_cos();
    let x = cy * p.x + sy * p.z;
    let z = -sy * p.x + cy * p.z;
    DVec3::new(x, cp * p.y - sp * z, sp * p.y + cp * z)
}
fn inverse(p: DVec3, camera: Camera) -> DVec3 {
    let (sy, cy) = camera.yaw.sin_cos();
    let (sp, cp) = camera.pitch.sin_cos();
    let y = cp * p.y + sp * p.z;
    let z = -sp * p.y + cp * p.z;
    DVec3::new(cy * p.x - sy * z, y, sy * p.x + cy * z)
}
impl Geometry {
    pub fn new(scene: &Scene, topology: &Topology) -> Result<Self> {
        scene.validate()?;
        if topology.triangle_shell.len() != scene.triangles.len() {
            return Err(Error("UV topology does not match geometry".into()));
        }
        let (lo, hi) = scene
            .positions
            .iter()
            .map(|p| DVec3::from_array(*p))
            .fold((DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)), |(lo, hi), p| (lo.min(p), hi.max(p)));
        let center = (lo + hi) * 0.5;
        let diameter = (hi - lo).length().max(1e-12);
        let mut triangles = Vec::new();
        triangles.try_reserve_exact(scene.triangles.len()).map_err(|_| Error("Not enough memory for prepared geometry".into()))?;
        for (id, t) in scene.triangles.iter().enumerate() {
            let [Some(a), Some(b), Some(c)] = t.corners.map(|c| scene.positions.get(c.vertex).map(|p| (DVec3::from_array(*p) - center) / diameter)) else {
                return Err(Error("Invalid geometry index".into()));
            };
            let e1 = b - a;
            let e2 = c - a;
            let face = e1.cross(e2).normalize_or_zero();
            let [u0, u1, u2] = t.corners.map(|c| c.uv);
            let (du1, dv1, du2, dv2) = (u1[0] - u0[0], u1[1] - u0[1], u2[0] - u0[0], u2[1] - u0[1]);
            let det = du1 * dv2 - du2 * dv1;
            let (tangent, bitangent) = if det.abs() > 1e-15 {
                (f(((e1 * dv2 - e2 * dv1) / det).normalize_or_zero()), f(((e2 * du1 - e1 * du2) / det).normalize_or_zero()))
            } else {
                ([0.0; 3], [0.0; 3])
            };
            let shell = topology.triangle_shell.get(id).copied().and_then(|s| u32::try_from(s).ok()).ok_or_else(|| Error("Invalid shell index".into()))?;
            triangles.push(Triangle {
                positions: [f(a), f(b), f(c)],
                normals: t.corners.map(|c| f(c.normal.map(DVec3::from_array).unwrap_or(face).normalize_or_zero())),
                uv: t.corners.map(|c| c.uv.map(|x| x as f32)),
                face: f(face),
                tangent,
                bitangent,
                material: t.material,
                shell,
            });
        }
        let len = triangles.len();
        let mut result = Self { triangles, center: center.to_array(), diameter, nodes: Vec::new(), order: (0..len).collect() };
        result.build(0..len, 0);
        Ok(result)
    }
    fn build(&mut self, range: std::ops::Range<usize>, depth: usize) -> usize {
        let mut lo = DVec3::splat(f64::INFINITY);
        let mut hi = DVec3::splat(f64::NEG_INFINITY);
        for id in self.order.get(range.clone()).unwrap_or_default() {
            if let Some(t) = self.triangles.get(*id) {
                for p in t.positions {
                    lo = lo.min(v(p));
                    hi = hi.max(v(p));
                }
            }
        }
        let id = self.nodes.len();
        self.nodes.push(Node { lo, hi, range: range.clone(), children: None });
        if range.len() > 8 && depth < 32 {
            let extent = hi - lo;
            let axis = if extent.x >= extent.y && extent.x >= extent.z {
                0
            } else if extent.y >= extent.z {
                1
            } else {
                2
            };
            let mid = range.start + range.len() / 2;
            let triangles = &self.triangles;
            if let Some(order) = self.order.get_mut(range.clone()) {
                order.select_nth_unstable_by(range.len() / 2, |a, b| {
                    let centroid = |id: usize| triangles.get(id).map_or(0.0, |t| t.positions.iter().map(|p| p.get(axis).copied().unwrap_or(0.0)).sum::<f32>());
                    centroid(*a).total_cmp(&centroid(*b)).then(a.cmp(b))
                });
            }
            let a = self.build(range.start..mid, depth + 1);
            let b = self.build(mid..range.end, depth + 1);
            if let Some(node) = self.nodes.get_mut(id) {
                node.children = Some([a, b]);
            }
        }
        id
    }
    /// The same world-space orbit target and orthographic projection as the reference renderer.
    pub fn camera_values(&self, camera: Camera, size: [u32; 2]) -> [f32; 16] {
        let target = camera.target.map_or(DVec3::ZERO, |p| (DVec3::from_array(p) - DVec3::from_array(self.center)) / self.diameter);
        let target = rotate(target, camera);
        let width = f64::from(size[0].max(1));
        let height = f64::from(size[1].max(1));
        let scale = width.min(height) * if camera.perspective { 1.2 } else { 0.8 * camera.zoom.clamp(0.01, 100.0) };
        let (sy, cy) = camera.yaw.sin_cos();
        let (sp, cp) = camera.pitch.sin_cos();
        [
            sy as f32,
            cy as f32,
            sp as f32,
            cp as f32,
            (2.0 * scale / width) as f32,
            (2.0 * scale / height) as f32,
            (camera.pan[0] / self.diameter - target.x) as f32,
            (camera.pan[1] / self.diameter - target.y) as f32,
            if camera.shaded && camera.channel == crate::preview::Channel::Material { 1.0 } else { 0.0 },
            if camera.authored_normals { 1.0 } else { 0.0 },
            if camera.flip_faces { -1.0 } else { 1.0 },
            match camera.channel {
                crate::preview::Channel::Material => 0.0,
                crate::preview::Channel::Normal => 2.0,
                _ => 1.0,
            },
            camera.normal_strength.clamp(0.0, 4.0) as f32,
            if camera.flip_normal_y { -1.0 } else { 1.0 },
            if camera.perspective { camera.distance() as f32 } else { 0.0 },
            target.z as f32,
        ]
    }
    /// Project normalized prepared-geometry coordinates into top-left-origin [0,1] view coordinates.
    pub fn project_normalized(&self, camera: Camera, position: [f64; 3], size: [u32; 2]) -> Option<[f64; 2]> {
        let p = rotate(DVec3::from_array(position), camera);
        let c = self.camera_values(camera, size);
        let depth = if camera.perspective { f64::from(c[14]) + f64::from(c[15]) - p.z } else { 1.0 };
        if !depth.is_finite() || depth <= camera.near() && camera.perspective {
            return None;
        }
        Some([((p.x + f64::from(c[6])) * f64::from(c[4]) / depth + 1.0) * 0.5, (1.0 - (p.y + f64::from(c[7])) * f64::from(c[5]) / depth) * 0.5])
    }
    /// Change the orbit target without moving any projected point in the current view.
    pub fn retarget(&self, mut camera: Camera, position: [f64; 3]) -> Camera {
        if position.iter().any(|x| !x.is_finite()) {
            return camera;
        }
        let delta = DVec3::from_array(position) - DVec3::from_array(camera.target.unwrap_or(self.center));
        let shift = rotate(delta, camera);
        camera.pan[0] += shift.x;
        camera.pan[1] += shift.y;
        if camera.perspective {
            let distance = (camera.distance() - shift.z / self.diameter).max(1.5e-6);
            camera.zoom = 1.5 / distance;
        }
        camera.target = Some(position);
        camera
    }
    pub fn pick(&self, scene: &Scene, camera: Camera, maps: &Materials<'_>, at: [f64; 2], size: [u32; 2]) -> Option<usize> {
        self.hit(scene, camera, maps, at, size).map(|hit| hit.triangle)
    }
    /// Visible surface and barycentrically interpolated UV for texture-space tools.
    pub fn hit(&self, scene: &Scene, camera: Camera, maps: &Materials<'_>, at: [f64; 2], size: [u32; 2]) -> Option<Hit> {
        if at.iter().any(|v| !v.is_finite()) || size.contains(&0) {
            return None;
        }
        let c = self.camera_values(camera, size);
        let x = (at[0] * 2.0 - 1.0) / f64::from(c[4]);
        let y = (1.0 - at[1] * 2.0) / f64::from(c[5]);
        let (origin, dir) = if camera.perspective {
            (
                inverse(DVec3::new(-f64::from(c[6]), -f64::from(c[7]), f64::from(c[14]) + f64::from(c[15])), camera),
                inverse(DVec3::new(x, y, -1.0).normalize(), camera),
            )
        } else {
            (inverse(DVec3::new(x - f64::from(c[6]), y - f64::from(c[7]), 2.0), camera), inverse(DVec3::new(0.0, 0.0, -1.0), camera))
        };
        let mut nearest = f64::INFINITY;
        let mut found = None;
        let mut stack = vec![0usize];
        while let Some(id) = stack.pop() {
            let node = self.nodes.get(id)?;
            let (mut near, mut far) = (0.0f64, nearest);
            for axis in 0..3 {
                let (o, d, l, h) = (origin[axis], dir[axis], node.lo[axis], node.hi[axis]);
                if d.abs() < 1e-15 {
                    if o < l || o > h {
                        far = -1.0;
                    }
                } else {
                    let (a, b) = ((l - o) / d, (h - o) / d);
                    near = near.max(a.min(b));
                    far = far.min(a.max(b));
                }
            }
            if near > far {
                continue;
            }
            if let Some(children) = node.children {
                stack.extend(children);
                continue;
            }
            for &id in self.order.get(node.range.clone()).unwrap_or_default() {
                let Some(t) = self.triangles.get(id) else { continue };
                if maps.hidden.contains(&t.material) || (camera.cull_backfaces && -v(t.face).dot(dir) * if camera.flip_faces { -1.0 } else { 1.0 } <= 0.0) {
                    continue;
                }
                let [a, b, vertex_c] = t.positions.map(v);
                let (e1, e2) = (b - a, vertex_c - a);
                let p = dir.cross(e2);
                let det = e1.dot(p);
                if det.abs() < 1e-15 {
                    continue;
                }
                let q = origin - a;
                let u = q.dot(p) / det;
                let cross = q.cross(e1);
                let vv = dir.dot(cross) / det;
                let distance = e2.dot(cross) / det;
                if u < -1e-8 || vv < -1e-8 || u + vv > 1.0 + 1e-8 || distance < 0.0 || distance >= nearest {
                    continue;
                }
                if camera.perspective {
                    let depth = f64::from(c[14]) + f64::from(c[15]) - rotate(a + e1 * u + e2 * vv, camera).z;
                    if depth < camera.near() || depth > camera.distance() + 2.0 {
                        continue;
                    }
                }
                let uv = std::array::from_fn(|i| f64::from(t.uv[0][i]) * (1.0 - u - vv) + f64::from(t.uv[1][i]) * u + f64::from(t.uv[2][i]) * vv);
                let alpha = maps.colors.get(t.material).and_then(Option::as_ref).map_or(1.0, |img| sample(img, uv)[3])
                    * scene.materials.get(t.material).map_or(1.0, |m| m.color[3]);
                if alpha < 0.5 {
                    continue;
                }
                nearest = distance;
                let position = (DVec3::from_array(self.center) + (a + e1 * u + e2 * vv) * self.diameter).to_array();
                found = Some(Hit { triangle: id, uv, position });
            }
        }
        found
    }
}
