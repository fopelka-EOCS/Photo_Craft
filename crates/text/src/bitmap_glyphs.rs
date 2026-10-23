//! Colour bitmap glyphs: fonts whose emoji are pictures (Apple's `sbix`, Google's `CBDT`/`CBLC`)
//! rather than outlines. The outline rasteriser skips them (they have nothing to fill), so they
//! are placed here, decoded, and composited as images in their own colours (#6883).

use std::collections::HashMap;

use photocraft_color::PixelFormat;
use skrifa::bitmap::{BitmapData, Origin};
use skrifa::instance::{LocationRef, NormalizedCoord, Size};
use skrifa::{GlyphId, MetadataProvider};

use crate::layout::TextLayout;
use crate::raster::Xform;
use crate::render::{glyph_center, glyph_xform};
use crate::warp::{Placement, Warp};

/// The most pixels along a side, and in all, of a bitmap glyph we decode. Colour emoji strikes
/// are 128–512 px; a hostile font could declare far more.
const MAX_SIDE: u32 = 2048;
const MAX_PIXELS: u64 = 2048 * 2048;

/// A bitmap glyph placed in a layout.
pub(crate) struct Placed<'a> {
    /// Bitmap pixel (column, row from the top) → text space.
    pub to_text: Xform,
    /// The bitmap's declared size, which `to_text` spans.
    pub width: u32,
    pub height: u32,
    pub data: BitmapData<'a>,
    /// Opacity of the glyph's style colour (the bitmap keeps its own colours).
    pub alpha: f32,
    /// Face and glyph, to decode each picture once.
    pub key: (u32, u32),
}

/// The glyphs of `layout` drawn from a colour bitmap strike, placed through `warp` as outlines
/// are. A bitmap is kept rigid: under a bending warp its three corners are mapped and the image
/// follows them as an affine map.
pub(crate) fn placed<'a>(layout: &'a TextLayout, warp: Option<&Warp>) -> Vec<Placed<'a>> {
    let mut out = Vec::new();
    // Most faces carry no bitmaps: look once per face, so ordinary text costs nothing here.
    let has_strikes: Vec<bool> = layout
        .faces
        .iter()
        .map(|f| skrifa::FontRef::from_index(f.font.data.as_ref(), f.font.index).is_ok_and(|font| !font.bitmap_strikes().is_empty()))
        .collect();
    if !has_strikes.iter().any(|&b| b) {
        return out;
    }
    for g in &layout.glyphs {
        if !has_strikes.get(g.face as usize).copied().unwrap_or(false) {
            continue;
        }
        let Some(face) = layout.faces.get(g.face as usize) else { continue };
        let Some(st) = layout.styles.get(g.style as usize) else { continue };
        let Ok(font) = skrifa::FontRef::from_index(face.font.data.as_ref(), face.font.index) else { continue };
        let strikes = font.bitmap_strikes();
        if strikes.is_empty() || !(face.size_px.is_finite() && face.size_px > 0.0) {
            continue;
        }
        let Some(bg) = strikes.glyph_for_size(Size::new(face.size_px), GlyphId::new(g.id)) else { continue };
        if !matches!(bg.data, BitmapData::Png(_) | BitmapData::Bgra(_)) {
            continue;
        }
        let (w, h) = (bg.width, bg.height);
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || u64::from(w) * u64::from(h) > MAX_PIXELS {
            continue;
        }
        if !(bg.ppem_x.is_finite() && bg.ppem_x > 0.0 && bg.ppem_y.is_finite() && bg.ppem_y > 0.0) {
            continue;
        }
        let upem = f32::from(font.metrics(Size::unscaled(), LocationRef::default()).units_per_em).max(1.0);
        // The bitmap's left edge and bottom edge from the pen, in strike pixels (y up).
        let (left, bottom) = match bg.placement_origin {
            Origin::BottomLeft => (bg.inner_bearing_x + bg.bearing_x * bg.ppem_x / upem, bg.inner_bearing_y + bg.bearing_y * bg.ppem_y / upem),
            Origin::TopLeft => (bg.inner_bearing_x, bg.inner_bearing_y - h as f32),
        };
        let (kx, ky) = (f64::from(face.size_px / bg.ppem_x), f64::from(face.size_px / bg.ppem_y));
        let top = f64::from(bottom) + f64::from(h);
        // Bitmap pixel → glyph space (px, y up) → text space.
        let pixel_to_glyph = Xform([kx, 0.0, 0.0, -ky, f64::from(left) * kx, top * ky]);
        let mut glyph = glyph_xform(layout, g, face.skew_deg);
        let mut bend = warp;
        if let Some(wp) = warp {
            let coords: Vec<NormalizedCoord> = face.coords.iter().map(|&c| NormalizedCoord::from_bits(c)).collect();
            match wp.place_glyph(glyph_center(layout, g, &font, face.size_px, &coords)) {
                Placement::Bend => {}
                Placement::Hidden => continue,
                Placement::Frame(m) => {
                    glyph = Xform(m).mul(&glyph);
                    bend = None;
                }
            }
        }
        let mut to_text = glyph.mul(&pixel_to_glyph);
        if let Some(wp) = bend {
            let (wf, hf) = (f64::from(w), f64::from(h));
            let bent = |p: (f64, f64)| wp.apply(p.0, p.1);
            let p0 = bent(to_text.apply(0.0, 0.0));
            let p1 = bent(to_text.apply(wf, 0.0));
            let p2 = bent(to_text.apply(0.0, hf));
            to_text = Xform([(p1.0 - p0.0) / wf, (p1.1 - p0.1) / wf, (p2.0 - p0.0) / hf, (p2.1 - p0.1) / hf, p0.0, p0.1]);
        }
        if !to_text.0.iter().all(|v| v.is_finite()) {
            continue;
        }
        out.push(Placed { to_text, width: w, height: h, data: bg.data, alpha: st.color.alpha.clamp(0.0, 1.0), key: (g.face, g.id) });
    }
    out
}

/// A decoded picture: premultiplied RGBA, 0–1.
struct Image {
    w: usize,
    h: usize,
    px: Vec<[f32; 4]>,
}

impl Image {
    fn texel(&self, x: i64, y: i64) -> [f32; 4] {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
            return [0.0; 4];
        }
        self.px.get(y as usize * self.w + x as usize).copied().unwrap_or([0.0; 4])
    }

    /// Bilinear sample at the image coordinate (`u`, `v`), texels centred on half pixels; beyond
    /// the picture is transparent.
    fn sample(&self, u: f64, v: f64) -> [f32; 4] {
        let (u, v) = (u - 0.5, v - 0.5);
        let (x0, y0) = (u.floor(), v.floor());
        let (fx, fy) = ((u - x0) as f32, (v - y0) as f32);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let (a, b, c, d) = (self.texel(x0, y0), self.texel(x0 + 1, y0), self.texel(x0, y0 + 1), self.texel(x0 + 1, y0 + 1));
        let mut o = [0.0f32; 4];
        for (i, v) in o.iter_mut().enumerate() {
            let at = |t: [f32; 4]| t.get(i).copied().unwrap_or(0.0);
            *v = (at(a) * (1.0 - fx) + at(b) * fx) * (1.0 - fy) + (at(c) * (1.0 - fx) + at(d) * fx) * fy;
        }
        o
    }

    /// Half the size, each pixel the average of its 2 × 2 block (so a big picture drawn small is
    /// filtered rather than sampled sparsely).
    fn halved(&self) -> Image {
        let (w, h) = (self.w.div_ceil(2).max(1), self.h.div_ceil(2).max(1));
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let mut s = [0.0f32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let t = self.texel((x * 2 + dx) as i64, (y * 2 + dy) as i64);
                    for (acc, v) in s.iter_mut().zip(t) {
                        *acc += v * 0.25;
                    }
                }
                px.push(s);
            }
        }
        Image { w, h, px }
    }
}

fn premultiplied(r: f32, g: f32, b: f32, a: f32) -> [f32; 4] {
    [r * a, g * a, b * a, a]
}

/// Decodes a glyph picture (PNG, or uncompressed premultiplied BGRA of `w` × `h`). `None` when it
/// isn't a valid picture of an allowed size.
fn decode(data: &BitmapData, w: u32, h: u32) -> Option<Image> {
    match data {
        BitmapData::Bgra(bytes) => {
            let n = (w as usize).checked_mul(h as usize)?;
            let px: Vec<[f32; 4]> = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .take(n)
                .map(|&[b, g, r, a]| [f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, f32::from(a) / 255.0])
                .collect();
            (px.len() == n).then_some(Image { w: w as usize, h: h as usize, px })
        }
        BitmapData::Png(bytes) => {
            let mut decoder = png::Decoder::new_with_limits(std::io::Cursor::new(*bytes), png::Limits { bytes: 32 * 1024 * 1024 });
            decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
            let mut reader = decoder.read_info().ok()?;
            let (pw, ph) = reader.info().size();
            if pw == 0 || ph == 0 || pw > MAX_SIDE || ph > MAX_SIDE || u64::from(pw) * u64::from(ph) > MAX_PIXELS {
                return None;
            }
            let mut buf = vec![0u8; reader.output_buffer_size()?];
            let frame = reader.next_frame(&mut buf).ok()?;
            let channels = match frame.color_type {
                png::ColorType::Grayscale => 1,
                png::ColorType::GrayscaleAlpha => 2,
                png::ColorType::Rgb => 3,
                png::ColorType::Rgba => 4,
                png::ColorType::Indexed => return None,
            };
            let px: Vec<[f32; 4]> = buf
                .get(..frame.buffer_size())?
                .chunks_exact(channels)
                .map(|c| {
                    let f = |v: u8| f32::from(v) / 255.0;
                    match *c {
                        [g] => premultiplied(f(g), f(g), f(g), 1.0),
                        [g, a] => premultiplied(f(g), f(g), f(g), f(a)),
                        [r, g, b] => premultiplied(f(r), f(g), f(b), 1.0),
                        [r, g, b, a] => premultiplied(f(r), f(g), f(b), f(a)),
                        _ => [0.0; 4],
                    }
                })
                .collect();
            Some(Image { w: frame.width as usize, h: frame.height as usize, px })
        }
        BitmapData::Mask(_) => None,
    }
}

/// Composites `placed` bitmap glyphs over the premultiplied accumulator `acc` (`w` × `h` pixels of
/// `format`'s colour channels plus alpha). `xf` maps text space to accumulator pixels.
pub(crate) fn composite(acc: &mut [f32], w: usize, h: usize, format: &PixelFormat, xf: &Xform, placed: &[Placed]) {
    if placed.is_empty() {
        return;
    }
    let n = format.mode.color_channels();
    let stride = n + 1;
    let plain = PixelFormat { alpha: false, ..*format };
    let mut comps = vec![0.0f32; n.max(4) + 1];
    let mut cache: HashMap<(u32, u32), Option<Image>> = HashMap::new();
    for p in placed {
        let Some(image) = cache.entry(p.key).or_insert_with(|| decode(&p.data, p.width, p.height)).as_ref() else { continue };
        // The picture fills the box the font declares for it, whatever its pixel size.
        let mut to_px = xf.mul(&p.to_text).mul(&Xform([f64::from(p.width) / image.w as f64, 0.0, 0.0, f64::from(p.height) / image.h as f64, 0.0, 0.0]));
        let det = |m: &Xform| m.0[0] * m.0[3] - m.0[1] * m.0[2];
        // Filter before shrinking by more than half.
        let mut owned: Option<Image> = None;
        let mut guard = 0;
        while det(&to_px).abs().sqrt() < 0.5 && guard < 8 {
            let src = owned.as_ref().unwrap_or(image);
            if src.w < 2 && src.h < 2 {
                break;
            }
            let next = src.halved();
            let ratio = Xform([next.w as f64 / src.w as f64, 0.0, 0.0, next.h as f64 / src.h as f64, 0.0, 0.0]);
            to_px = to_px.mul(&Xform([1.0 / ratio.0[0], 0.0, 0.0, 1.0 / ratio.0[3], 0.0, 0.0]));
            owned = Some(next);
            guard += 1;
        }
        let img = owned.as_ref().unwrap_or(image);
        let d = det(&to_px);
        if !d.is_finite() || d.abs() < 1e-12 {
            continue;
        }
        let [a, b, c, dd, e, f] = to_px.0;
        let corners = [to_px.apply(0.0, 0.0), to_px.apply(img.w as f64, 0.0), to_px.apply(img.w as f64, img.h as f64), to_px.apply(0.0, img.h as f64)];
        let min_x = corners.iter().map(|p| p.0).fold(f64::INFINITY, f64::min).floor().max(0.0);
        let max_x = corners.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max).ceil().min(w as f64);
        let min_y = corners.iter().map(|p| p.1).fold(f64::INFINITY, f64::min).floor().max(0.0);
        let max_y = corners.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max).ceil().min(h as f64);
        if !(min_x < max_x && min_y < max_y) {
            continue;
        }
        for y in (min_y as usize)..(max_y as usize) {
            for x in (min_x as usize)..(max_x as usize) {
                let (px, py) = (x as f64 + 0.5 - e, y as f64 + 0.5 - f);
                let (u, v) = ((dd * px - c * py) / d, (-b * px + a * py) / d);
                let s = img.sample(u, v);
                let alpha = s.get(3).copied().unwrap_or(0.0) * p.alpha;
                if alpha <= 0.0 {
                    continue;
                }
                let inv = 1.0 / s.get(3).copied().unwrap_or(1.0);
                let rgb = [s.first().copied().unwrap_or(0.0) * inv, s.get(1).copied().unwrap_or(0.0) * inv, s.get(2).copied().unwrap_or(0.0) * inv, 1.0];
                photocraft_raster::from_rgba_into(&plain, rgb, &mut comps);
                let Some(px) = acc.get_mut((y * w + x) * stride..(y * w + x + 1) * stride) else { continue };
                let keep = 1.0 - alpha;
                for (dst, col) in px.iter_mut().zip(&comps).take(n) {
                    *dst = col * alpha + *dst * keep;
                }
                if let Some(da) = px.get_mut(n) {
                    *da = alpha + *da * keep;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, SampleType};

    fn png_of(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(rgba).unwrap();
        out
    }

    fn placed_over<'a>(data: BitmapData<'a>, w: u32, h: u32, scale: f64) -> Placed<'a> {
        Placed { to_text: Xform([scale, 0.0, 0.0, scale, 0.0, 0.0]), width: w, height: h, data, alpha: 1.0, key: (0, 1) }
    }

    fn rgb() -> PixelFormat {
        PixelFormat::new(ColorMode::Rgb, SampleType::F32, true)
    }

    /// #6883: a PNG glyph picture decodes to its own colours, premultiplied.
    #[test]
    fn decodes_png_and_bgra_pictures() {
        let data = png_of(2, 1, &[255, 0, 0, 255, 0, 255, 0, 128]);
        let img = decode(&BitmapData::Png(&data), 2, 1).unwrap();
        assert_eq!((img.w, img.h), (2, 1));
        assert_eq!(img.px[0], [1.0, 0.0, 0.0, 1.0]);
        assert!((img.px[1][1] - 128.0 / 255.0).abs() < 1e-6 && (img.px[1][3] - 128.0 / 255.0).abs() < 1e-6, "{:?}", img.px[1]);
        // Premultiplied BGRA is read as it is, in RGBA order.
        let bgra = decode(&BitmapData::Bgra(&[0, 0, 255, 255]), 1, 1).unwrap();
        assert_eq!(bgra.px[0], [1.0, 0.0, 0.0, 1.0]);
    }

    /// A glyph picture that isn't one — truncated, not a PNG, far too big, the wrong length — is
    /// refused, not a panic.
    #[test]
    fn hostile_pictures_are_refused() {
        let good = png_of(4, 4, &[200; 64]);
        assert!(decode(&BitmapData::Png(&good), 4, 4).is_some());
        for bad in [&good[..good.len() / 2], b"not a png".as_slice(), &[]] {
            assert!(decode(&BitmapData::Png(bad), 4, 4).is_none());
        }
        assert!(decode(&BitmapData::Bgra(&[0; 8]), 4, 4).is_none(), "too short");
        // A PNG whose header declares a huge picture.
        let mut huge = good.clone();
        huge.get_mut(16..20).unwrap().copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(decode(&BitmapData::Png(&huge), 4, 4).is_none());
    }

    /// The picture lands where the matrix puts it, in its own colours, and outside it nothing is
    /// touched; the style colour's opacity scales it.
    #[test]
    fn composites_the_picture_in_its_own_colours() {
        let red = [0u8, 0, 255, 255].repeat(4);
        let (w, h, stride) = (8usize, 8usize, 4usize);
        let mut acc = vec![0.0f32; w * h * stride];
        let mut p = placed_over(BitmapData::Bgra(&red), 2, 2, 2.0);
        composite(&mut acc, w, h, &rgb(), &Xform::IDENTITY, std::slice::from_ref(&p));
        let at = |acc: &[f32], x: usize, y: usize| acc[(y * w + x) * stride..(y * w + x + 1) * stride].to_vec();
        assert_eq!(at(&acc, 1, 1), vec![1.0, 0.0, 0.0, 1.0]);
        assert_eq!(at(&acc, 6, 6), vec![0.0; 4], "outside the picture");
        let mut half = vec![0.0f32; w * h * stride];
        p.alpha = 0.5;
        composite(&mut half, w, h, &rgb(), &Xform::IDENTITY, &[p]);
        assert_eq!(at(&half, 1, 1), vec![0.5, 0.0, 0.0, 0.5]);
    }

    /// A big picture drawn small is filtered, and every colour mode takes it without a panic.
    #[test]
    fn shrinks_big_pictures_and_serves_every_colour_mode() {
        let blue = [255u8, 0, 0, 255].repeat(64 * 64);
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            let format = PixelFormat::new(mode, SampleType::F32, true);
            let stride = mode.color_channels() + 1;
            let mut acc = vec![0.0f32; 8 * 8 * stride];
            composite(&mut acc, 8, 8, &format, &Xform::IDENTITY, &[placed_over(BitmapData::Bgra(&blue), 64, 64, 0.125)]);
            let centre = &acc[(4 * 8 + 4) * stride..(4 * 8 + 5) * stride];
            assert!((centre[stride - 1] - 1.0).abs() < 1e-3, "{mode:?}: {centre:?}");
        }
        // Degenerate and non-finite placements draw nothing.
        let mut acc = vec![0.0f32; 8 * 8 * 4];
        let flat = Placed { to_text: Xform([0.0; 6]), ..placed_over(BitmapData::Bgra(&blue), 64, 64, 1.0) };
        let wild = Placed { to_text: Xform([1e300, 0.0, 0.0, 1e300, 1e300, 0.0]), ..placed_over(BitmapData::Bgra(&blue), 64, 64, 1.0) };
        composite(&mut acc, 8, 8, &rgb(), &Xform::IDENTITY, &[flat, wild]);
        assert!(acc.iter().all(|v| *v == 0.0));
    }

    /// #6883 end to end: with a colour emoji font installed (Apple Color Emoji, Noto Color Emoji),
    /// an emoji in a text layer draws its picture — in colour, not a blank gap — and the layer's
    /// ink covers it. Skipped on a machine without one (fonts are not bundled).
    #[test]
    fn emoji_render_in_colour() {
        use photocraft_doc::TextLayer;
        use photocraft_doc::text::{CharStyle, TextRun};
        let mut engine = crate::TextEngine::with_system_fonts();
        let Some(family) = ["Apple Color Emoji", "Noto Color Emoji"].into_iter().find(|f| engine.fonts.has_family(f)) else { return };
        let text = "\u{1F4A1}".to_string();
        let style = CharStyle { font_family: family.into(), size_pt: 48.0, ..Default::default() };
        let layer = TextLayer { runs: vec![TextRun { len: text.len(), style }], text, ..Default::default() };
        let (layout, r) = engine.render(&layer, 72.0, rgb());
        assert!(placed(&layout, None).len() == 1, "the emoji is a bitmap glyph of {family}");
        assert!(r.rect.width() > 10 && r.rect.height() > 10, "{:?}", r.rect);
        let mut px = vec![[0u8; 4]; (r.rect.width() * r.rect.height()) as usize];
        r.surface.read_rgba8_into(r.rect, &mut px);
        let opaque: Vec<&[u8; 4]> = px.iter().filter(|p| p[3] > 200).collect();
        assert!(opaque.len() > 200, "the picture has ink: {}", opaque.len());
        let distinct: std::collections::HashSet<[u8; 3]> = opaque.iter().map(|p| [p[0] / 32, p[1] / 32, p[2] / 32]).collect();
        assert!(distinct.len() > 4, "it is in colour, not one fill: {}", distinct.len());
    }
}
