//! Art History Brush: paint stylized strokes from a history state.
//!
//! Photoshop styles:
//! - Tight Short, Tight Medium, Tight Long
//! - Loose Medium, Loose Long
//! - Dab
//! - Tight Curl, Loose Curl
//!
//! Area controls the scatter radius around the brush, and Tolerance limits application by color difference.

use photocraft_color::{BlendMode, PixelFormat};
use photocraft_geom::Rect;
use photocraft_paint::retouch::{Region, alpha_index, mix_back, over_native};
use photocraft_paint::{Stroke, dabs};
use photocraft_raster::{Surface, from_rgba_into, to_rgba};
use serde_json::{Value, json};

use super::{
    bad, blend_param, damage_json, history_source, num, parse_brush, run_stroke, string,
};
use crate::{EngineError, Result, Session};

/// Maximum stroke bounding box pixels to prevent runaway allocations.
const MAX_STROKE_PIXELS: u64 = 67_108_864;

const STREAM_DX: u64 = 201;
const STREAM_DY: u64 = 202;
const STREAM_ANGLE: u64 = 203;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtStyle {
    TightShort,
    TightMedium,
    TightLong,
    LooseMedium,
    LooseLong,
    Dab,
    TightCurl,
    LooseCurl,
}

impl ArtStyle {
    pub fn parse(s: &str, cmd: &str) -> Result<Self> {
        match s.to_ascii_lowercase().replace([' ', '_', '-'], "").as_str() {
            "tightshort" => Ok(Self::TightShort),
            "tightmedium" => Ok(Self::TightMedium),
            "tightlong" => Ok(Self::TightLong),
            "loosemedium" => Ok(Self::LooseMedium),
            "looselong" => Ok(Self::LooseLong),
            "dab" => Ok(Self::Dab),
            "tightcurl" => Ok(Self::TightCurl),
            "loosecurl" => Ok(Self::LooseCurl),
            _ => Err(bad(
                cmd,
                format!(
                    "unknown style `{s}` (tightShort|tightMedium|tightLong|looseMedium|looseLong|dab|tightCurl|looseCurl)"
                ),
            )),
        }
    }
}

pub fn run(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "paint.artHistoryBrush";
    let (mut stroke, id) = parse_brush(s, p, CMD)?;
    let mode = blend_param(p, CMD)?;
    let style = ArtStyle::parse(string(p, "style", "tightMedium"), CMD)?;
    let area = num(p, "area", 50.0).clamp(1.0, 500.0);
    let tolerance = num(p, "tolerance", 0.0).clamp(0.0, 100.0) / 100.0;
    let src_surface = history_source(s, id, p, CMD)?;

    let seed = p.get("seed").and_then(Value::as_u64).unwrap_or_else(|| {
        let bytes = p.get("points").map(std::string::ToString::to_string).unwrap_or_default();
        photocraft_paint::rng::seed_from_bytes(bytes.as_bytes())
    });
    stroke.brush.seed = seed;

    let dmg = run_stroke(s, "Art History Brush", id, p, |_pre, surf, sel, lock| {
        paint_art_history(surf, &src_surface, &stroke, style, area, tolerance, seed, mode, sel, lock)
    })?;

    Ok(json!({ "damage": damage_json(dmg) }))
}

#[allow(clippy::too_many_arguments)]
fn paint_art_history(
    target: &mut Surface,
    src_surface: &Surface,
    stroke: &Stroke,
    style: ArtStyle,
    area: f32,
    tolerance: f32,
    seed: u64,
    mode: BlendMode,
    selection: Option<&Surface>,
    lock_transparency: bool,
) -> Result<Rect> {
    let ds = dabs(stroke);
    if ds.is_empty() {
        return Ok(Rect::EMPTY);
    }

    let fmt = target.format();
    let src = if src_surface.format() == fmt {
        src_surface.clone()
    } else {
        src_surface.convert(fmt)
    };

    let brush_size = stroke.brush.size.max(1.0);
    let brush_radius = (brush_size * 0.5).max(0.5);
    let brush_hardness = stroke.brush.hardness.clamp(0.0, 1.0);

    let reach = match style {
        ArtStyle::Dab => brush_radius,
        ArtStyle::TightShort => brush_radius * 2.0,
        ArtStyle::TightMedium | ArtStyle::LooseMedium => brush_radius * 3.5,
        ArtStyle::TightLong | ArtStyle::LooseLong => brush_radius * 6.5,
        ArtStyle::TightCurl => brush_radius * 2.5,
        ArtStyle::LooseCurl => brush_radius * 4.0,
    };

    let pad = (area + reach + 4.0).ceil() as i32;
    let base_bounds = ds.iter().fold(Rect::EMPTY, |acc, d| {
        let r = Rect::new(
            (d.center.x - d.radius as f64).floor() as i32,
            (d.center.y - d.radius as f64).floor() as i32,
            (d.center.x + d.radius as f64).ceil() as i32,
            (d.center.y + d.radius as f64).ceil() as i32,
        );
        acc.union(&r)
    });

    if base_bounds.is_empty() {
        return Ok(Rect::EMPTY);
    }

    let outer = Rect::new(
        base_bounds.x0 - pad,
        base_bounds.y0 - pad,
        base_bounds.x1 + pad,
        base_bounds.y1 + pad,
    );

    let w = outer.width() as u64;
    let h = outer.height() as u64;
    if w.saturating_mul(h) > MAX_STROKE_PIXELS {
        return Err(EngineError::BadParams {
            cmd: "paint.artHistoryBrush".into(),
            msg: format!("stroke too large ({w}×{h} px, max {MAX_STROKE_PIXELS})"),
        });
    }

    let orig = Region::read(target, outer);
    let mut work = orig.clone();
    let mut actual_damage = Rect::EMPTY;

    let n = fmt.channels();
    let a_idx = alpha_index(&fmt);
    let mut src_px = vec![0.0f32; n];

    for (i, d) in ds.iter().enumerate() {
        let idx = (d.index ^ (i as u64)).wrapping_add(1);
        let scatter = (area * 0.1).min(brush_radius * 0.35);
        let jx = photocraft_paint::rng::rand_signed(seed, idx, STREAM_DX) * scatter;
        let jy = photocraft_paint::rng::rand_signed(seed, idx, STREAM_DY) * scatter;

        let cx = d.center.x as f32 + jx;
        let cy = d.center.y as f32 + jy;

        let ix = d.center.x.round() as i32;
        let iy = d.center.y.round() as i32;

        if !work.rect.contains(ix, iy) {
            continue;
        }

        src.read_pixel(ix, iy, &mut src_px);

        if tolerance > 0.0 {
            let dst_px = work.px(ix, iy);
            let s_rgba = to_rgba(&fmt, &src_px);
            let d_rgba = to_rgba(&fmt, dst_px);
            let diff_sq = (s_rgba[0] - d_rgba[0]).powi(2)
                + (s_rgba[1] - d_rgba[1]).powi(2)
                + (s_rgba[2] - d_rgba[2]).powi(2);
            let diff = (diff_sq / 3.0).sqrt();
            if diff > tolerance {
                continue;
            }
        }

        let mark_flow = (d.alpha * stroke.brush.flow).clamp(0.01, 1.0);
        let angle_jitter = photocraft_paint::rng::rand_signed(seed, idx, STREAM_ANGLE);

        let mark_rect = apply_art_mark(
            &mut work,
            &fmt,
            cx,
            cy,
            d.radius.max(0.5),
            brush_hardness,
            d.angle,
            angle_jitter,
            style,
            &src_px,
            mark_flow,
            mode,
            lock_transparency,
            a_idx,
        );

        actual_damage = actual_damage.union(&mark_rect);
    }

    let final_damage = actual_damage.intersect(&work.rect);
    if !final_damage.is_empty() {
        mix_back(
            target,
            &orig,
            &work,
            final_damage,
            stroke.brush.opacity,
            selection,
            lock_transparency,
        );
    }

    Ok(final_damage)
}

#[allow(clippy::too_many_arguments)]
fn apply_art_mark(
    work: &mut Region,
    fmt: &PixelFormat,
    cx: f32,
    cy: f32,
    radius: f32,
    hardness: f32,
    stroke_angle: f32,
    angle_jitter: f32,
    style: ArtStyle,
    src_px: &[f32],
    flow: f32,
    blend: BlendMode,
    lock_transparency: bool,
    a_idx: Option<usize>,
) -> Rect {
    match style {
        ArtStyle::Dab => {
            let r = radius;
            let x0 = (cx - r - 1.0).floor() as i32;
            let y0 = (cy - r - 1.0).floor() as i32;
            let x1 = (cx + r + 2.0).ceil() as i32;
            let y1 = (cy + r + 2.0).ceil() as i32;
            let mark_rect = Rect::new(x0, y0, x1, y1).intersect(&work.rect);
            if mark_rect.is_empty() {
                return Rect::EMPTY;
            }

            for y in mark_rect.y0..mark_rect.y1 {
                for x in mark_rect.x0..mark_rect.x1 {
                    let dist = (x as f32 - cx).hypot(y as f32 - cy);
                    let cov = brush_falloff(dist, r, hardness) * flow;
                    if cov > 0.0 {
                        blend_pixel(work, fmt, x, y, src_px, cov, blend, lock_transparency, a_idx);
                    }
                }
            }
            mark_rect
        }
        ArtStyle::TightShort
        | ArtStyle::TightMedium
        | ArtStyle::TightLong
        | ArtStyle::LooseMedium
        | ArtStyle::LooseLong => {
            let (len_factor, theta) = match style {
                ArtStyle::TightShort => (2.0, stroke_angle + angle_jitter * 0.25),
                ArtStyle::TightMedium => (3.5, stroke_angle + angle_jitter * 0.35),
                ArtStyle::TightLong => (6.0, stroke_angle + angle_jitter * 0.45),
                ArtStyle::LooseMedium => (3.5, angle_jitter * std::f32::consts::PI),
                ArtStyle::LooseLong => (6.5, angle_jitter * std::f32::consts::PI),
                _ => (3.0, stroke_angle),
            };

            let half_len = (radius * len_factor).max(1.0);
            let half_w = (radius * 0.6).max(0.5);

            let (sin_t, cos_t) = theta.sin_cos();
            let p0x = cx - cos_t * half_len;
            let p0y = cy - sin_t * half_len;
            let p1x = cx + cos_t * half_len;
            let p1y = cy + sin_t * half_len;

            let x0 = (p0x.min(p1x) - half_w - 1.0).floor() as i32;
            let y0 = (p0y.min(p1y) - half_w - 1.0).floor() as i32;
            let x1 = (p0x.max(p1x) + half_w + 2.0).ceil() as i32;
            let y1 = (p0y.max(p1y) + half_w + 2.0).ceil() as i32;

            let mark_rect = Rect::new(x0, y0, x1, y1).intersect(&work.rect);
            if mark_rect.is_empty() {
                return Rect::EMPTY;
            }

            let vx = p1x - p0x;
            let vy = p1y - p0y;
            let len_sq = vx.mul_add(vx, vy * vy).max(1e-4);

            for y in mark_rect.y0..mark_rect.y1 {
                for x in mark_rect.x0..mark_rect.x1 {
                    let fx = x as f32;
                    let fy = y as f32;
                    let t = ((fx - p0x).mul_add(vx, (fy - p0y) * vy) / len_sq).clamp(0.0, 1.0);
                    let qx = p0x + t * vx;
                    let qy = p0y + t * vy;
                    let dist = (fx - qx).hypot(fy - qy);
                    let cov = brush_falloff(dist, half_w, hardness) * flow;
                    if cov > 0.0 {
                        blend_pixel(work, fmt, x, y, src_px, cov, blend, lock_transparency, a_idx);
                    }
                }
            }
            mark_rect
        }
        ArtStyle::TightCurl | ArtStyle::LooseCurl => {
            let (curl_r, sweep) = match style {
                ArtStyle::TightCurl => (radius * 1.2, std::f32::consts::PI * 1.3),
                ArtStyle::LooseCurl => (radius * 2.2, std::f32::consts::PI * 1.8),
                _ => (radius, std::f32::consts::PI),
            };

            let theta0 = stroke_angle + angle_jitter * 0.8;
            let (sin0, cos0) = theta0.sin_cos();
            let center_x = cx - cos0 * curl_r;
            let center_y = cy - sin0 * curl_r;
            let half_w = (radius * 0.5).max(0.5);

            let max_r = curl_r + half_w + 1.0;
            let x0 = (center_x - max_r).floor() as i32;
            let y0 = (center_y - max_r).floor() as i32;
            let x1 = (center_x + max_r + 1.0).ceil() as i32;
            let y1 = (center_y + max_r + 1.0).ceil() as i32;

            let mark_rect = Rect::new(x0, y0, x1, y1).intersect(&work.rect);
            if mark_rect.is_empty() {
                return Rect::EMPTY;
            }

            for y in mark_rect.y0..mark_rect.y1 {
                for x in mark_rect.x0..mark_rect.x1 {
                    let fx = x as f32;
                    let fy = y as f32;
                    let dx = fx - center_x;
                    let dy = fy - center_y;
                    let dist_center = dx.hypot(dy);

                    let ang = dy.atan2(dx);
                    let mut rel_ang = ang - theta0;
                    while rel_ang < 0.0 {
                        rel_ang += std::f32::consts::TAU;
                    }
                    while rel_ang >= std::f32::consts::TAU {
                        rel_ang -= std::f32::consts::TAU;
                    }

                    let dist = if rel_ang <= sweep {
                        (dist_center - curl_r).abs()
                    } else {
                        let d_start = (fx - (center_x + cos0 * curl_r))
                            .hypot(fy - (center_y + sin0 * curl_r));
                        let end_ang = theta0 + sweep;
                        let end_x = center_x + end_ang.cos() * curl_r;
                        let end_y = center_y + end_ang.sin() * curl_r;
                        let d_end = (fx - end_x).hypot(fy - end_y);
                        d_start.min(d_end)
                    };

                    let cov = brush_falloff(dist, half_w, hardness) * flow;
                    if cov > 0.0 {
                        blend_pixel(work, fmt, x, y, src_px, cov, blend, lock_transparency, a_idx);
                    }
                }
            }
            mark_rect
        }
    }
}

#[inline]
fn brush_falloff(dist: f32, radius: f32, hardness: f32) -> f32 {
    if dist >= radius || radius <= 0.0 {
        return 0.0;
    }
    let inner = radius * hardness;
    if dist <= inner {
        1.0
    } else {
        (1.0 - (dist - inner) / (radius - inner).max(1e-4)).clamp(0.0, 1.0)
    }
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn blend_pixel(
    work: &mut Region,
    fmt: &PixelFormat,
    x: i32,
    y: i32,
    src_px: &[f32],
    cov: f32,
    blend: BlendMode,
    lock_transparency: bool,
    a_idx: Option<usize>,
) {
    let n = work.ch;
    let px = work.px_mut(x, y);
    if lock_transparency && a_idx.is_some_and(|a| px.get(a).copied().unwrap_or(0.0) <= 0.0) {
        return;
    }

    if blend == BlendMode::Normal {
        over_native(fmt, px, src_px, cov, lock_transparency);
    } else {
        let d = to_rgba(fmt, px);
        let s = to_rgba(fmt, src_px);
        let mut o = photocraft_color::blend::composite(blend, d, s, cov);
        if lock_transparency {
            o[3] = d[3];
        }
        let mut enc = [0.0f32; 8];
        from_rgba_into(fmt, o, &mut enc);
        for c in 0..n {
            if lock_transparency && Some(c) == a_idx {
                continue;
            }
            if let Some(dst) = px.get_mut(c)
                && let Some(val) = enc.get(c)
            {
                *dst = *val;
            }
        }
    }
}
