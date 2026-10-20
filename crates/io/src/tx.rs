//! Save as `.tx` (optimized for rendering): a tiled, MIP-mapped TIFF as OpenImageIO's `maketx`
//! writes it, which renderers page in tile by tile at the MIP level they need (V-Ray's
//! VRayBitmap, Arnold, RenderMan).
//!
//! The reference is the call `maketx --oiio --tile 64 64 --filter lanczos3 --monochrome-detect`
//! (OpenImageIO 1.4.12, compared against Arnold's 2.6.3 on the same sources):
//!
//! * the values and the bit depth stay as they are, and no colour conversion happens (V-Ray
//!   chooses the colour space on the VRayBitmap); CMYK and Lab documents are written as RGB;
//! * alpha is premultiplied (rounded, as 2.6.3 does; 1.4.12 truncates);
//! * each MIP level is the previous one resampled with the filter (by default Lanczos-3 for 8- and
//!   16-bit textures and maketx's triangle for half and float ones), half its size rounded down,
//!   the edges repeating outwards;
//! * an RGB image whose channels are all exactly equal is written as gray
//!   (`--monochrome-detect`; never with alpha, as both versions measured);
//! * an image of one colour becomes a tile-sized texture (`--constant-color-detect`);
//! * an alpha that is opaque everywhere is dropped, as for every flat export here (what 2.6.3's
//!   `--opaque-detect` does; 1.4.12 keeps it).

use photocraft_algo::resample::{Resample, ResizeStop, try_resize_surface_in_canvas};
use photocraft_codecs::{self as codecs, ChannelLayout, Image, SampleType as CSample, TxEncodeOptions, TxWrap, f16, tx_level_sizes};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::Document;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};

use crate::{ExportResult, IoError};

/// Sample type of the texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TxDataType {
    /// The document's own depth (`maketx`'s default: as the input).
    #[default]
    AsDocument,
    U8,
    U16,
    /// 16-bit float.
    Half,
    /// 32-bit float.
    Float,
}

/// Filter that builds the MIP levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TxFilter {
    /// Lanczos with three lobes (`maketx --filter lanczos3`), the classic choice for 8 and 16 bit.
    #[default]
    Lanczos3,
    /// `maketx --filter triangle`: a tent as wide as two output pixels, never negative. For HDR:
    /// Lanczos rings around very bright texels (measured on a sun HDRI: down to −262 next to the
    /// sun), and `--hicomp` against that loses up to 84 % of the energy in the small levels.
    Triangle,
    /// Lanczos 3 for an 8- or 16-bit texture, triangle for a half or float one (not a maketx
    /// option; the file records the filter it took).
    Auto,
    /// `maketx --filter box`: a bilinear sample at each texel's centre (a 2×2 average when a
    /// level halves exactly), measured on maketx's output.
    Box,
}

/// Repair of NaN and infinite samples in a float image (`maketx --fixnan`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TxFixNan {
    /// Leave them (they spread through the MIP levels).
    #[default]
    Off,
    /// Replace them with 0.
    Black,
    /// Replace them with the average of the finite samples around them (3×3).
    Box3,
}

/// The ready-made settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxPreset {
    /// For V-Ray and rendering in general: the classic call plus the detections and the NaN
    /// repair `maketx` 2.6 adds by itself.
    VRay,
    /// Exactly the classic call `maketx --oiio --tile 64 64 --filter lanczos3 --monochrome-detect`.
    Classic,
}

impl TxPreset {
    pub const ALL: [TxPreset; 2] = [TxPreset::VRay, TxPreset::Classic];
}

/// Settings of a `.tx` save (the TX Options dialog).
#[derive(Debug, Clone, PartialEq)]
pub struct TxOptions {
    pub data_type: TxDataType,
    /// Tile edge in pixels: 32, 64, 128 or 256.
    pub tile: u32,
    /// Deflate with a predictor; off stores the tiles uncompressed.
    pub compress: bool,
    /// Write the MIP levels (off is `maketx --nomipmap`: the full size alone).
    pub mipmap: bool,
    pub filter: TxFilter,
    /// Wrap modes of s and t, recorded for the renderer (`--swrap`, `--twrap`).
    pub wrap: (TxWrap, TxWrap),
    /// Write an RGB image whose channels are all equal as gray.
    pub monochrome_detect: bool,
    /// Write an image of one colour as a single tile.
    pub constant_color_detect: bool,
    /// Float images only.
    pub fix_nan: TxFixNan,
    /// Compress highlights before each reduction and expand them after (`--hicomp`): less
    /// ringing around very bright texels in HDR images. Applies only to a float texture
    /// (half or float output); 8- and 16-bit textures ignore it.
    pub hicomp: bool,
}

/// Tile edges the dialog offers.
pub const TX_TILE_SIZES: [u32; 4] = [32, 64, 128, 256];

impl TxOptions {
    pub fn preset(p: TxPreset) -> Self {
        let classic = TxOptions {
            data_type: TxDataType::AsDocument,
            tile: 64,
            compress: true,
            mipmap: true,
            filter: TxFilter::Lanczos3,
            wrap: (TxWrap::Black, TxWrap::Black),
            monochrome_detect: true,
            constant_color_detect: false,
            fix_nan: TxFixNan::Off,
            hicomp: false,
        };
        match p {
            TxPreset::Classic => classic,
            TxPreset::VRay => TxOptions { constant_color_detect: true, fix_nan: TxFixNan::Box3, filter: TxFilter::Auto, ..classic },
        }
    }

    /// The preset these settings are, if any.
    pub fn as_preset(&self) -> Option<TxPreset> {
        TxPreset::ALL.into_iter().find(|p| TxOptions::preset(*p) == *self)
    }

    /// These settings with [`TxFilter::Auto`] replaced by the filter it takes for a texture that
    /// is (`float`) or isn't half/float.
    pub fn resolved(&self, float: bool) -> TxOptions {
        let mut o = self.clone();
        if o.filter == TxFilter::Auto {
            o.filter = if float { TxFilter::Triangle } else { TxFilter::Lanczos3 };
        }
        o
    }

    /// The `maketx` options that write the same file, for the `Software` tag. Call it on
    /// [`Self::resolved`] settings: `--filter auto` is no maketx option and only round-trips
    /// through [`Self::apply_maketx_args`].
    pub fn maketx_args(&self) -> String {
        let filter = match self.filter {
            TxFilter::Lanczos3 => "lanczos3",
            TxFilter::Box => "box",
            TxFilter::Triangle => "triangle",
            TxFilter::Auto => "auto",
        };
        let mut a = format!("--oiio --tile {0} {0} --filter {1}", self.tile, filter);
        let d = match self.data_type {
            TxDataType::AsDocument => None,
            TxDataType::U8 => Some("uint8"),
            TxDataType::U16 => Some("uint16"),
            TxDataType::Half => Some("half"),
            TxDataType::Float => Some("float"),
        };
        if let Some(d) = d {
            a += &format!(" -d {d}");
        }
        if !self.compress {
            a += " --compression none";
        }
        if !self.mipmap {
            a += " --nomipmap";
        }
        if self.wrap != (TxWrap::Black, TxWrap::Black) {
            a += &format!(" --swrap {} --twrap {}", self.wrap.0.name(), self.wrap.1.name());
        }
        for (on, flag) in
            [(self.monochrome_detect, " --monochrome-detect"), (self.constant_color_detect, " --constant-color-detect"), (self.hicomp, " --hicomp")]
        {
            if on {
                a += flag;
            }
        }
        match self.fix_nan {
            TxFixNan::Off => {}
            TxFixNan::Black => a += " --fixnan black",
            TxFixNan::Box3 => a += " --fixnan box3",
        }
        a
    }
}

impl TxOptions {
    /// Applies `maketx` options (the ones [`Self::maketx_args`] writes, plus `--wrap`) on top of
    /// these settings, so a script can say `--filter box --hicomp` the way `maketx` takes it.
    /// Unknown options are an error, never ignored.
    pub fn apply_maketx_args(&mut self, args: &str) -> Result<(), String> {
        let mut it = args.split_whitespace();
        let wrap = |v: Option<&str>| v.and_then(TxWrap::from_name).ok_or_else(|| "expected black, clamp, periodic or mirror".to_string());
        while let Some(a) = it.next() {
            match a {
                "--oiio" => {}
                "--tile" => {
                    let (x, y) = (it.next(), it.next());
                    let t: u32 = x.and_then(|v| v.parse().ok()).ok_or("--tile needs a size")?;
                    if y != x || !TX_TILE_SIZES.contains(&t) {
                        return Err(format!("--tile: square 32, 64, 128 or 256, got {} {}", x.unwrap_or(""), y.unwrap_or("")));
                    }
                    self.tile = t;
                }
                "--filter" => {
                    self.filter = match it.next() {
                        Some("lanczos3") => TxFilter::Lanczos3,
                        Some("box") => TxFilter::Box,
                        Some("triangle") => TxFilter::Triangle,
                        // Not a maketx filter: the V-Ray preset's choice by bit depth.
                        Some("auto") => TxFilter::Auto,
                        other => return Err(format!("--filter: lanczos3, triangle, box or auto, got {}", other.unwrap_or(""))),
                    }
                }
                "-d" => {
                    self.data_type = match it.next() {
                        Some("uint8") => TxDataType::U8,
                        Some("uint16") => TxDataType::U16,
                        Some("half") => TxDataType::Half,
                        Some("float") => TxDataType::Float,
                        other => return Err(format!("-d: uint8, uint16, half or float, got {}", other.unwrap_or(""))),
                    }
                }
                "--compression" => {
                    self.compress = match it.next() {
                        Some("zip") => true,
                        Some("none") => false,
                        other => return Err(format!("--compression: zip or none, got {}", other.unwrap_or(""))),
                    }
                }
                "--nomipmap" => self.mipmap = false,
                "--wrap" => {
                    let w = wrap(it.next()).map_err(|e| format!("--wrap: {e}"))?;
                    self.wrap = (w, w);
                }
                "--swrap" => self.wrap.0 = wrap(it.next()).map_err(|e| format!("--swrap: {e}"))?,
                "--twrap" => self.wrap.1 = wrap(it.next()).map_err(|e| format!("--twrap: {e}"))?,
                "--monochrome-detect" => self.monochrome_detect = true,
                "--constant-color-detect" => self.constant_color_detect = true,
                // Not maketx options: the V-Ray preset has both detections on, so a script needs a way off.
                "--no-monochrome-detect" => self.monochrome_detect = false,
                "--no-constant-color-detect" => self.constant_color_detect = false,
                "--hicomp" => self.hicomp = true,
                "--fixnan" => {
                    self.fix_nan = match it.next() {
                        Some("none") => TxFixNan::Off,
                        Some("black") => TxFixNan::Black,
                        Some("box3") => TxFixNan::Box3,
                        other => return Err(format!("--fixnan: none, black or box3, got {}", other.unwrap_or(""))),
                    }
                }
                other => return Err(format!("unsupported maketx option `{other}`")),
            }
        }
        Ok(())
    }
}

impl Default for TxOptions {
    fn default() -> Self {
        TxOptions::preset(TxPreset::VRay)
    }
}

/// Writes `doc` as a `.tx` texture.
pub(crate) fn export_tx(doc: &Document, o: &TxOptions, ctl: &Interrupt) -> Result<ExportResult, IoError> {
    if !TX_TILE_SIZES.contains(&o.tile) {
        return Err(IoError::Unsupported(format!("TX tile size {} (32, 64, 128 or 256)", o.tile)));
    }
    let mode = doc.pixel_format().mode;
    if matches!(mode, ColorMode::Multichannel | ColorMode::Duotone) {
        return Err(IoError::Unsupported(format!("a {mode:?} document can't be written as a .tx texture; convert it to RGB or Grayscale first")));
    }
    let mut warnings = Vec::new();
    // CMYK and Lab go through the sRGB composite (the texture is gray or RGB).
    let mut img = crate::flat::document_to_image_for(doc, &mut warnings, false, false)?;
    if img.layout().is_cmyk() {
        // A single CMYK layer is handed over natively; the texture holds RGB.
        img = crate::flat::cmyk_image_to_srgb(&img)?;
        warnings.push("CMYK converted to sRGB through the document's colour profile".into());
    }
    if !matches!(img.layout(), ChannelLayout::Gray | ChannelLayout::GrayA | ChannelLayout::Rgb | ChannelLayout::Rgba) {
        return Err(IoError::Unsupported(format!("a {:?} image can't be written as a .tx texture", img.layout())));
    }
    let target = match o.data_type {
        TxDataType::AsDocument => img.sample_type(),
        TxDataType::U8 => CSample::U8,
        TxDataType::U16 => CSample::U16,
        TxDataType::Half => CSample::F16,
        TxDataType::Float => CSample::F32,
    };
    // The settings that take effect here: the NaN repair needs float samples to repair, highlight
    // compression a float texture with MIP levels. The Software tag and the fingerprint name
    // these, never an option that changed nothing.
    let mut o = o.resolved(matches!(target, CSample::F16 | CSample::F32));
    if !matches!(img.sample_type(), CSample::F32 | CSample::F16) {
        o.fix_nan = TxFixNan::Off;
    }
    if !(o.mipmap && matches!(target, CSample::F16 | CSample::F32)) {
        o.hicomp = false;
    }
    let o = &o;
    if o.fix_nan != TxFixNan::Off {
        let fixed = fix_nan(&mut img, o.fix_nan)?;
        if fixed > 0 {
            warnings.push(format!("{fixed} NaN or infinite sample(s) repaired"));
        }
    }
    if o.monochrome_detect && img.layout() == ChannelLayout::Rgb && is_monochrome(&img) {
        img = first_channel(&img)?;
    }
    // Asked to repair NaN/Inf: the texture stays finite on every level (see `quantize`).
    let repair = o.fix_nan != TxFixNan::Off;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let layout = img.layout();
    let constant = o.constant_color_detect.then(|| constant_pixel(&img)).flatten();
    // Nothing is reduced for one colour or a single level: the filter and highlight compression
    // then change nothing and are left out of the Software tag and the fingerprint.
    let mut eff = o.clone();
    if constant.is_some() || tx_level_sizes(img.width(), img.height()).len() < 2 || !o.mipmap {
        eff.hicomp = false;
        eff.filter = TxFilter::default();
    }
    let o = &eff;
    let levels = if let Some(px) = constant {
        // One colour: a texture of at most one tile, every level the same colour.
        let mut sizes = tx_level_sizes(img.width().min(o.tile), img.height().min(o.tile));
        if !o.mipmap {
            sizes.truncate(1);
        }
        let mut px = px;
        premultiply(&mut px, layout);
        sizes.into_iter().map(|(w, h)| quantize(&px.repeat((w * h) as usize), w, h, layout, target, repair)).collect::<Result<Vec<_>, _>>()?
    } else {
        mip_chain(&img, o, target, repair, ctl)?
    };
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(0.85);
    let opts = TxEncodeOptions {
        tile: o.tile,
        deflate: o.compress,
        wrap: o.wrap,
        software: Some(format!("PhotoCraft {} : maketx-equivalent {}", env!("CARGO_PKG_VERSION"), o.maketx_args())),
        date_time: None,
        // Into the SHA-1 with the samples: two textures whose top levels match but whose lower
        // levels differ (filter, hicomp, no MIP levels) must not share a fingerprint, or an
        // OpenImageIO texture cache would treat them as duplicates (maketx hashes its options too).
        hash_extra: Some(o.maketx_args()),
        dpi: Some((f64::from(doc.resolution_dpi), f64::from(doc.resolution_dpi))),
    };
    let cancelled = || ctl.cancelled();
    let bytes = codecs::encode_tx_with(&levels, &opts, &cancelled).map_err(|e| if ctl.cancelled() { IoError::Cancelled } else { e.into() })?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    Ok(ExportResult { bytes, warnings })
}

/// Rows converted to floats at a time for the top level.
const BAND_PIXELS: usize = 1 << 22;

/// The working surfaces' default pixel: a value real samples practically never hold (the smallest
/// subnormal). The resampler writes only over a surface's content, the pixels that differ from its
/// default; with 0 as the default, a stripe of fully transparent (all-zero) columns lay outside
/// it, and the Lanczos ringing that maketx writes there was left out (measured: up to 0.05 next to
/// a hard alpha edge). A pixel that does hold this value is outside the content and reads as
/// this value, so it changes nothing.
const UNSEEN: f32 = f32::from_bits(1);

/// One MIP level in floats, premultiplied, as two alpha-free surfaces: the colour channels and,
/// for an image with alpha, the alpha. Filtering them as plain channels is filtering the
/// premultiplied pixel, as `maketx` does: nothing is clipped and nothing is divided back (the
/// resampler would clamp a straight alpha that rings below 0 and lose the colour on it).
struct Planes {
    color: Surface,
    alpha: Option<Surface>,
}

impl Planes {
    fn new(layout: ChannelLayout) -> Self {
        let mode = if layout.is_gray() { ColorMode::Grayscale } else { ColorMode::Rgb };
        let colors = if layout.is_gray() { 1 } else { 3 };
        let alpha = layout.has_alpha().then(|| Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[UNSEEN]));
        Planes { color: Surface::with_default(PixelFormat::new(mode, SampleType::F32, false), &[UNSEEN; 3][..colors]), alpha }
    }

    /// Stores rows `r` of interleaved premultiplied pixels.
    fn write(&mut self, r: Rect, px: &[f32], n: usize) -> Result<(), IoError> {
        let oom = |_| IoError::Unsupported("not enough memory for the texture".into());
        let Some(alpha) = &mut self.alpha else { return self.color.try_write_region(r, px).map_err(oom) };
        let mut color = try_f32_buffer(px.len() / n * (n - 1))?;
        let mut a = try_f32_buffer(px.len() / n)?;
        for p in px.chunks_exact(n) {
            color.extend_from_slice(p.get(..n - 1).unwrap_or(&[]));
            a.push(p.get(n - 1).copied().unwrap_or(1.0));
        }
        self.color.try_write_region(r, &color).map_err(oom)?;
        alpha.try_write_region(r, &a).map_err(oom)
    }

    /// The level `(0, 0)..(w, h)` stored at `target` (see [`quantize`]), read and converted a
    /// band of rows at a time, so no full-size interleaved float copy is made.
    fn quantize(&self, w: u32, h: u32, layout: ChannelLayout, target: CSample, repair: bool) -> Result<Image, IoError> {
        let oom = |_| IoError::Unsupported("not enough memory for the texture".into());
        let n = layout.channels();
        let mut data = crate::flat::try_buffer((w as usize).saturating_mul(h as usize).saturating_mul(n), target.bytes())?;
        let rows = (BAND_PIXELS / (w as usize).max(1)).max(1);
        let mut y = 0usize;
        while y < h as usize {
            let y1 = (y + rows).min(h as usize);
            let r = Rect::new(0, y as i32, w as i32, y1 as i32);
            let color = self.color.try_read_region(r).map_err(oom)?;
            let band = match &self.alpha {
                None => color,
                Some(alpha) => {
                    let a = alpha.try_read_region(r).map_err(oom)?;
                    let mut px = try_f32_buffer(color.len() + a.len())?;
                    for (c, a) in color.chunks_exact(n - 1).zip(a) {
                        px.extend_from_slice(c);
                        px.push(a);
                    }
                    px
                }
            };
            data.extend_from_slice(quantize(&band, w, (y1 - y) as u32, layout, target, repair)?.data());
            y = y1;
        }
        Ok(Image::from_raw(w, h, layout, target, data)?)
    }
}

/// The levels, full size first: the top one from the image's own samples, each further one
/// reduced from the one above (premultiplied floats), all stored premultiplied at `target`.
fn mip_chain(img: &Image, o: &TxOptions, target: CSample, repair: bool, ctl: &Interrupt) -> Result<Vec<Image>, IoError> {
    let (w, h) = img.dimensions();
    let layout = img.layout();
    let n = layout.channels();
    let sizes = if o.mipmap { tx_level_sizes(w, h) } else { vec![(w, h)] };
    // The top level, and the planes the next level is reduced from, a band of rows at a time.
    let mut cur = Planes::new(layout);
    let mut top = crate::flat::try_buffer((w as usize).saturating_mul(h as usize).saturating_mul(n), target.bytes())?;
    let hicomp = o.hicomp;
    let row_px = w as usize;
    let rows = (BAND_PIXELS / row_px.max(1)).max(1);
    let mut y = 0usize;
    while y < h as usize {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let y1 = (y + rows).min(h as usize);
        let mut band = to_f32(img, y * row_px * n, y1 * row_px * n)?;
        premultiply(&mut band, layout);
        if sizes.len() > 1 {
            cur.write(Rect::new(0, y as i32, w as i32, y1 as i32), &band, n)?;
        }
        top.extend_from_slice(quantize(&band, w, (y1 - y) as u32, layout, target, repair)?.data());
        y = y1;
    }
    let mut levels = vec![Image::from_raw(w, h, layout, target, top)?];
    let count = sizes.len();
    for (i, pair) in sizes.windows(2).enumerate() {
        let &[(w0, h0), (w1, h1)] = pair else { continue };
        ctl.check().map_err(|_| IoError::Cancelled)?;
        if hicomp {
            // On the colour channels only, as OpenImageIO's rangecompress leaves alpha alone.
            cur.color.map_color(range_compress);
        }
        let mut next = Planes { color: reduce(&cur.color, (w0, h0), (w1, h1), o.filter, ctl)?, alpha: None };
        if let Some(a) = &cur.alpha {
            next.alpha = Some(reduce(a, (w0, h0), (w1, h1), o.filter, ctl)?);
        }
        if hicomp {
            // As maketx after `rangeexpand`: colour clamped to ≥ 0 and alpha to 0..1 (measured on
            // 1.4.12 and 2.6.3 with a hard alpha edge; without --hicomp neither clamps).
            next.color.map_color(|v| range_expand(v).max(0.0));
            if let Some(a) = &mut next.alpha {
                a.map_color(|v| v.clamp(0.0, 1.0));
            }
        }
        levels.push(next.quantize(w1, h1, layout, target, repair)?);
        cur = next;
        ctl.progress(0.85 * (i + 2) as f32 / count as f32);
    }
    Ok(levels)
}

/// One level smaller: `s` (`w0`×`h0`, alpha-free) reduced to `w1`×`h1`, the edges repeating.
fn reduce(s: &Surface, (w0, h0): (u32, u32), (w1, h1): (u32, u32), filter: TxFilter, ctl: &Interrupt) -> Result<Surface, IoError> {
    let canvas = Rect::new(0, 0, w0 as i32, h0 as i32);
    match filter {
        // Cancellable between bands, and an error rather than an abort when memory runs out.
        TxFilter::Lanczos3 => try_resize_surface_in_canvas(s, f64::from(w1) / f64::from(w0), f64::from(h1) / f64::from(h0), Resample::Lanczos, canvas, ctl)
            .map_err(|e| match e {
                ResizeStop::Cancelled => IoError::Cancelled,
                ResizeStop::OutOfMemory => IoError::Unsupported("not enough memory for the texture".into()),
            }),
        TxFilter::Triangle | TxFilter::Auto => {
            // As Box: a row of the result at a time from the source rows its tent covers.
            let oom = |_| IoError::Unsupported("not enough memory for the texture".into());
            let n = s.channels();
            let cols: Vec<(usize, Vec<f32>)> = (0..w1 as usize).map(|x| tent_taps(x, w0 as usize, w1 as usize)).collect();
            let mut out = Surface::with_default(s.format(), &s.default_pixel());
            for y in 0..h1 as usize {
                if y % 64 == 0 {
                    ctl.check().map_err(|_| IoError::Cancelled)?;
                }
                let (r0, wy) = tent_taps(y, h0 as usize, h1 as usize);
                let rows = s.try_read_region(Rect::new(0, r0 as i32, w0 as i32, (r0 + wy.len()) as i32)).map_err(oom)?;
                let px = tent_row(&rows, n, w0 as usize, &wy, &cols);
                out.try_write_region(Rect::new(0, y as i32, w1 as i32, y as i32 + 1), &px).map_err(oom)?;
            }
            Ok(out)
        }
        TxFilter::Box => {
            // A row of the result at a time from the two source rows it samples, so no
            // full-size float copy of the level is made.
            let oom = |_| IoError::Unsupported("not enough memory for the texture".into());
            let n = s.channels();
            let mut out = Surface::with_default(s.format(), &s.default_pixel());
            for y in 0..h1 as usize {
                if y % 64 == 0 {
                    ctl.check().map_err(|_| IoError::Cancelled)?;
                }
                let (r0, r1, fy) = lerp_taps(y, h0 as usize, h1 as usize);
                let rows = s.try_read_region(Rect::new(0, r0 as i32, w0 as i32, r1 as i32 + 1)).map_err(oom)?;
                let px = bilinear_row(&rows, n, w0 as usize, r1 - r0, fy, w1 as usize);
                out.try_write_region(Rect::new(0, y as i32, w1 as i32, y as i32 + 1), &px).map_err(oom)?;
            }
            Ok(out)
        }
    }
}

/// The two source pixels and the weight of the second for output `o` of `n1` from `n0`: the
/// output pixel's centre mapped into the source, edges clamped.
fn lerp_taps(o: usize, n0: usize, n1: usize) -> (usize, usize, f32) {
    let last = n0.saturating_sub(1) as f64;
    let c = ((o as f64 + 0.5) * n0 as f64 / n1 as f64 - 0.5).clamp(0.0, last);
    let i0 = c.floor();
    let i1 = (i0 + 1.0).min(last);
    (i0 as usize, i1 as usize, (c - i0) as f32)
}

/// `maketx --filter box`, as measured on its output (1.4.12 and 2.6.3 agree bit for bit): not a
/// box but a bilinear sample of the level above at each output pixel's centre — the plain 2×2
/// average when a level halves exactly, only the four nearest pixels when it doesn't (125 → 62).
/// One output row from `rows` (source rows `r0..=r0 + dy`, `w0` wide, `n` channels): the first
/// and the last of them, weighted `1 - fy` and `fy`.
fn bilinear_row(rows: &[f32], n: usize, w0: usize, dy: usize, fy: f32, w1: usize) -> Vec<f32> {
    let at = |x: usize, y: usize, c: usize| rows.get((y * w0 + x) * n + c).copied().unwrap_or(0.0);
    let mut out = vec![0.0f32; w1 * n];
    for x in 0..w1 {
        let (x0, x1, fx) = lerp_taps(x, w0, w1);
        for c in 0..n {
            let top = at(x0, 0, c) * (1.0 - fx) + at(x1, 0, c) * fx;
            let bot = at(x0, dy, c) * (1.0 - fx) + at(x1, dy, c) * fx;
            if let Some(d) = out.get_mut(x * n + c) {
                *d = top * (1.0 - fy) + bot * fy;
            }
        }
    }
    out
}

/// `maketx --filter triangle`, as measured on its output (2.6.3 and 3.2.1 agree, and this matches
/// both to float precision on every level, odd sizes included): output `o` of `n1` from `n0` is a
/// tent `n0 / n1` source pixels wide on each side of its centre `(o + 0.5) · n0 / n1`, the edge
/// samples repeating outwards, the weights summing to 1. The first source sample and the weights.
fn tent_taps(o: usize, n0: usize, n1: usize) -> (usize, Vec<f32>) {
    let r = n0 as f64 / n1.max(1) as f64;
    let c = (o as f64 + 0.5) * r;
    let last = n0.saturating_sub(1) as i64;
    let (lo, hi) = ((c - r - 0.5).floor() as i64, (c + r - 0.5).ceil() as i64);
    let first = lo.clamp(0, last);
    let mut w = vec![0.0f64; (hi.clamp(0, last) - first + 1) as usize];
    for i in lo..=hi {
        let d = ((i as f64 + 0.5 - c) / r).abs();
        if d < 1.0
            && let Some(slot) = w.get_mut((i.clamp(0, last) - first) as usize)
        {
            *slot += 1.0 - d;
        }
    }
    let sum: f64 = w.iter().sum();
    (first as usize, w.into_iter().map(|v| (v / sum) as f32).collect())
}

/// One output row from `rows` (source rows `r0..r0 + wy.len()`, `w0` wide, `n` channels): the
/// rows weighted with `wy` into one, then each output pixel from its column taps.
fn tent_row(rows: &[f32], n: usize, w0: usize, wy: &[f32], cols: &[(usize, Vec<f32>)]) -> Vec<f32> {
    let mut line = vec![0.0f32; w0 * n];
    for (k, &wk) in wy.iter().enumerate() {
        for (d, v) in line.iter_mut().zip(rows.iter().skip(k * w0 * n)) {
            *d += wk * v;
        }
    }
    let mut out = Vec::with_capacity(cols.len() * n);
    for (x0, wx) in cols {
        for c in 0..n {
            out.push(wx.iter().enumerate().map(|(k, w)| w * line.get((x0 + k) * n + c).copied().unwrap_or(0.0)).sum());
        }
    }
    out
}

/// [`bilinear_row`] over every row of a `w0`×`h0` image (tests).
#[cfg(test)]
fn bilinear_reduce(src: &[f32], n: usize, (w0, h0): (usize, usize), (w1, h1): (usize, usize)) -> Vec<f32> {
    (0..h1)
        .flat_map(|y| {
            let (r0, r1, fy) = lerp_taps(y, h0, h1);
            let rows = src.get(r0 * w0 * n..(r1 + 1) * w0 * n).unwrap_or(&[]);
            bilinear_row(rows, n, w0, r1 - r0, fy, w1)
        })
        .collect()
}

/// An empty float buffer with room for `len` samples, or an error when memory runs out.
fn try_f32_buffer(len: usize) -> Result<Vec<f32>, IoError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| IoError::Unsupported("not enough memory for the texture".into()))?;
    Ok(v)
}

/// Straight alpha → premultiplied, in place.
fn premultiply(px: &mut [f32], layout: ChannelLayout) {
    if !layout.has_alpha() {
        return;
    }
    let n = layout.channels();
    for p in px.chunks_exact_mut(n) {
        let a = p.get(n - 1).copied().unwrap_or(1.0);
        for v in p.iter_mut().take(n - 1) {
            *v *= a;
        }
    }
}

/// Samples `from..to` of `img` as normalised floats (8 and 16 bit to 0..1).
fn to_f32(img: &Image, from: usize, to: usize) -> Result<Vec<f32>, IoError> {
    let s = img.sample_type();
    let b = s.bytes();
    let bytes = img.data().get(from * b..to * b).ok_or_else(|| IoError::Unsupported("texture rows out of range".into()))?;
    let mut out = try_f32_buffer(to.saturating_sub(from))?;
    match s {
        CSample::U8 => out.extend(bytes.iter().map(|&v| f32::from(v) / 255.0)),
        CSample::U16 => out.extend(bytes.as_chunks::<2>().0.iter().map(|c| f32::from(u16::from_ne_bytes(*c)) / 65535.0)),
        CSample::F16 => out.extend(bytes.as_chunks::<2>().0.iter().map(|c| f16::from_bits(u16::from_ne_bytes(*c)).to_f32())),
        CSample::F32 => out.extend(bytes.as_chunks::<4>().0.iter().map(|c| f32::from_ne_bytes(*c))),
    }
    Ok(out)
}

/// Floats (already premultiplied) → samples of `target`, rounded (8/16 bit clipped to 0..1).
/// `repair` (NaN/Inf repair asked for): a float texture stays finite on every level — half-float
/// values beyond ±65504 are held there, and a float sample the reduction pushed to ±Inf (finite
/// values near f32::MAX) or NaN becomes ±f32::MAX or 0 — instead of bringing back what was repaired.
fn quantize(px: &[f32], w: u32, h: u32, layout: ChannelLayout, target: CSample, repair: bool) -> Result<Image, IoError> {
    let mut out = crate::flat::try_buffer(px.len(), target.bytes())?;
    for &v in px {
        {
            match target {
                CSample::U8 => out.push((v.clamp(0.0, 1.0) * 255.0).round() as u8),
                CSample::U16 => out.extend_from_slice(&((v.clamp(0.0, 1.0) * 65535.0).round() as u16).to_ne_bytes()),
                CSample::F16 => {
                    let v = if repair && v.is_nan() {
                        0.0
                    } else if repair {
                        v.clamp(-65504.0, 65504.0)
                    } else {
                        v
                    };
                    out.extend_from_slice(&f16::from_f32(v).to_bits().to_ne_bytes())
                }
                CSample::F32 => {
                    let v = if repair && !v.is_finite() { if v.is_nan() { 0.0 } else { f32::MAX.copysign(v) } } else { v };
                    out.extend_from_slice(&v.to_ne_bytes())
                }
            }
        }
    }
    Ok(Image::from_raw(w, h, layout, target, out)?)
}

/// `true` when every pixel of an RGB image has three bit-identical channels.
fn is_monochrome(img: &Image) -> bool {
    let b = img.sample_type().bytes();
    img.data().chunks_exact(3 * b).all(|p| p.get(..b) == p.get(b..2 * b) && p.get(..b) == p.get(2 * b..3 * b))
}

/// The first channel of an RGB image as gray (for a monochrome image: the image itself).
fn first_channel(img: &Image) -> Result<Image, IoError> {
    let b = img.sample_type().bytes();
    let mut out = crate::flat::try_buffer(img.pixel_count(), b)?;
    for p in img.data().chunks_exact(3 * b) {
        out.extend_from_slice(p.get(..b).unwrap_or(&[]));
    }
    Ok(Image::from_raw(img.width(), img.height(), ChannelLayout::Gray, img.sample_type(), out)?)
}

/// The colour of an image whose pixels are all bit-identical, as straight floats.
fn constant_pixel(img: &Image) -> Option<Vec<f32>> {
    let px = img.layout().channels() * img.sample_type().bytes();
    let first = img.data().get(..px)?;
    if !img.data().chunks_exact(px).all(|p| p == first) {
        return None;
    }
    to_f32(img, 0, img.layout().channels()).ok()
}

/// Repairs NaN and infinite samples of a float image in place; returns how many there were.
/// Box3 averages the finite samples of the same channel in the 3×3 block around each one, read
/// from the image as it was before any repair (a copy of its samples, made only when there is
/// something to repair), or writes 0 when there are none.
fn fix_nan(img: &mut Image, how: TxFixNan) -> Result<usize, IoError> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    if w == 0 || h == 0 {
        return Ok(0);
    }
    let n = img.layout().channels();
    let sample = img.sample_type();
    let b = sample.bytes();
    let get = |d: &[u8], i: usize| -> f32 {
        match sample {
            CSample::F32 => d.get(i * 4..i * 4 + 4).map_or(0.0, |c| f32::from_ne_bytes([c[0], c[1], c[2], c[3]])),
            _ => d.get(i * 2..i * 2 + 2).map_or(0.0, |c| f16::from_bits(u16::from_ne_bytes([c[0], c[1]])).to_f32()),
        }
    };
    let total = w * h * n;
    if (0..total).all(|i| get(img.data(), i).is_finite()) {
        return Ok(0);
    }
    let original = if how == TxFixNan::Box3 {
        let mut v = crate::flat::try_buffer(img.data().len(), 1)?;
        v.extend_from_slice(img.data());
        Some(v)
    } else {
        None
    };
    let mut fixed = 0;
    for i in 0..total {
        if get(img.data(), i).is_finite() {
            continue;
        }
        let v = match &original {
            Some(o) => {
                let (p, c) = (i / n, i % n);
                let (x, y) = (p % w, p / w);
                let (mut sum, mut cnt) = (0.0f64, 0u32);
                for yy in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                    for xx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                        let s = get(o, (yy * w + xx) * n + c);
                        if s.is_finite() {
                            sum += f64::from(s);
                            cnt += 1;
                        }
                    }
                }
                if cnt > 0 { (sum / f64::from(cnt)) as f32 } else { 0.0 }
            }
            None => 0.0,
        };
        if let Some(d) = img.data_mut().get_mut(i * b..i * b + b) {
            match sample {
                CSample::F32 => d.copy_from_slice(&v.to_ne_bytes()),
                _ => d.copy_from_slice(&f16::from_f32(v).to_bits().to_ne_bytes()),
            }
        }
        fixed += 1;
    }
    Ok(fixed)
}

// OpenImageIO's highlight compression (`rangecompress` / `rangeexpand`, "formula courtesy of Sony
// Pictures Imageworks", the coefficients OIIO uses): identity up to 0.18, logarithmic above,
// symmetric around 0. Applied to the colour channels, not to alpha, as OIIO does.
const RC_X1: f32 = 0.18;
const RC_A: f32 = -0.545_768_86;
const RC_B: f32 = 0.183_516_7;
const RC_C: f32 = 284.357_8;

// Evaluated in f64: `c·x` and `exp(…)` overflow f32 for finite samples near f32::MAX.
fn range_compress(x: f32) -> f32 {
    let ax = f64::from(x.abs());
    if ax <= f64::from(RC_X1) || !x.is_finite() {
        return x;
    }
    ((f64::from(RC_A) + f64::from(RC_B) * (f64::from(RC_C) * ax + 1.0).abs().ln()) as f32).copysign(x)
}

fn range_expand(y: f32) -> f32 {
    let ay = y.abs();
    if ay <= RC_X1 || !y.is_finite() {
        return y;
    }
    let (ay, a, b, c) = (f64::from(ay), f64::from(RC_A), f64::from(RC_B), f64::from(RC_C));
    let e = ((ay - a) / b).exp();
    // The compression takes an absolute value, so there are two roots; below x1 is the wrong one.
    let mut x = (e - 1.0) / c;
    if x < f64::from(RC_X1) {
        x = (-e - 1.0) / c;
    }
    // Overshoot near f32::MAX must not leave the range of the sample type.
    (x.min(f64::from(f32::MAX)) as f32).copysign(y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets() {
        assert_eq!(TxOptions::default().as_preset(), Some(TxPreset::VRay));
        let s = TxOptions::preset(TxPreset::Classic);
        assert_eq!(s.as_preset(), Some(TxPreset::Classic));
        assert_eq!(s.maketx_args(), "--oiio --tile 64 64 --filter lanczos3 --monochrome-detect");
        assert_eq!(TxOptions { tile: 128, ..s }.as_preset(), None);
    }

    #[test]
    fn maketx_args_round_trip() {
        let all = [
            TxOptions::default(),
            TxOptions::preset(TxPreset::Classic),
            TxOptions {
                data_type: TxDataType::Half,
                tile: 128,
                compress: false,
                mipmap: false,
                filter: TxFilter::Box,
                wrap: (TxWrap::Periodic, TxWrap::Mirror),
                monochrome_detect: false,
                constant_color_detect: false,
                fix_nan: TxFixNan::Black,
                hicomp: true,
            },
        ];
        for o in all {
            let mut back = TxOptions { monochrome_detect: false, constant_color_detect: false, fix_nan: TxFixNan::Off, ..Default::default() };
            back.apply_maketx_args(&o.maketx_args()).unwrap();
            assert_eq!(back, o, "{}", o.maketx_args());
        }
        let mut o = TxOptions::default();
        let mut off = TxOptions::default();
        off.apply_maketx_args("--no-monochrome-detect --no-constant-color-detect").unwrap();
        assert!(!off.monochrome_detect && !off.constant_color_detect);
        for bad in ["--sharpen 0.5", "--tile 64 32", "--tile 48 48", "--filter catrom", "--wrap edge", "-d double", "--fixnan"] {
            assert!(o.apply_maketx_args(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn range_compress_round_trips() {
        // Finite samples near f32::MAX stay finite both ways.
        assert!(range_compress(1e37).is_finite() && range_expand(range_compress(3e38)).is_finite());
        for v in [0.0f32, 0.1, 0.18, 0.2, 1.0, 4.0, 50.0, 1000.0, -3.0] {
            let c = range_compress(v);
            assert!(c.abs() <= v.abs() + 1e-6, "{v} -> {c}");
            assert!((range_expand(c) - v).abs() <= 1e-3 * v.abs().max(1.0), "{v} -> {c} -> {}", range_expand(c));
        }
        // Continuous at the knee.
        assert!((range_compress(0.180_01) - 0.18).abs() < 1e-3);
    }

    #[test]
    fn fix_nan_box3_and_black() {
        let vals = [1.0f32, 2.0, 3.0, 4.0, f32::NAN, 6.0, 7.0, 8.0, f32::INFINITY];
        let mk = || Image::from_raw(3, 3, ChannelLayout::Gray, CSample::F32, vals.iter().flat_map(|v| v.to_ne_bytes()).collect()).unwrap();
        let mut img = mk();
        assert_eq!(fix_nan(&mut img, TxFixNan::Box3).unwrap(), 2);
        let got = to_f32(&img, 0, 9).unwrap();
        assert_eq!(got[4], (1.0 + 2.0 + 3.0 + 4.0 + 6.0 + 7.0 + 8.0) / 7.0);
        assert_eq!(got[8], (6.0 + 8.0) / 2.0, "the corner averages its finite neighbours, not the repaired centre");
        let mut img = mk();
        assert_eq!(fix_nan(&mut img, TxFixNan::Black).unwrap(), 2);
        assert_eq!(to_f32(&img, 0, 9).unwrap()[4], 0.0);
    }

    #[test]
    fn premultiplies_with_rounding() {
        // 200 at alpha 100: 200·100/255 = 78.43 → 78 (1.4.12 truncates to 78 too; 201 → 78.82 → 79).
        let mut px = [200.0 / 255.0, 201.0 / 255.0, 0.0, 100.0 / 255.0];
        premultiply(&mut px, ChannelLayout::Rgba);
        let img = quantize(&px, 1, 1, ChannelLayout::Rgba, CSample::U8, false).unwrap();
        assert_eq!(img.data(), &[78, 79, 0, 100]);
    }

    #[test]
    fn box_is_maketx_bilinear() {
        // Exact halving: the centre falls between two pixels, half each.
        assert_eq!(lerp_taps(0, 8, 4), (0, 1, 0.5));
        assert_eq!(lerp_taps(3, 8, 4), (6, 7, 0.5));
        // Edges clamp.
        assert_eq!(lerp_taps(0, 3, 1), (1, 2, 0.0));
        assert_eq!(lerp_taps(0, 1, 1), (0, 0, 0.0));
        assert_eq!(bilinear_reduce(&[0.0, 1.0, 2.0, 3.0], 1, (4, 1), (2, 1)), vec![0.5, 2.5]);
        // 125 → 62: the centre of output 0 lies at 0.508, between source 0 and 1.
        let (i0, i1, f) = lerp_taps(0, 125, 62);
        assert_eq!((i0, i1), (0, 1));
        assert!((f - 0.508_064_5).abs() < 1e-6);
    }

    #[test]
    fn triangle_is_maketx_tent() {
        // The weights over all `n0` source samples.
        let dense = |o, n0, n1| {
            let (a, w) = tent_taps(o, n0, n1);
            let mut d = vec![0.0f32; n0];
            for (k, v) in w.into_iter().enumerate() {
                d[a + k] += v;
            }
            d
        };
        let near = |g: &[f32], e: &[f32]| g.len() == e.len() && g.iter().zip(e).all(|(g, e)| (g - e).abs() < 1e-6);
        // Exact halving: 1/8, 3/8, 3/8, 1/8 around the centre, the edge sample repeated.
        assert!(near(&dense(1, 8, 4), &[0.0, 0.125, 0.375, 0.375, 0.125, 0.0, 0.0, 0.0]));
        assert!(near(&dense(0, 8, 4), &[0.5, 0.375, 0.125, 0.0, 0.0, 0.0, 0.0, 0.0]));
        // In range, never negative, summing to 1.
        for (n0, n1) in [(125, 62), (77, 38), (3, 1), (2, 1), (1, 1)] {
            for o in 0..n1 {
                let (a, w) = tent_taps(o, n0, n1);
                assert!(a + w.len() <= n0 && w.iter().all(|v| *v >= 0.0) && (w.iter().sum::<f32>() - 1.0).abs() < 1e-5, "{n0}->{n1} {o}");
            }
        }
        // A bright sample spreads instead of ringing: nothing drops below the background.
        let mut src = [0.1f32; 16];
        src[5] = 5000.0;
        let cols: Vec<_> = (0..2).map(|x| tent_taps(x, 4, 2)).collect();
        let (r0, wy) = tent_taps(0, 4, 2);
        let row = tent_row(&src[r0 * 4..(r0 + wy.len()) * 4], 1, 4, &wy, &cols);
        assert!(row.len() == 2 && row.iter().all(|v| *v >= 0.1), "{row:?}");
    }

    #[test]
    fn auto_filter_by_bit_depth() {
        let mut o = TxOptions::default();
        assert_eq!(o.filter, TxFilter::Auto);
        assert_eq!(TxOptions::preset(TxPreset::Classic).filter, TxFilter::Lanczos3, "the historic call stays as it was");
        assert_eq!(o.resolved(true).filter, TxFilter::Triangle);
        assert_eq!(o.resolved(false).filter, TxFilter::Lanczos3);
        assert!(o.resolved(true).maketx_args().contains("--filter triangle"));
        o.apply_maketx_args("--filter triangle").unwrap();
        assert_eq!(o.filter, TxFilter::Triangle);
    }

    #[test]
    fn detections() {
        let gray = Image::from_raw(2, 1, ChannelLayout::Rgb, CSample::U8, vec![5, 5, 5, 9, 9, 9]).unwrap();
        assert!(is_monochrome(&gray));
        assert_eq!(first_channel(&gray).unwrap().data(), &[5, 9]);
        let not = Image::from_raw(2, 1, ChannelLayout::Rgb, CSample::U8, vec![5, 5, 5, 9, 9, 8]).unwrap();
        assert!(!is_monochrome(&not));
        assert!(constant_pixel(&not).is_none());
        let flat = Image::from_raw(2, 1, ChannelLayout::Rgb, CSample::U8, vec![0, 51, 255, 0, 51, 255]).unwrap();
        assert_eq!(constant_pixel(&flat), Some(vec![0.0, 0.2, 1.0]));
    }
}
