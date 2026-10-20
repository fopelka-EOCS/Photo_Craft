//! Save as `.tx`: the texture's levels, depths, alpha and detections, read back through the TIFF
//! decoder and through File › Open.

use photocraft_codecs::{ChannelLayout, DecodeOptions, SampleType as CSample, decode_tiff_page, tiff_info};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_io::*;

const W: u32 = 100;
const H: u32 = 60;

/// A `W`×`H` document of one raster layer from straight RGBA samples in `0..=1` (`px(x, y)`).
fn doc(mode: ColorMode, depth: SampleType, px: impl Fn(u32, u32) -> [f32; 4]) -> photocraft_doc::Document {
    let mut d = photocraft_doc::Document::new("t", photocraft_geom::Size::new(W, H), mode, depth);
    let fmt = d.pixel_format();
    let mut s = photocraft_raster::Surface::new(fmt);
    let n = fmt.channels();
    let mut data = Vec::new();
    for y in 0..H {
        for x in 0..W {
            let p = px(x, y);
            match n {
                2 => data.extend_from_slice(&[p[0], p[3]]),
                4 => data.extend_from_slice(&p),
                _ => data.extend_from_slice(&[p[0], p[1], p[2], p[3], 1.0][..n]),
            }
        }
    }
    s.write_region(d.bounds(), &data);
    d.layers.push(photocraft_doc::Layer::new("Background", photocraft_doc::LayerContent::Raster(s)));
    d
}

fn varied(x: u32, y: u32) -> [f32; 4] {
    [(x * 7 % 256) as f32 / 255.0, (y * 13 % 256) as f32 / 255.0, ((x * y) % 256) as f32 / 255.0, 1.0]
}

fn tx(d: &photocraft_doc::Document, o: Option<TxOptions>) -> Vec<u8> {
    export(d, "t.tx", &ExportOptions { tx: o, ..Default::default() }).expect("export .tx").bytes
}

/// `bytes` with `ExtraSamples = 1` (associated alpha) turned into 2 in every directory, so the
/// decoder returns the stored premultiplied samples (it un-premultiplies associated alpha).
fn stored(bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let rd16 = |b: &[u8], at: usize| u16::from_le_bytes([b[at], b[at + 1]]) as usize;
    let rd32 = |b: &[u8], at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize;
    assert_eq!(&bytes[..4], b"II*\0");
    let mut ifd = rd32(bytes, 4);
    while ifd != 0 {
        let n = rd16(bytes, ifd);
        for i in 0..n {
            let e = ifd + 2 + 12 * i;
            if rd16(bytes, e) == 338 {
                out[e + 8] = 2;
            }
        }
        ifd = rd32(bytes, ifd + 2 + 12 * n);
    }
    out
}

fn page(bytes: &[u8], i: usize) -> photocraft_codecs::Image {
    decode_tiff_page(bytes, i, &DecodeOptions::default()).expect("decode level")
}

#[test]
fn every_level_down_to_one_pixel() {
    let bytes = tx(&doc(ColorMode::Rgb, SampleType::U8, varied), None);
    let info = tiff_info(&bytes).unwrap();
    let sizes: Vec<(u32, u32)> = info.pages.iter().map(|p| (p.width, p.height)).collect();
    assert_eq!(sizes, vec![(100, 60), (50, 30), (25, 15), (12, 7), (6, 3), (3, 1), (1, 1)]);
}

#[test]
fn top_level_keeps_the_samples_at_every_depth() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let d = doc(ColorMode::Rgb, depth, varied);
        let want = photocraft_io::document_to_image(&d, &mut Vec::new()).unwrap();
        let got = page(&tx(&d, None), 0);
        assert_eq!(got.layout(), ChannelLayout::Rgb, "{depth:?}");
        assert_eq!(got.sample_type(), want.sample_type(), "{depth:?}");
        assert!(got.data() == want.data(), "{depth:?}: the top level is the document's own samples");
    }
}

#[test]
fn float_keeps_values_above_one() {
    let d = doc(ColorMode::Rgb, SampleType::F32, |x, _| if x < 50 { [4.0, 0.5, 16.0, 1.0] } else { [0.25, 0.25, 0.25, 1.0] });
    let bytes = tx(&d, None);
    for i in 0..tiff_info(&bytes).unwrap().pages.len() {
        let img = page(&bytes, i);
        let max = img.data().as_chunks::<4>().0.iter().map(|c| f32::from_ne_bytes(*c)).fold(0.0f32, f32::max);
        assert!(max > 3.0, "level {i}: HDR values survive the reduction (max {max})");
    }
}

#[test]
fn alpha_is_premultiplied_on_every_level() {
    let d = doc(ColorMode::Rgb, SampleType::U8, |x, _| [200.0 / 255.0, 1.0, 0.0, if x < 50 { 100.0 / 255.0 } else { 0.0 }]);
    let bytes = stored(&tx(&d, None));
    let top = page(&bytes, 0);
    assert_eq!(top.layout(), ChannelLayout::Rgba);
    assert_eq!(&top.data()[..4], &[78, 100, 0, 100], "200 at alpha 100/255 → 78 (rounded)");
    assert_eq!(&top.data()[60 * 4..60 * 4 + 4], &[0, 0, 0, 0], "transparent pixels are black");
    for i in 1..tiff_info(&bytes).unwrap().pages.len() {
        let img = page(&bytes, i);
        for p in img.data().as_chunks::<4>().0 {
            assert!(p[0] <= p[3] && p[1] <= p[3], "level {i}: colour never exceeds alpha {p:?}");
        }
    }
}

#[test]
fn equal_channels_become_gray_unless_asked_not_to() {
    let gray = |x: u32, y: u32| {
        let v = ((x + y) % 256) as f32 / 255.0;
        [v, v, v, 1.0]
    };
    let d = doc(ColorMode::Rgb, SampleType::U8, gray);
    assert_eq!(page(&tx(&d, None), 0).layout(), ChannelLayout::Gray);
    let off = TxOptions { monochrome_detect: false, ..Default::default() };
    assert_eq!(page(&tx(&d, Some(off)), 0).layout(), ChannelLayout::Rgb);
    // Never with alpha (both maketx versions keep RGBA).
    let ga = doc(ColorMode::Rgb, SampleType::U8, |x, y| {
        let [v, _, _, _] = gray(x, y);
        [v, v, v, 0.5]
    });
    assert_eq!(page(&tx(&ga, None), 0).layout(), ChannelLayout::Rgba);
}

#[test]
fn one_colour_becomes_one_tile() {
    let d = doc(ColorMode::Rgb, SampleType::U8, |_, _| [0.2, 0.4, 0.6, 1.0]);
    let bytes = tx(&d, None);
    let info = tiff_info(&bytes).unwrap();
    assert_eq!((info.pages[0].width, info.pages[0].height), (64, 60));
    assert_eq!(info.pages.len(), 7);
    assert_eq!(&page(&bytes, 0).data()[..3], &[51, 102, 153]);
    let classic = tiff_info(&tx(&d, Some(TxOptions::preset(TxPreset::Classic)))).unwrap();
    assert_eq!((classic.pages[0].width, classic.pages[0].height), (100, 60), "the classic call has no constant-colour detection");
}

#[test]
fn options_reach_the_file() {
    let d = doc(ColorMode::Rgb, SampleType::U8, varied);
    let one = TxOptions { mipmap: false, ..Default::default() };
    assert_eq!(tiff_info(&tx(&d, Some(one))).unwrap().pages.len(), 1);
    let wide = TxOptions { data_type: TxDataType::U16, tile: 32, compress: false, ..Default::default() };
    let img = page(&tx(&d, Some(wide)), 0);
    assert_eq!(img.sample_type(), CSample::U16);
    let src = photocraft_io::document_to_image(&d, &mut Vec::new()).unwrap();
    let as16: Vec<u8> = src.data().iter().flat_map(|&v| (u16::from(v) * 257).to_ne_bytes()).collect();
    assert!(img.data() == as16.as_slice(), "8 → 16 bit scales by 257");
    let half = TxOptions { data_type: TxDataType::Half, ..Default::default() };
    assert_eq!(page(&tx(&d, Some(half)), 0).sample_type(), CSample::F16);
    let bad = TxOptions { tile: 48, ..Default::default() };
    assert!(export(&d, "t.tx", &ExportOptions { tx: Some(bad), ..Default::default() }).is_err());
}

/// The fingerprint covers what shapes the lower levels, not only the top level's samples: an
/// OpenImageIO texture cache must not take a Box texture for its Lanczos twin.
#[test]
fn fingerprint_differs_when_the_lower_levels_do() {
    let d = doc(ColorMode::Rgb, SampleType::U8, varied);
    let desc = |o: TxOptions| {
        let bytes = tx(&d, Some(o));
        let at = bytes.windows(6).position(|w| w == b"SHA-1=").unwrap();
        bytes[at..at + 46].to_vec()
    };
    let lanczos = desc(TxOptions::default());
    assert_ne!(lanczos, desc(TxOptions { filter: TxFilter::Box, ..Default::default() }));
    assert_ne!(lanczos, desc(TxOptions { mipmap: false, ..Default::default() }));
    assert_eq!(lanczos, desc(TxOptions::default()), "the same settings, the same fingerprint");
}

/// Next to a fully transparent stripe the reduction rings as maketx's does: the texels beside
/// the hard alpha edge are computed, not left at the empty stripe's zero.
#[test]
fn ringing_reaches_into_an_empty_stripe() {
    let d = doc(ColorMode::Rgb, SampleType::F32, |x, _| if x < 32 { [0.0, 0.0, 0.0, 0.0] } else { [1.0, 1.0, 1.0, 1.0] });
    let bytes = stored(&tx(&d, Some(TxOptions { filter: TxFilter::Lanczos3, ..Default::default() })));
    let level1 = page(&bytes, 1);
    let alpha_at = |x: usize| {
        let i = (x * 4 + 3) * 4;
        f32::from_ne_bytes([level1.data()[i], level1.data()[i + 1], level1.data()[i + 2], level1.data()[i + 3]])
    };
    // Lanczos undershoots just before the edge (texel 16 of the half-size level).
    assert!(alpha_at(14) < 0.0 || alpha_at(15) < 0.0, "ringing before the edge: {} {}", alpha_at(14), alpha_at(15));
}

/// The default filter follows the bit depth: a float texture is reduced with the triangle (no
/// ringing, nothing below zero next to the edge), an 8-bit one with Lanczos 3, and the file
/// records the filter it took, never `auto`.
#[test]
fn default_filter_is_triangle_for_float() {
    let software = |b: &[u8]| {
        let at = b.windows(18).position(|w| w == b"maketx-equivalent ").expect("Software tag");
        String::from_utf8_lossy(&b[at..b[at..].iter().position(|c| *c == 0).map_or(b.len(), |e| at + e)]).into_owned()
    };
    let d = doc(ColorMode::Rgb, SampleType::F32, |x, _| if x < 32 { [0.0, 0.0, 0.0, 0.0] } else { [1.0, 1.0, 1.0, 1.0] });
    let bytes = tx(&d, None);
    assert!(software(&bytes).contains("--filter triangle"), "{}", software(&bytes));
    let level1 = page(&stored(&bytes), 1);
    let min = level1.data().as_chunks::<4>().0.iter().map(|c| f32::from_ne_bytes(*c)).fold(f32::INFINITY, f32::min);
    assert!(min >= 0.0, "no undershoot with the triangle: {min}");
    let d8 = doc(ColorMode::Rgb, SampleType::U8, varied);
    let b8 = tx(&d8, None);
    assert!(software(&b8).contains("--filter lanczos3"), "{}", software(&b8));
}

#[test]
fn cmyk_is_written_as_rgb() {
    let d = doc(ColorMode::Cmyk, SampleType::U8, |x, _| [x as f32 / 100.0, 0.2, 0.0, 0.1]);
    let img = page(&tx(&d, None), 0);
    assert_eq!(img.layout(), ChannelLayout::Rgb);
}

#[test]
fn opens_again_read_only() {
    let d = doc(ColorMode::Rgb, SampleType::U16, varied);
    let r = import("t.tx", &tx(&d, None)).expect("open .tx");
    assert!(r.source_read_only, "a texture opens without a path to save back to");
    assert_eq!(r.document.size, d.size);
    assert!(!r.warnings.iter().any(|w| w.contains("pages")), "a complete MIP chain is no 'more pages' warning: {:?}", r.warnings);
    let _ = PixelFormat::RGBA8;
}
