//! Filter › Render › Lens Flare.
//!
//! A flare is built from elements placed along its axis: the line from the flare centre (t = 0)
//! through the centre of the image (t = 1), as in a real lens, where the internal reflections line
//! up with the optical centre. Each lens has its own set, measured from Photoshop's output
//! (`lens_flare_data.rs`):
//!
//! - radial elements: a colour profile by distance around a point on the axis (the core, glows,
//!   rings and the reflection discs, with their coloured rims);
//! - streaks: straight lines centred on the axis (Movie Prime's horizontal and diagonal streaks, the
//!   105 mm Prime's spike);
//! - rays: thin one-pixel rays out of the centre, drawn without antialiasing as Photoshop does.
//!
//! Sizes are in thousandths of the image diagonal, so a flare scales with the image. Brightness
//! scales the light, which is then screened onto the pixels.

use photocraft_geom::Rect;

use crate::LensType;
use crate::fxutil::{rgba, set_rgba, smoothstep};
use crate::image::Image;
use crate::{Ctx, lens_flare_data as data};

/// A colour profile by distance (in diagonal thousandths) around a point on the axis; zero
/// outside the first and last knot, linear between knots.
pub(crate) struct Radial {
    pub t: f32,
    /// Offset across the axis, in diagonal thousandths.
    pub perp: f32,
    pub knots: &'static [(f32, [f32; 3])],
}

/// A straight streak centred on the axis: full strength along `half · (1 − taper)`, fading to
/// nothing at `half`; a Gaussian `width` across.
pub(crate) struct Streak {
    pub t: f32,
    pub perp: f32,
    /// Degrees, clockwise from the +x axis (image y points down).
    pub angle: f32,
    pub half: f32,
    pub taper: f32,
    pub width: f32,
    pub rgb: [f32; 3],
}

/// A one-pixel ray out of the flare centre from radius `r0` to `r1`, fading in and out over
/// `fade` of its length.
pub(crate) struct Ray {
    pub angle: f32,
    pub r0: f32,
    pub r1: f32,
    pub fade: f32,
    pub rgb: [f32; 3],
}

struct Design {
    radial: &'static [Radial],
    streaks: &'static [Streak],
    rays: &'static [Ray],
}

fn design(lens: LensType) -> Design {
    match lens {
        LensType::Zoom => Design { radial: data::ZOOM_RADIAL, streaks: data::ZOOM_STREAKS, rays: data::ZOOM_RAYS },
        LensType::Prime35 => Design { radial: data::PRIME35_RADIAL, streaks: data::PRIME35_STREAKS, rays: data::PRIME35_RAYS },
        LensType::Prime105 => Design { radial: data::PRIME105_RADIAL, streaks: data::PRIME105_STREAKS, rays: data::PRIME105_RAYS },
        LensType::MoviePrime => Design { radial: data::MOVIE_PRIME_RADIAL, streaks: data::MOVIE_PRIME_STREAKS, rays: data::MOVIE_PRIME_RAYS },
    }
}

/// The profile at distance `d`; `None` outside it.
fn profile(knots: &[(f32, [f32; 3])], d: f32) -> Option<[f32; 3]> {
    let (first, last) = (knots.first()?, knots.last()?);
    if d < first.0 || d > last.0 {
        return None;
    }
    // The first knot at or beyond `d` (binary search: profiles have up to a few hundred knots).
    let i = knots.partition_point(|k| k.0 < d);
    let Some(hi) = knots.get(i) else { return Some(last.1) };
    let Some(lo) = i.checked_sub(1).and_then(|j| knots.get(j)) else { return Some(hi.1) };
    let span = hi.0 - lo.0;
    let f = if span > 0.0 { (d - lo.0) / span } else { 0.0 };
    Some(std::array::from_fn(|k| lo.1[k] + (hi.1[k] - lo.1[k]) * f))
}

/// Flare geometry in document pixels.
struct Geometry {
    centre: (f32, f32),
    /// Axis: image centre minus flare centre.
    axis: (f32, f32),
    /// Unit normal to the axis.
    normal: (f32, f32),
    /// Pixels per diagonal thousandth.
    unit: f32,
}

impl Geometry {
    fn point(&self, t: f32, perp: f32) -> (f32, f32) {
        (self.centre.0 + t * self.axis.0 + perp * self.unit * self.normal.0, self.centre.1 + t * self.axis.1 + perp * self.unit * self.normal.1)
    }
}

pub(crate) fn lens_flare(src: &Image, out: Rect, ctx: &Ctx, brightness: f32, centre: (f32, f32), lens: LensType) -> Vec<f32> {
    let n = src.ch;
    let b = ctx.bounds;
    let (bw, bh) = (b.width() as f32, b.height() as f32);
    let c = (b.x0 as f32 + centre.0 * bw, b.y0 as f32 + centre.1 * bh);
    let mid = (b.x0 as f32 + bw / 2.0, b.y0 as f32 + bh / 2.0);
    let axis = (mid.0 - c.0, mid.1 - c.1);
    let len = axis.0.hypot(axis.1);
    // At the image centre the axis has no direction; the offsets across it are tiny anyway.
    let normal = if len > 1e-6 { (-axis.1 / len, axis.0 / len) } else { (0.0, 1.0) };
    let g = Geometry { centre: c, axis, normal, unit: (bw.hypot(bh) / 1000.0).max(1e-6) };
    let bright = brightness.clamp(10.0, 300.0) / 100.0;
    let d = design(lens);

    let (w, h) = (out.width() as usize, out.height() as usize);
    let mut light = vec![[0.0f32; 3]; w * h];
    let (ox, oy) = (out.x0 as f32, out.y0 as f32);

    // Radial elements, each only over the pixels it can reach.
    for e in d.radial {
        let Some(reach) = e.knots.last().map(|k| k.0 * g.unit) else { continue };
        let p = g.point(e.t, e.perp);
        let x0 = ((p.0 - reach - ox).floor().max(0.0) as usize).min(w);
        let x1 = ((p.0 + reach - ox).ceil().max(0.0) as usize).min(w);
        let y0 = ((p.1 - reach - oy).floor().max(0.0) as usize).min(h);
        let y1 = ((p.1 + reach - oy).ceil().max(0.0) as usize).min(h);
        for y in y0..y1 {
            let fy = oy + y as f32 + 0.5 - p.1;
            for x in x0..x1 {
                let fx = ox + x as f32 + 0.5 - p.0;
                if let Some(v) = profile(e.knots, fx.hypot(fy) / g.unit)
                    && let Some(px) = light.get_mut(y * w + x)
                {
                    for (p, v) in px.iter_mut().zip(v) {
                        *p += v;
                    }
                }
            }
        }
    }

    for s in d.streaks {
        let p = g.point(s.t, s.perp);
        let (sin, cos) = s.angle.to_radians().sin_cos();
        let half = s.half.max(1e-3);
        let taper = s.taper.clamp(0.01, 1.0);
        let width = s.width.max(1e-3);
        for (i, px) in light.iter_mut().enumerate() {
            let fx = ox + (i % w) as f32 + 0.5 - p.0;
            let fy = oy + (i / w) as f32 + 0.5 - p.1;
            let across = (-fx * sin + fy * cos) / g.unit / width;
            if across.abs() > 4.0 {
                continue;
            }
            let along = ((fx * cos + fy * sin) / g.unit).abs();
            let k = (1.0 - smoothstep(half * (1.0 - taper), half, along)) * (-across * across).exp();
            for (p, c) in px.iter_mut().zip(s.rgb) {
                *p += k * c;
            }
        }
    }

    // Rays: walk each ray in short steps and light every pixel it crosses once.
    for r in d.rays {
        let (sin, cos) = r.angle.to_radians().sin_cos();
        let (r0, r1) = (r.r0 * g.unit, r.r1 * g.unit);
        let length = (r1 - r0).max(1e-6);
        let fade = r.fade.max(1e-3);
        let steps = ((r1 - r0) / 0.35).ceil().clamp(0.0, 1e6) as usize;
        let mut last = None;
        for i in 0..steps {
            let rad = r0 + i as f32 * 0.35;
            let (px, py) = ((c.0 + rad * cos).floor(), (c.1 + rad * sin).floor());
            if last == Some((px, py)) {
                continue;
            }
            last = Some((px, py));
            let (lx, ly) = (px - ox, py - oy);
            if lx < 0.0 || ly < 0.0 || lx >= w as f32 || ly >= h as f32 {
                continue;
            }
            let pos = (rad - r0) / length;
            let env = ((pos.min(1.0 - pos)) / fade).clamp(0.0, 1.0);
            if let Some(v) = light.get_mut(ly as usize * w + lx as usize) {
                for (p, c) in v.iter_mut().zip(r.rgb) {
                    *p += env * c;
                }
            }
        }
    }

    let mut res = src.crop(out);
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let Some(f) = light.get(i) else { break };
        let mut c = rgba(ctx, px);
        for k in 0..3 {
            let a = (f[k] * bright).clamp(0.0, 1.0);
            // Screen (c + a·(1 − c)) keeps display-referred values below white; a 32-bit value
            // already above white is left as is rather than pulled down towards it.
            c[k] += a * (1.0 - c[k]).max(0.0);
        }
        set_rgba(ctx, px, c);
    }
    res
}
