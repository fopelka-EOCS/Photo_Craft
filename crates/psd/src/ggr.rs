//! GIMP gradient files (`.ggr`).
//!
//! A text file: a `GIMP Gradient` line, an optional `Name: …` line, the segment count, then one
//! line per segment — `left middle right`, the RGBA colour at each end, the blend function
//! (0 linear, 1 curved, 2 sine, 3 sphere increasing, 4 sphere decreasing) and the colour type
//! (0 RGB, 1 HSV counter-clockwise, 2 HSV clockwise), optionally followed by two more numbers
//! (associated colour types) that are ignored. Positions run 0..1.
//!
//! A segment is a curve, not a pair of stops, so the result is a [`GrdGradient`] whose colour and
//! opacity stops sample each segment where its curve is not a straight RGB ramp: a plain linear
//! segment with a centred midpoint stays two stops, every other one gets [`SAMPLES`] sub-steps. A
//! segment that starts with another colour than the previous one ended with keeps both stops at the
//! shared position (a hard edge).

use crate::error::{PsdError, Result};
use crate::grd::{GrdColor, GrdGradient, GrdStop};

/// The first bytes of every `.ggr` file.
pub const SIGNATURE: &[u8] = b"GIMP Gradient";
/// Most segments read from one file.
pub const MAX_SEGMENTS: usize = 1_000;
/// Sub-steps a curved segment is sampled with (fewer in files with very many segments).
pub const SAMPLES: usize = 16;

const EPSILON: f32 = 1e-5;

#[derive(Clone, Copy)]
struct Segment {
    left: f32,
    middle: f32,
    right: f32,
    start: [f32; 4],
    end: [f32; 4],
    blend: u8,
    color_type: u8,
}

fn number(token: Option<&str>) -> Result<f32> {
    let v: f32 = token
        .ok_or_else(|| PsdError::invalid("a gradient segment is missing numbers"))?
        .parse()
        .map_err(|_| PsdError::invalid("a gradient segment holds a non-number"))?;
    if v.is_finite() { Ok(v) } else { Err(PsdError::invalid("a gradient segment holds a non-finite number")) }
}

fn rgba(t: &mut std::str::SplitWhitespace<'_>) -> Result<[f32; 4]> {
    let mut v = [0.0f32; 4];
    for c in &mut v {
        *c = number(t.next())?.clamp(0.0, 1.0);
    }
    Ok(v)
}

fn segment(line: &str) -> Result<Segment> {
    let mut t = line.split_whitespace();
    let left = number(t.next())?.clamp(0.0, 1.0);
    let middle = number(t.next())?;
    let right = number(t.next())?.clamp(0.0, 1.0);
    let start = rgba(&mut t)?;
    let end = rgba(&mut t)?;
    let blend = number(t.next())?;
    let color_type = number(t.next())?;
    // The middle must lie inside the segment; a stray one would bend the curve out of range.
    let middle = middle.clamp(left.min(right), right.max(left));
    Ok(Segment { left, middle, right, start, end, blend: blend as u8, color_type: color_type as u8 })
}

fn linear_factor(middle: f32, pos: f32) -> f32 {
    if pos <= middle {
        if middle < EPSILON { 0.0 } else { 0.5 * pos / middle }
    } else {
        let (pos, middle) = (pos - middle, 1.0 - middle);
        if middle < EPSILON { 1.0 } else { 0.5 + 0.5 * pos / middle }
    }
}

fn rgb_to_hsv([r, g, b]: [f32; 3]) -> [f32; 3] {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    let h = if d < EPSILON {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    [h, if max < EPSILON { 0.0 } else { d / max }, max]
}

fn hsv_to_rgb([h, s, v]: [f32; 3]) -> [f32; 3] {
    let k = |n: f32| {
        let x = (n + h * 6.0).rem_euclid(6.0);
        v - v * s * x.min(4.0 - x).clamp(0.0, 1.0)
    };
    [k(5.0), k(3.0), k(1.0)]
}

impl Segment {
    /// The colour and opacity at `pos` (inside the segment), as GIMP evaluates it.
    fn at(&self, pos: f32) -> [f32; 4] {
        let len = self.right - self.left;
        let (middle, pos) = if len < EPSILON { (0.5, 0.5) } else { ((self.middle - self.left) / len, ((pos - self.left) / len).clamp(0.0, 1.0)) };
        let factor = match self.blend {
            1 => pos.powf(0.5f32.ln() / middle.max(EPSILON).ln()),
            2 => ((-std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * linear_factor(middle, pos)).sin() + 1.0) / 2.0,
            3 => {
                let p = linear_factor(middle, pos) - 1.0;
                (1.0 - p * p).max(0.0).sqrt()
            }
            4 => {
                let p = linear_factor(middle, pos);
                1.0 - (1.0 - p * p).max(0.0).sqrt()
            }
            _ => linear_factor(middle, pos),
        };
        let alpha = self.start[3] + (self.end[3] - self.start[3]) * factor;
        let mut rgb = [0.0; 3];
        if self.color_type == 1 || self.color_type == 2 {
            let (mut a, mut b) = (rgb_to_hsv([self.start[0], self.start[1], self.start[2]]), rgb_to_hsv([self.end[0], self.end[1], self.end[2]]));
            if self.color_type == 1 && a[0] < b[0] {
                a[0] += 1.0;
            } else if self.color_type == 2 && b[0] < a[0] {
                b[0] += 1.0;
            }
            rgb = hsv_to_rgb([(a[0] + (b[0] - a[0]) * factor).rem_euclid(1.0), a[1] + (b[1] - a[1]) * factor, a[2] + (b[2] - a[2]) * factor]);
        } else {
            for (i, c) in rgb.iter_mut().enumerate() {
                *c = self.start[i] + (self.end[i] - self.start[i]) * factor;
            }
        }
        [rgb[0].clamp(0.0, 1.0), rgb[1].clamp(0.0, 1.0), rgb[2].clamp(0.0, 1.0), alpha.clamp(0.0, 1.0)]
    }

    /// Whether the segment is a plain two-stop ramp.
    fn is_straight(&self) -> bool {
        self.blend == 0 && self.color_type == 0 && (self.middle - (self.left + self.right) / 2.0).abs() < 1e-4
    }
}

/// Parses a `.ggr` file into one gradient (named by its `Name:` line, or empty).
pub fn parse(data: &[u8]) -> Result<GrdGradient> {
    let text = String::from_utf8_lossy(data);
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next().ok_or(PsdError::UnexpectedEof { offset: 0, needed: SIGNATURE.len() })?;
    if !first.as_bytes().starts_with(SIGNATURE) {
        return Err(PsdError::invalid("not a GIMP gradient file (no `GIMP Gradient` header)"));
    }
    let mut name = String::new();
    let mut next = lines.next();
    if let Some(n) = next.and_then(|l| l.strip_prefix("Name:")) {
        name = n.trim().to_string();
        next = lines.next();
    }
    let count: usize = next
        .ok_or_else(|| PsdError::invalid("the gradient file has no segment count"))?
        .parse()
        .map_err(|_| PsdError::invalid("the segment count is not a number"))?;
    if count == 0 {
        return Err(PsdError::invalid("the gradient file holds no segments"));
    }
    if count > MAX_SEGMENTS {
        return Err(PsdError::LimitExceeded("too many gradient segments"));
    }
    let mut segments = Vec::with_capacity(count);
    for _ in 0..count {
        let line = lines.next().ok_or_else(|| PsdError::invalid("the gradient file ends before its last segment"))?;
        segments.push(segment(line)?);
    }
    segments.sort_by(|a, b| a.left.total_cmp(&b.left));

    let samples = (MAX_SEGMENTS / count).clamp(1, SAMPLES);
    let mut out = GrdGradient { name, ..Default::default() };
    let same = |a: f32, b: f32| (a - b).abs() < 1e-4;
    for seg in &segments {
        let steps = if seg.is_straight() { 1 } else { samples };
        for k in 0..=steps {
            let loc = seg.left + (seg.right - seg.left) * k as f32 / steps as f32;
            let c = seg.at(loc);
            let rgb = [c[0], c[1], c[2]];
            let dup =
                out.stops.last().is_some_and(|s| same(s.location, loc) && matches!(s.color, GrdColor::Rgb(p) if p.iter().zip(rgb).all(|(a, b)| same(*a, b))));
            if !dup {
                out.stops.push(GrdStop { location: loc, midpoint: 0.5, color: GrdColor::Rgb(rgb) });
            }
            if !out.opacity.last().is_some_and(|o| same(o.0, loc) && same(o.1, c[3])) {
                out.opacity.push((loc, c[3], 0.5));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED_BLUE: &str =
        "GIMP Gradient\nName: Red to Blue\n1\n0.000000 0.500000 1.000000 1.000000 0.000000 0.000000 1.000000 0.000000 0.000000 1.000000 1.000000 0 0 0 0\n";

    fn rgb(c: GrdColor) -> [f32; 3] {
        match c {
            GrdColor::Rgb(v) => v,
            other => panic!("not rgb: {other:?}"),
        }
    }

    #[test]
    fn linear_segment_is_two_stops() {
        let g = parse(RED_BLUE.as_bytes()).unwrap();
        assert_eq!(g.name, "Red to Blue");
        assert_eq!(g.stops.len(), 2);
        assert_eq!((g.stops[0].location, rgb(g.stops[0].color)), (0.0, [1.0, 0.0, 0.0]));
        assert_eq!((g.stops[1].location, rgb(g.stops[1].color)), (1.0, [0.0, 0.0, 1.0]));
        assert!(g.opacity.iter().all(|o| o.1 == 1.0));
    }

    #[test]
    fn alpha_becomes_opacity_stops() {
        let text = "GIMP Gradient\nName: Fade\n1\n0 0.5 1 0 1 0 1 0 1 0 0 0 0\n";
        let g = parse(text.as_bytes()).unwrap();
        assert_eq!(g.opacity, vec![(0.0, 1.0, 0.5), (1.0, 0.0, 0.5)]);
    }

    #[test]
    fn off_centre_middle_is_sampled() {
        let text = "GIMP Gradient\n1\n0 0.25 1 0 0 0 1 1 1 1 1 0 0\n";
        let g = parse(text.as_bytes()).unwrap();
        assert_eq!(g.name, "");
        assert_eq!(g.stops.len(), SAMPLES + 1);
        let at = |loc: f32| g.stops.iter().find(|s| (s.location - loc).abs() < 1e-4).map(|s| rgb(s.color)[0]);
        assert!((at(0.25).unwrap() - 0.5).abs() < 1e-3, "the middle point is half way in colour");
    }

    #[test]
    fn hard_edge_keeps_both_colours() {
        let text = "GIMP Gradient\nName: Two\n2\n0 0.25 0.5 1 0 0 1 1 0 0 1 0 0\n0.5 0.75 1 0 0 1 1 0 0 1 1 0 0\n";
        let g = parse(text.as_bytes()).unwrap();
        let at_half: Vec<[f32; 3]> = g.stops.iter().filter(|s| s.location == 0.5).map(|s| rgb(s.color)).collect();
        assert_eq!(at_half, vec![[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]);
    }

    #[test]
    fn hsv_segments_go_round_the_wheel() {
        // red -> green counter-clockwise passes through blue and magenta; clockwise through yellow.
        let ccw = parse(b"GIMP Gradient\n1\n0 0.5 1 1 0 0 1 0 1 0 1 0 1\n").unwrap();
        let cw = parse(b"GIMP Gradient\n1\n0 0.5 1 1 0 0 1 0 1 0 1 0 2\n").unwrap();
        let mid = |g: &GrdGradient| g.stops.iter().min_by(|a, b| (a.location - 0.5).abs().total_cmp(&(b.location - 0.5).abs())).map(|s| rgb(s.color)).unwrap();
        assert!(mid(&cw)[0] > 0.9 && mid(&cw)[1] > 0.9, "clockwise: yellow");
        assert!(mid(&ccw)[2] > 0.9, "counter-clockwise: blue-ish side of the wheel: {:?}", mid(&ccw));
    }

    #[test]
    fn malformed_files_are_errors_not_panics() {
        for bad in [
            "",
            "GIMP Gradient",
            "GIMP Gradient\nName: x\n",
            "GIMP Gradient\nName: x\nmany\n",
            "GIMP Gradient\nName: x\n0\n",
            "GIMP Gradient\n2\n0 0.5 1 1 0 0 1 0 0 1 1 0 0\n",
            "GIMP Gradient\n1\n0 0.5 1 1 0 0 1 0 0 1\n",
            "GIMP Gradient\n1\n0 NaN 1 1 0 0 1 0 0 1 1 0 0\n",
            "GIMP Gradient\n1\ninf 0.5 1 1 0 0 1 0 0 1 1 0 0\n",
            "GIMP Gradient\n99999999999999999999\n",
            "GIMP Gradient\n1000000\n",
            "not a gradient\n",
        ] {
            assert!(parse(bad.as_bytes()).is_err(), "{bad:?}");
        }
        assert!(parse(&[0xff, 0xfe, 0x00]).is_err());
    }

    #[test]
    fn extreme_values_stay_in_range() {
        let text = "GIMP Gradient\n1\n-5 9 7 -3 4 2 8 -1 5 5 9 9 9\n";
        let g = parse(text.as_bytes()).unwrap();
        assert!(g.stops.iter().all(|s| (0.0..=1.0).contains(&s.location) && rgb(s.color).iter().all(|v| (0.0..=1.0).contains(v))));
        assert!(g.opacity.iter().all(|o| (0.0..=1.0).contains(&o.1)));
    }

    #[test]
    fn many_segments_stay_bounded() {
        let mut text = format!("GIMP Gradient\n{MAX_SEGMENTS}\n");
        for i in 0..MAX_SEGMENTS {
            let (l, r) = (i as f32 / MAX_SEGMENTS as f32, (i + 1) as f32 / MAX_SEGMENTS as f32);
            text.push_str(&format!("{l} {} {r} 0 0 0 1 1 1 1 1 2 1 0\n", (l + r) / 2.0));
        }
        let g = parse(text.as_bytes()).unwrap();
        assert!(g.stops.len() <= 2 * MAX_SEGMENTS + 1);
    }
}
