//! Camera Raw's **Auto**: reads the picture and chooses the Light sliders for it.
//!
//! Original method (not a copy of any other editor's Auto, whose numbers are proprietary).
//! It works closed-loop on a small thumbnail: each slider is solved by bisection against a
//! statistic of what [`develop`](super::develop) actually produces, so the result is right for
//! this pipeline whatever the sliders' exact curves are. The goals, in order:
//!
//! 1. **Exposure** brings the median brightness toward a mid-tone target, damped (a dark,
//!    moody picture is only partly lifted) and limited so it never blows out more than a few
//!    percent of the pixels.
//! 2. **Highlights** recover a bright top end (98th percentile above 0.88).
//! 3. **Shadows** open a crushed bottom (5th percentile below 0.10).
//! 4. **Whites** stretch a top end that stops short of white; **Blacks** deepen a milky floor,
//!    or lift a floor that is clipped.
//! 5. **Contrast** nudges a flat picture up and a very contrasty one down.
//!
//! Everything is read off the document's own (display-encoded) values. Empty, uniform, black
//! or non-finite input gives all zeros, never a panic.

use super::{CameraRaw, develop, luma};

/// The Light sliders chosen by [`auto_tone`], in Camera Raw's units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AutoTone {
    /// Stops, −5..=5.
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
}

impl AutoTone {
    /// Writes the six Light sliders into `p`; every other setting stays as it is.
    pub fn apply(&self, p: &mut CameraRaw) {
        p.exposure = self.exposure;
        p.contrast = self.contrast;
        p.highlights = self.highlights;
        p.shadows = self.shadows;
        p.whites = self.whites;
        p.blacks = self.blacks;
    }
    pub fn is_neutral(&self) -> bool {
        *self == AutoTone::default()
    }
}

/// Longest side of the analysis thumbnail.
const THUMB: usize = 112;
/// Median brightness Exposure aims for (display-encoded, about 18 % grey in light).
const MEDIAN_TARGET: f32 = 0.46;
/// Share of the way to that target Exposure goes.
const EXPOSURE_DAMPING: f32 = 0.7;
/// Most pixels (share) Exposure may clip on the way up.
const MAX_CLIPPED: f32 = 0.04;
const P98_LIMIT: f32 = 0.88;
const P5_FLOOR: f32 = 0.10;
const WHITE_TARGET: f32 = 0.985;
const BLACK_TARGET: f32 = 0.02;
const SPREAD_TARGET: f32 = 0.62;

/// How the thumbnail is rendered with a candidate set of settings: `(pixels, w, h, settings)`.
/// [`auto_tone_develop`] uses [`develop`]; a caller whose preview does more (the raw-open
/// dialog applies Shadows and Highlights to the sensor data) passes its own.
pub type Render<'a> = &'a dyn Fn(&[[f32; 4]], usize, usize, &CameraRaw) -> Vec<[f32; 4]>;

/// Auto for straight RGBA pixels (`w × h`, display-encoded) with the settings `base` otherwise
/// in force (white balance, Dehaze, curves… are kept in the loop; the Light sliders start at 0).
pub fn auto_tone_develop(px: &[[f32; 4]], w: usize, h: usize, base: &CameraRaw, float: bool) -> AutoTone {
    auto_tone(px, w, h, base, &|src, tw, th, p| {
        let mut v = src.to_vec();
        develop(&mut v, tw, th, p, float);
        v
    })
}

/// [`auto_tone_develop`] with a caller-supplied renderer.
pub fn auto_tone(px: &[[f32; 4]], w: usize, h: usize, base: &CameraRaw, render: Render) -> AutoTone {
    solve_tone(px, w, h, base, render).unwrap_or_default()
}

fn solve_tone(px: &[[f32; 4]], w: usize, h: usize, base: &CameraRaw, render: Render) -> Option<AutoTone> {
    let (thumb, tw, th) = thumbnail(px, w, h)?;
    let mut p = base.clone();
    AutoTone::default().apply(&mut p);
    p.pixel_scale = (tw as f32 / w.max(1) as f32).clamp(0.01, 1.0);
    let eval = |p: &CameraRaw| stats(&render(&thumb, tw, th, p));
    let s0 = eval(&p)?;
    // A black frame or a flat field has nothing to read.
    if s0.hi < 0.03 || s0.hi - s0.lo < 0.01 {
        return None;
    }
    let mut out = AutoTone::default();

    // 1. Exposure.
    let full = solve(-4.0, 4.0, MEDIAN_TARGET, |e| eval(&CameraRaw { exposure: e, ..p.clone() }).map(|s| s.median))?;
    let mut e = (full * EXPOSURE_DAMPING).clamp(-2.5, 2.5);
    if e > 0.0 && eval(&CameraRaw { exposure: e, ..p.clone() })?.clipped > MAX_CLIPPED {
        e = solve(0.0, e, -MAX_CLIPPED, |x| eval(&CameraRaw { exposure: x, ..p.clone() }).map(|s| -s.clipped))?;
    }
    out.exposure = (e * 20.0).round() / 20.0;
    p.exposure = out.exposure;

    // 2. Highlights.
    if eval(&p)?.p98 > P98_LIMIT {
        let v = solve(-80.0, 0.0, P98_LIMIT, |x| eval(&CameraRaw { highlights: x, ..p.clone() }).map(|s| s.p98))?;
        out.highlights = v.round();
        p.highlights = out.highlights;
    }

    // 3. Shadows.
    if eval(&p)?.p5 < P5_FLOOR {
        let v = solve(0.0, 70.0, P5_FLOOR, |x| eval(&CameraRaw { shadows: x, ..p.clone() }).map(|s| s.p5))?;
        out.shadows = v.round();
        p.shadows = out.shadows;
    }

    // 4. Whites and Blacks.
    let s = eval(&p)?;
    if (0.6..0.96).contains(&s.hi) {
        let v = solve(0.0, 80.0, WHITE_TARGET, |x| eval(&CameraRaw { whites: x, ..p.clone() }).map(|s| s.hi))?;
        out.whites = v.round();
        p.whites = out.whites;
    }
    let s = eval(&p)?;
    if s.lo > 0.04 {
        let v = solve(-80.0, 0.0, BLACK_TARGET, |x| eval(&CameraRaw { blacks: x, ..p.clone() }).map(|s| s.lo))?;
        out.blacks = v.round();
        p.blacks = out.blacks;
    } else if s.crushed > 0.01 {
        let v = solve(0.0, 60.0, -0.01, |x| eval(&CameraRaw { blacks: x, ..p.clone() }).map(|s| -s.crushed))?;
        out.blacks = v.round();
        p.blacks = out.blacks;
    }

    // 5. Contrast, from the spread that is left.
    let s = eval(&p)?;
    let spread = s.p95 - s.p5;
    if (spread - SPREAD_TARGET).abs() > 0.08 {
        out.contrast = ((SPREAD_TARGET - spread) / SPREAD_TARGET * 60.0).clamp(-25.0, 35.0).round();
    }
    Some(out)
}

/// Brightness statistics of one rendering (display-encoded, 0..=1).
#[derive(Clone, Copy, Debug)]
struct Stats {
    /// 0.5th percentile of luminance.
    lo: f32,
    p5: f32,
    median: f32,
    p95: f32,
    p98: f32,
    /// 99.5th percentile of the brightest channel.
    hi: f32,
    /// Share of pixels with a channel at white.
    clipped: f32,
    /// Share of pixels that are all but black.
    crushed: f32,
}

fn stats(px: &[[f32; 4]]) -> Option<Stats> {
    let mut lum = Vec::with_capacity(px.len());
    let mut peak = Vec::with_capacity(px.len());
    for q in px {
        if q[3].is_nan() || q[3] <= 0.0 || !q.iter().all(|v| v.is_finite()) {
            continue;
        }
        let c = [q[0].clamp(0.0, 1.0), q[1].clamp(0.0, 1.0), q[2].clamp(0.0, 1.0)];
        lum.push(luma(c));
        peak.push(c[0].max(c[1]).max(c[2]));
    }
    if lum.is_empty() {
        return None;
    }
    let n = lum.len() as f32;
    let clipped = peak.iter().filter(|v| **v >= 0.995).count() as f32 / n;
    let crushed = peak.iter().filter(|v| **v <= 0.004).count() as f32 / n;
    lum.sort_by(f32::total_cmp);
    peak.sort_by(f32::total_cmp);
    let at = |v: &[f32], q: f32| v.get(((v.len() - 1) as f32 * q).round() as usize).copied().unwrap_or(0.0);
    Some(Stats {
        lo: at(&lum, 0.005),
        p5: at(&lum, 0.05),
        median: at(&lum, 0.5),
        p95: at(&lum, 0.95),
        p98: at(&lum, 0.98),
        hi: at(&peak, 0.995),
        clipped,
        crushed,
    })
}

/// Box-averaged copy of the visible pixels, at most [`THUMB`] on the long side.
fn thumbnail(px: &[[f32; 4]], w: usize, h: usize) -> Option<(Vec<[f32; 4]>, usize, usize)> {
    if w == 0 || h == 0 || px.len() < w.checked_mul(h)? {
        return None;
    }
    let f = w.max(h).div_ceil(THUMB).max(1);
    let (tw, th) = (w.div_ceil(f), h.div_ceil(f));
    let mut out = Vec::with_capacity(tw * th);
    for ty in 0..th {
        for tx in 0..tw {
            let mut sum = [0.0f64; 3];
            let mut n = 0.0f64;
            for y in ty * f..((ty + 1) * f).min(h) {
                for x in tx * f..((tx + 1) * f).min(w) {
                    let q = px.get(y * w + x)?;
                    if q[3] > 0.0 && q.iter().all(|v| v.is_finite()) {
                        for c in 0..3 {
                            sum[c] += f64::from(q[c]);
                        }
                        n += 1.0;
                    }
                }
            }
            out.push(if n > 0.0 { [(sum[0] / n) as f32, (sum[1] / n) as f32, (sum[2] / n) as f32, 1.0] } else { [0.0; 4] });
        }
    }
    Some((out, tw, th))
}

/// Finds `x` in `[lo, hi]` where the monotone response `f` equals `target`; a target outside
/// the response's range gives the nearer end. `None` if the response can't be measured.
fn solve(lo: f32, hi: f32, target: f32, mut f: impl FnMut(f32) -> Option<f32>) -> Option<f32> {
    let (flo, fhi) = (f(lo)?, f(hi)?);
    let rising = fhi >= flo;
    if rising && target <= flo || !rising && target >= flo {
        return Some(lo);
    }
    if rising && target >= fhi || !rising && target <= fhi {
        return Some(hi);
    }
    let (mut a, mut b) = (lo, hi);
    for _ in 0..9 {
        let m = 0.5 * (a + b);
        let below = f(m)? < target;
        if below == rising {
            a = m;
        } else {
            b = m;
        }
    }
    Some(0.5 * (a + b))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic scene: a horizontal ramp with a little texture, mapped to `[lo, hi]`.
    fn scene(w: usize, h: usize, lo: f32, hi: f32) -> Vec<[f32; 4]> {
        let mut seed = 12345u32;
        (0..w * h)
            .map(|i| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = (seed >> 16) as f32 / 65535.0 - 0.5;
                let x = (i % w) as f32 / (w - 1) as f32;
                let v = (lo + (hi - lo) * (0.8 * x + 0.2 * (0.5 + noise))).clamp(0.0, 1.0);
                [v, v * 0.95, v * 0.9, 1.0]
            })
            .collect()
    }

    fn auto(px: &[[f32; 4]], w: usize, h: usize) -> AutoTone {
        auto_tone_develop(px, w, h, &CameraRaw::default(), false)
    }

    fn median_luma(px: &[[f32; 4]]) -> f32 {
        stats(px).map(|s| s.median).unwrap_or(0.0)
    }

    #[test]
    fn a_dark_picture_is_brightened() {
        let px = scene(80, 60, 0.02, 0.35);
        let a = auto(&px, 80, 60);
        assert!(a.exposure > 0.3, "{a:?}");
        let mut out = px.clone();
        let mut p = CameraRaw::default();
        a.apply(&mut p);
        develop(&mut out, 80, 60, &p, false);
        assert!(median_luma(&out) > median_luma(&px) + 0.05);
    }

    #[test]
    fn a_bright_picture_is_darkened_and_its_highlights_recovered() {
        let px = scene(80, 60, 0.45, 1.0);
        let a = auto(&px, 80, 60);
        assert!(a.exposure < -0.1 || a.highlights < 0.0, "{a:?}");
    }

    #[test]
    fn a_flat_picture_gains_contrast_and_a_contrasty_one_loses_it() {
        let flat = scene(80, 60, 0.35, 0.65);
        assert!(auto(&flat, 80, 60).contrast > 0.0);
        let hard = scene(80, 60, 0.0, 1.0);
        assert!(auto(&hard, 80, 60).contrast <= 0.0);
    }

    #[test]
    fn a_dull_top_end_gets_whites_and_a_milky_floor_gets_blacks() {
        let px = scene(80, 60, 0.15, 0.8);
        let a = auto(&px, 80, 60);
        assert!(a.whites > 0.0, "{a:?}");
        assert!(a.blacks < 0.0, "{a:?}");
    }

    #[test]
    fn the_results_stay_inside_the_sliders() {
        for (lo, hi) in [(0.0, 0.05), (0.0, 1.0), (0.9, 1.0), (0.1, 0.4), (0.3, 0.9)] {
            let a = auto(&scene(48, 48, lo, hi), 48, 48);
            assert!((-5.0..=5.0).contains(&a.exposure), "{a:?}");
            for v in [a.contrast, a.highlights, a.shadows, a.whites, a.blacks] {
                assert!((-100.0..=100.0).contains(&v), "{a:?}");
            }
        }
    }

    #[test]
    fn nothing_to_read_means_no_change() {
        let mut p = CameraRaw::default();
        AutoTone::default().apply(&mut p);
        assert!(p.is_identity());
        assert!(auto(&[], 0, 0).is_neutral());
        assert!(auto(&[[0.0, 0.0, 0.0, 1.0]; 64], 8, 8).is_neutral(), "black");
        assert!(auto(&[[0.5, 0.5, 0.5, 1.0]; 64], 8, 8).is_neutral(), "flat");
        assert!(auto(&[[0.5, 0.5, 0.5, 0.0]; 64], 8, 8).is_neutral(), "transparent");
        let mut bad = scene(8, 8, 0.1, 0.9);
        bad[3] = [f32::NAN, 0.5, 0.5, 1.0];
        bad[9] = [f32::INFINITY, 0.5, 0.5, 1.0];
        let _ = auto(&bad, 8, 8);
        // Fewer pixels than w × h is rejected, not indexed.
        assert!(auto(&[[0.5, 0.5, 0.5, 1.0]; 10], 8, 8).is_neutral());
        assert!(auto(&[[0.5, 0.5, 0.5, 1.0]; 10], usize::MAX, 2).is_neutral());
    }

    #[test]
    fn a_well_exposed_picture_changes_little() {
        let px = scene(80, 60, 0.04, 0.96);
        let a = auto(&px, 80, 60);
        assert!(a.exposure.abs() <= 0.5, "{a:?}");
        assert!(a.highlights >= -40.0 && a.shadows <= 40.0, "{a:?}");
    }

    #[test]
    fn running_auto_on_its_own_result_settles() {
        let px = scene(80, 60, 0.02, 0.4);
        let first = auto(&px, 80, 60);
        let mut out = px.clone();
        let mut p = CameraRaw::default();
        first.apply(&mut p);
        develop(&mut out, 80, 60, &p, false);
        let second = auto(&out, 80, 60);
        assert!(second.exposure.abs() < first.exposure.abs(), "{first:?} then {second:?}");
    }

    #[test]
    fn auto_keeps_the_other_settings_in_the_loop() {
        let px = scene(80, 60, 0.1, 0.7);
        let base = CameraRaw { vibrance: 30.0, temperature: 10.0, ..Default::default() };
        let a = auto_tone_develop(&px, 80, 60, &base, false);
        let mut p = base.clone();
        a.apply(&mut p);
        assert_eq!((p.vibrance, p.temperature), (30.0, 10.0));
    }

    #[test]
    fn solve_handles_both_directions_and_unreachable_targets() {
        assert_eq!(solve(0.0, 10.0, 99.0, Some), Some(10.0));
        assert_eq!(solve(0.0, 10.0, -5.0, Some), Some(0.0));
        let up = solve(0.0, 10.0, 4.0, Some).unwrap();
        assert!((up - 4.0).abs() < 0.05, "{up}");
        let down = solve(0.0, 10.0, 4.0, |x| Some(10.0 - x)).unwrap();
        assert!((down - 6.0).abs() < 0.05, "{down}");
        assert_eq!(solve(0.0, 1.0, 0.5, |_| None), None);
    }
}
