//! Writer for `.tx` textures: the tiled, MIP-mapped TIFF that OpenImageIO's `maketx` writes and
//! renderers (V-Ray's VRayBitmap, Arnold, RenderMan) page in tile by tile.
//!
//! The layout follows `maketx --oiio` (OpenImageIO 1.4.12, measured on its output):
//!
//! * classic little-endian TIFF, one directory per MIP level in the main chain, full size first,
//!   each level half the previous (rounded down) down to 1×1; no `NewSubfileType` (the levels
//!   are plain pages, as `maketx` writes them);
//! * every level tiled (64×64 by default, the 1×1 level too), samples interleaved;
//! * Deflate with predictor 2 (integer) or 3 (floating point), or no compression;
//! * alpha *associated* (premultiplied), `ExtraSamples = 1`; the caller premultiplies;
//! * `ImageDescription = "SHA-1=<hex>"` (here: of the top level's samples as written),
//!   `Software`, `DateTime`, and Pixar's tags 33302 `PixarTextureFormat = "Plain Texture"`,
//!   33303 `PixarWrapModes = "<s>,<t>"` and 33304 `PixarFOVCOT` (width / height), repeated in
//!   every directory as 1.4.12 does.
//!
//! This is not a [`crate::Format`]: a `.tx` is a TIFF, which the TIFF decoder already reads; only
//! this writer is separate, so a texture is written only when asked for by name.

use std::io::Write;

use crate::error::CodecError;
use crate::format::Format;
use crate::image::{ChannelLayout, Image, SampleType};

/// Wrap mode of one texture axis, recorded in `PixarWrapModes` (it doesn't change the pixels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TxWrap {
    /// Outside the texture is black (`maketx`'s default).
    #[default]
    Black,
    Clamp,
    Periodic,
    Mirror,
}

impl TxWrap {
    pub const ALL: [TxWrap; 4] = [TxWrap::Black, TxWrap::Clamp, TxWrap::Periodic, TxWrap::Mirror];

    /// The name `maketx --wrap` takes and `PixarWrapModes` stores.
    pub const fn name(self) -> &'static str {
        match self {
            TxWrap::Black => "black",
            TxWrap::Clamp => "clamp",
            TxWrap::Periodic => "periodic",
            TxWrap::Mirror => "mirror",
        }
    }

    /// The mode named `name` (as [`Self::name`] spells it).
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|w| w.name() == name)
    }
}

/// How [`encode_tx`] writes the levels.
#[derive(Debug, Clone, PartialEq)]
pub struct TxEncodeOptions {
    /// Tile edge in pixels (TIFF requires a multiple of 16; at most 1024 here).
    pub tile: u32,
    /// Deflate with a predictor; `false` stores the tiles uncompressed.
    pub deflate: bool,
    /// Wrap modes of the s (horizontal) and t (vertical) axis.
    pub wrap: (TxWrap, TxWrap),
    /// The `Software` tag (`None`: not written).
    pub software: Option<String>,
    /// The `DateTime` tag, `"YYYY:MM:DD HH:MM:SS"` (`None`: not written; the codec has no clock).
    pub date_time: Option<String>,
    /// Resolution in pixels per inch (`None`: not written).
    pub dpi: Option<(f64, f64)>,
    /// Hashed after the top level's samples into `ImageDescription`'s SHA-1: whatever else makes
    /// the texture differ (the options that shape the lower levels), as `maketx` adds its own.
    pub hash_extra: Option<String>,
}

impl Default for TxEncodeOptions {
    fn default() -> Self {
        TxEncodeOptions { tile: 64, deflate: true, wrap: (TxWrap::Black, TxWrap::Black), software: None, date_time: None, dpi: None, hash_extra: None }
    }
}

/// The size of MIP level `n` below `(w, h)`: each level halves the previous one, rounded down and
/// never below 1, as `maketx` does (1000×600 → 500×300 → … → 3×2 → 1×1).
pub fn tx_level_sizes(w: u32, h: u32) -> Vec<(u32, u32)> {
    let mut out = vec![(w.max(1), h.max(1))];
    let (mut w, mut h) = (w.max(1), h.max(1));
    while w > 1 || h > 1 {
        w = (w / 2).max(1);
        h = (h / 2).max(1);
        out.push((w, h));
    }
    out
}

const TIFF_ASCII: u16 = 2;
const TIFF_SHORT: u16 = 3;
const TIFF_LONG: u16 = 4;
const TIFF_RATIONAL: u16 = 5;
const TIFF_FLOAT: u16 = 11;

/// One directory entry before layout: tag, type, count, value bytes (little-endian).
struct Field {
    tag: u16,
    ty: u16,
    count: u32,
    bytes: Vec<u8>,
}

impl Field {
    fn shorts(tag: u16, v: &[u16]) -> Self {
        Field { tag, ty: TIFF_SHORT, count: v.len() as u32, bytes: v.iter().flat_map(|x| x.to_le_bytes()).collect() }
    }
    fn longs(tag: u16, v: &[u32]) -> Self {
        Field { tag, ty: TIFF_LONG, count: v.len() as u32, bytes: v.iter().flat_map(|x| x.to_le_bytes()).collect() }
    }
    fn ascii(tag: u16, s: &str) -> Self {
        // TIFF ASCII is 7-bit and NUL-terminated; anything else is replaced, never truncated mid-char.
        let mut bytes: Vec<u8> = s.chars().map(|c| if c.is_ascii() && c != '\0' { c as u8 } else { b'?' }).collect();
        bytes.push(0);
        Field { tag, ty: TIFF_ASCII, count: bytes.len() as u32, bytes }
    }
    fn rational(tag: u16, v: f64) -> Self {
        // Whole hundredths are enough for a resolution and keep the fraction exact for 72 or 300.
        let num = (v.clamp(0.0, 42_949_672.0) * 100.0).round() as u32;
        let mut bytes = num.to_le_bytes().to_vec();
        bytes.extend_from_slice(&100u32.to_le_bytes());
        Field { tag, ty: TIFF_RATIONAL, count: 1, bytes }
    }
    fn float(tag: u16, v: f32) -> Self {
        Field { tag, ty: TIFF_FLOAT, count: 1, bytes: v.to_le_bytes().to_vec() }
    }
}

fn err(e: impl std::fmt::Display) -> CodecError {
    CodecError::encode(Format::Tiff, e)
}

/// A u32 file offset, or an error when the file outgrows classic TIFF (4 GiB; `maketx` 1.4 writes
/// no BigTIFF, so neither do we).
fn offset(n: usize) -> Result<u32, CodecError> {
    u32::try_from(n).map_err(|_| err("the texture exceeds 4 GiB, the limit of a classic TIFF"))
}

/// Whether a file of `len` bytes (`None`: the sum overflowed) stays within a classic TIFF.
fn fits(len: Option<usize>) -> Result<(), CodecError> {
    offset(len.ok_or_else(|| err("the texture exceeds 4 GiB, the limit of a classic TIFF"))?).map(|_| ())
}

/// Tiles encoded together (in parallel) before they are appended: bounds what is held at once.
const TILE_BATCH: usize = 16;

/// Bytes hashed between two cancellation checks (a multiple of 4, so no sample is split).
const HASH_PIECE: usize = 64 << 20;

/// Writes `levels` (full size first, as [`tx_level_sizes`] lists them) as a `.tx`.
///
/// Every level must share one layout (gray or RGB, with or without alpha) and sample type, and an
/// alpha layout must already be premultiplied. Fewer levels than the full chain is allowed (one
/// level = `maketx --nomipmap`), as long as each one halves the previous.
pub fn encode_tx(levels: &[Image], opts: &TxEncodeOptions) -> Result<Vec<u8>, CodecError> {
    encode_tx_with(levels, opts, &|| false)
}

/// [`encode_tx`] that asks `cancelled` before each row of tiles and stops with an error once it
/// says yes (the caller tells that error apart by asking again).
pub fn encode_tx_with(levels: &[Image], opts: &TxEncodeOptions, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<Vec<u8>, CodecError> {
    let first = levels.first().ok_or_else(|| err("a texture needs at least one level"))?;
    let (layout, sample) = (first.layout(), first.sample_type());
    let photometric: u16 = match layout {
        ChannelLayout::Gray | ChannelLayout::GrayA => 1,
        ChannelLayout::Rgb | ChannelLayout::Rgba => 2,
        other => return Err(CodecError::unsupported(Format::Tiff, format!("a .tx texture is gray or RGB, not {other:?}"))),
    };
    if opts.tile == 0 || !opts.tile.is_multiple_of(16) || opts.tile > 1024 {
        return Err(err(format!("tile size {} is not a multiple of 16 between 16 and 1024", opts.tile)));
    }
    let sizes = tx_level_sizes(first.width(), first.height());
    if levels.len() > sizes.len() {
        return Err(err(format!("{} levels for a {}×{} texture, at most {}", levels.len(), first.width(), first.height(), sizes.len())));
    }
    for (i, (img, want)) in levels.iter().zip(&sizes).enumerate() {
        if img.dimensions() != *want || img.layout() != layout || img.sample_type() != sample {
            return Err(err(format!(
                "level {i} is {}×{} {:?} {:?}, expected {}×{} {layout:?} {sample:?}",
                img.width(),
                img.height(),
                img.layout(),
                img.sample_type(),
                want.0,
                want.1
            )));
        }
    }
    let spp = layout.channels();
    let bps = sample.bytes();
    let (sample_format, predictor): (u16, u16) = match sample {
        SampleType::U8 | SampleType::U16 => (1, 2),
        SampleType::F16 | SampleType::F32 => (3, 3),
    };
    // The fingerprint covers every level (its size, layout and type, then its samples) and the
    // caller's extra: the levels below the top are not derived from the stored top level alone,
    // so its samples could match while theirs don't.
    let mut hasher = Sha1::new();
    for img in levels {
        hasher.update(format!("{}x{} {:?} {:?};", img.width(), img.height(), layout, sample).as_bytes());
        // In bounded pieces, asking `cancelled` between them: hashing a large texture takes seconds.
        for piece in img.data().chunks(HASH_PIECE) {
            if cancelled() {
                return Err(err("cancelled"));
            }
            hasher.update(&le_samples(piece, sample));
        }
    }
    hasher.update(opts.hash_extra.as_deref().unwrap_or("").as_bytes());
    let description = format!("SHA-1={}", hex_upper(&hasher.finish()));
    let wraps = format!("{},{}", opts.wrap.0.name(), opts.wrap.1.name());
    let fovcot = first.width() as f32 / first.height().max(1) as f32;

    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"II");
    out.extend_from_slice(&42u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // first IFD offset, patched below
    // Where the previous directory's "next IFD" pointer sits (the header's for the first level).
    let mut next_ptr = 4usize;
    for img in levels {
        // A row of tiles at a time (in parallel within the row), appended as soon as it is
        // encoded: the level's compressed tiles are never held twice.
        let t = opts.tile as usize;
        let (across, down) = ((img.width() as usize).div_ceil(t), (img.height() as usize).div_ceil(t));
        let mut tile_offsets = Vec::new();
        let mut tile_counts = Vec::new();
        let count = across.checked_mul(down).ok_or_else(|| err("tile count overflows"))?;
        tile_offsets.try_reserve_exact(count).map_err(|_| err("not enough memory for the texture"))?;
        tile_counts.try_reserve_exact(count).map_err(|_| err("not enough memory for the texture"))?;
        for ty in 0..down {
            for tx0 in (0..across).step_by(TILE_BATCH) {
                if cancelled() {
                    return Err(err("cancelled"));
                }
                for tile in encode_tile_batch(img, ty, tx0..(tx0 + TILE_BATCH).min(across), t, spp, bps, sample, opts.deflate)? {
                    // Never grow past what a classic TIFF can address: checked before growing.
                    let pad = out.len() % 2;
                    fits(out.len().checked_add(pad).and_then(|n| n.checked_add(tile.len())))?;
                    out.try_reserve(pad + tile.len()).map_err(|_| err("not enough memory for the texture"))?;
                    if pad == 1 {
                        out.push(0);
                    }
                    tile_offsets.push(offset(out.len())?);
                    tile_counts.push(offset(tile.len())?);
                    out.extend_from_slice(&tile);
                }
            }
        }
        let bits = vec![(bps * 8) as u16; spp];
        let mut fields = vec![
            Field::longs(256, &[img.width()]),
            Field::longs(257, &[img.height()]),
            Field::shorts(258, &bits),
            Field::shorts(259, &[if opts.deflate { 8 } else { 1 }]),
            Field::shorts(262, &[photometric]),
            Field::ascii(270, &description),
            Field::shorts(274, &[1]),
            Field::shorts(277, &[spp as u16]),
            Field::shorts(284, &[1]),
            Field::shorts(322, &[opts.tile as u16]),
            Field::shorts(323, &[opts.tile as u16]),
            Field::longs(324, &tile_offsets),
            Field::longs(325, &tile_counts),
            Field::shorts(339, &vec![sample_format; spp]),
            Field::ascii(33302, "Plain Texture"),
            Field::ascii(33303, &wraps),
            Field::float(33304, fovcot),
        ];
        if opts.deflate {
            fields.push(Field::shorts(317, &[predictor]));
        }
        if layout.has_alpha() {
            fields.push(Field::shorts(338, &[1]));
        }
        if let Some(s) = &opts.software {
            fields.push(Field::ascii(305, s));
        }
        if let Some(s) = &opts.date_time {
            fields.push(Field::ascii(306, s));
        }
        if let Some((x, y)) = opts.dpi.filter(|(x, y)| x.is_finite() && y.is_finite() && *x > 0.0 && *y > 0.0) {
            fields.push(Field::rational(282, x));
            fields.push(Field::rational(283, y));
            fields.push(Field::shorts(296, &[2]));
        }
        fields.sort_by_key(|f| f.tag);
        let pad = out.len() % 2;
        let ifd = out.len().checked_add(pad).ok_or_else(|| err("the texture exceeds 4 GiB, the limit of a classic TIFF"))?;
        let ifd_len = 2 + 12 * fields.len() + 4;
        // Values longer than four bytes follow the directory, each at a word boundary.
        let mut extra = ifd + ifd_len;
        let mut table = Vec::with_capacity(ifd_len);
        let mut tail: Vec<u8> = Vec::new();
        table.extend_from_slice(&(fields.len() as u16).to_le_bytes());
        for f in &fields {
            table.extend_from_slice(&f.tag.to_le_bytes());
            table.extend_from_slice(&f.ty.to_le_bytes());
            table.extend_from_slice(&f.count.to_le_bytes());
            if f.bytes.len() <= 4 {
                let mut v = f.bytes.clone();
                v.resize(4, 0);
                table.extend_from_slice(&v);
            } else {
                table.extend_from_slice(&offset(extra)?.to_le_bytes());
                tail.extend_from_slice(&f.bytes);
                if f.bytes.len() % 2 == 1 {
                    tail.push(0);
                }
                extra = ifd + ifd_len + tail.len();
            }
        }
        let next_at = ifd + 2 + 12 * fields.len();
        table.extend_from_slice(&0u32.to_le_bytes());
        fits(ifd.checked_add(table.len()).and_then(|n| n.checked_add(tail.len())))?;
        out.try_reserve(pad + table.len() + tail.len()).map_err(|_| err("not enough memory for the texture"))?;
        if pad == 1 {
            out.push(0);
        }
        out.extend_from_slice(&table);
        out.extend_from_slice(&tail);
        let at = offset(ifd)?.to_le_bytes();
        out.get_mut(next_ptr..next_ptr + 4).ok_or_else(|| err("directory chain"))?.copy_from_slice(&at);
        next_ptr = next_at;
    }
    offset(out.len())?;
    Ok(out)
}

/// The samples of `data` (native-endian) as little-endian bytes, as the file stores them.
fn le_samples(data: &[u8], sample: SampleType) -> std::borrow::Cow<'_, [u8]> {
    if cfg!(target_endian = "little") || sample.bytes() == 1 {
        return std::borrow::Cow::Borrowed(data);
    }
    let mut v = data.to_vec();
    for s in v.chunks_exact_mut(sample.bytes()) {
        s.reverse();
    }
    std::borrow::Cow::Owned(v)
}

/// The tiles of one level, row by row, each predicted and compressed on its own.
/// Tiles `txs` of tile row `ty`, in order.
#[allow(clippy::too_many_arguments)]
fn encode_tile_batch(
    img: &Image,
    ty: usize,
    txs: std::ops::Range<usize>,
    t: usize,
    spp: usize,
    bps: usize,
    sample: SampleType,
    deflate: bool,
) -> Result<Vec<Vec<u8>>, CodecError> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let one = |tx: usize| encode_tile(img.data(), (w, h), (tx, ty), t, spp, bps, sample, deflate);
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        txs.into_par_iter().map(one).collect()
    }
    #[cfg(target_arch = "wasm32")]
    {
        txs.map(one).collect()
    }
}

/// A zeroed buffer of `n` bytes, or an error (not an abort) when memory runs out.
fn zeroed(n: usize) -> Result<Vec<u8>, CodecError> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| err("not enough memory for the texture"))?;
    v.resize(n, 0);
    Ok(v)
}

/// One tile: its pixels (zero beyond the image), little-endian, predicted, then deflated.
#[allow(clippy::too_many_arguments)]
fn encode_tile(
    data: &[u8],
    (w, h): (usize, usize),
    (tx, ty): (usize, usize),
    t: usize,
    spp: usize,
    bps: usize,
    sample: SampleType,
    deflate: bool,
) -> Result<Vec<u8>, CodecError> {
    let px = spp * bps;
    let row_bytes = t * px;
    let mut buf = zeroed(row_bytes.checked_mul(t).ok_or_else(|| err("tile size overflows"))?)?;
    let (x0, y0) = (tx * t, ty * t);
    let cols = w.saturating_sub(x0).min(t);
    for r in 0..t.min(h.saturating_sub(y0)) {
        let src = (y0 + r) * w * px + x0 * px;
        let s = data.get(src..src + cols * px).ok_or_else(|| err("tile outside the image"))?;
        let d = buf.get_mut(r * row_bytes..r * row_bytes + cols * px).ok_or_else(|| err("tile row"))?;
        d.copy_from_slice(s);
    }
    if !deflate {
        if cfg!(target_endian = "big") && bps > 1 {
            for s in buf.chunks_exact_mut(bps) {
                s.reverse();
            }
        }
        return Ok(buf);
    }
    let mut scratch = zeroed(row_bytes)?;
    for row in buf.chunks_exact_mut(row_bytes) {
        match sample {
            SampleType::U8 => predict_u8(row, spp),
            SampleType::U16 => predict_u16(row, spp),
            SampleType::F16 | SampleType::F32 => predict_float(row, &mut scratch, spp, bps),
        }
    }
    // Room for the worst case up front (deflate's stored blocks: 5 bytes per 64 KiB, plus the zlib
    // header and checksum), so the encoder never grows the vector infallibly.
    let mut dst = Vec::new();
    dst.try_reserve_exact(buf.len() + buf.len() / 65535 * 5 + 64).map_err(|_| err("not enough memory for the texture"))?;
    let mut enc = flate2::write::ZlibEncoder::new(dst, flate2::Compression::default());
    enc.write_all(&buf).map_err(err)?;
    enc.finish().map_err(err)
}

/// Horizontal differencing (predictor 2) of one row of 8-bit samples, `spp` per pixel.
fn predict_u8(row: &mut [u8], spp: usize) {
    for i in (spp..row.len()).rev() {
        row[i] = row[i].wrapping_sub(row[i - spp]);
    }
}

/// Predictor 2 for 16-bit samples: native-endian values in, differences out little-endian, in place
/// (from the end, so each sample is differenced against its still-unchanged left neighbour).
fn predict_u16(row: &mut [u8], spp: usize) {
    let s = row.as_chunks_mut::<2>().0;
    for i in (0..s.len()).rev() {
        let v = u16::from_ne_bytes(s[i]);
        let d = if i >= spp { v.wrapping_sub(u16::from_ne_bytes(s[i - spp])) } else { v };
        s[i] = d.to_le_bytes();
    }
}

/// The floating-point predictor (3, Adobe TIFF Technical Note 3): the bytes of a row are
/// regrouped by significance (all most significant bytes first, as big-endian), then
/// differenced byte by byte with a stride of one pixel.
fn predict_float(row: &mut [u8], scratch: &mut [u8], spp: usize, bps: usize) {
    let wc = row.len() / bps;
    for (i, s) in row.chunks_exact(bps).enumerate() {
        for k in 0..bps {
            // Byte k of the big-endian value: native little-endian stores it at bps-1-k.
            let b = if cfg!(target_endian = "little") { s[bps - 1 - k] } else { s[k] };
            scratch[k * wc + i] = b;
        }
    }
    row.copy_from_slice(scratch);
    for i in (spp..row.len()).rev() {
        row[i] = row[i].wrapping_sub(row[i - spp]);
    }
}

fn hex_upper(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}

/// Streaming SHA-1 (FIPS 180-4). Only an identity for the texture (what `maketx -u` compares),
/// not a security feature.
struct Sha1 {
    h: [u32; 5],
    buf: [u8; 64],
    fill: usize,
    len: u64,
}

impl Sha1 {
    fn new() -> Self {
        Sha1 { h: [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0], buf: [0; 64], fill: 0, len: 0 }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.len = self.len.wrapping_add(data.len() as u64);
        if self.fill > 0 {
            let take = (64 - self.fill).min(data.len());
            let (head, rest) = data.split_at(take);
            if let Some(dst) = self.buf.get_mut(self.fill..self.fill + take) {
                dst.copy_from_slice(head);
            }
            self.fill += take;
            data = rest;
            if self.fill < 64 {
                return;
            }
            let block = self.buf;
            compress(&mut self.h, &block);
            self.fill = 0;
        }
        let (blocks, rest) = data.as_chunks::<64>();
        for block in blocks {
            compress(&mut self.h, block);
        }
        if let Some(dst) = self.buf.get_mut(..rest.len()) {
            dst.copy_from_slice(rest);
        }
        self.fill = rest.len();
    }

    fn finish(mut self) -> [u8; 20] {
        let bits = self.len.wrapping_mul(8);
        self.update(&[0x80]);
        while self.fill != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_be_bytes());
        let mut out = [0u8; 20];
        for (o, x) in out.as_chunks_mut::<4>().0.iter_mut().zip(self.h) {
            *o = x.to_be_bytes();
        }
        out
    }
}

/// One 64-byte block into the state.
fn compress(h: &mut [u32; 5], block: &[u8; 64]) {
    let mut w = [0u32; 80];
    for (i, c) in block.as_chunks::<4>().0.iter().enumerate() {
        w[i] = u32::from_be_bytes(*c);
    }
    for i in 16..80 {
        w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
    }
    let [mut a, mut b, mut c, mut d, mut e] = *h;
    for (i, wi) in w.iter().enumerate() {
        let (f, k) = match i {
            0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
            20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
            40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
            _ => (b ^ c ^ d, 0xCA62_C1D6),
        };
        let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
        e = d;
        d = c;
        c = b.rotate_left(30);
        b = a;
        a = t;
    }
    for (x, v) in h.iter_mut().zip([a, b, c, d, e]) {
        *x = x.wrapping_add(v);
    }
}

/// SHA-1 of `data` followed by `extra` (tests).
#[cfg(test)]
fn sha1(data: &[u8], extra: &[u8]) -> [u8; 20] {
    let mut s = Sha1::new();
    s.update(data);
    s.update(extra);
    s.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32, layout: ChannelLayout, sample: SampleType) -> Image {
        let mut img = Image::new(w, h, layout, sample).unwrap();
        let n = layout.channels();
        let bps = sample.bytes();
        for (i, px) in img.data_mut().chunks_exact_mut(n * bps).enumerate() {
            let (x, y) = ((i as u32 % w) as f32 / w as f32, (i as u32 / w) as f32 / h as f32);
            for (c, s) in px.chunks_exact_mut(bps).enumerate() {
                let v = match c % 3 {
                    0 => x,
                    1 => y,
                    _ => (x * 7.0).fract(),
                };
                match sample {
                    SampleType::U8 => s[0] = (v * 255.0) as u8,
                    SampleType::U16 => s.copy_from_slice(&((v * 65535.0) as u16).to_ne_bytes()),
                    SampleType::F16 => s.copy_from_slice(&half::f16::from_f32(v * 4.0).to_bits().to_ne_bytes()),
                    SampleType::F32 => s.copy_from_slice(&(v * 4.0).to_ne_bytes()),
                }
            }
        }
        img
    }

    /// A box-filtered chain, enough to exercise the writer (the real chain is built in `io`).
    fn chain(top: Image) -> Vec<Image> {
        let sizes = tx_level_sizes(top.width(), top.height());
        let mut out = vec![top];
        for &(w, h) in sizes.iter().skip(1) {
            let prev = out.last().unwrap();
            let mut img = Image::new(w, h, prev.layout(), prev.sample_type()).unwrap();
            let px = prev.layout().channels() * prev.sample_type().bytes();
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let s = ((2 * y).min(prev.height() as usize - 1) * prev.width() as usize + (2 * x).min(prev.width() as usize - 1)) * px;
                    let d = (y * w as usize + x) * px;
                    let v = prev.data()[s..s + px].to_vec();
                    img.data_mut()[d..d + px].copy_from_slice(&v);
                }
            }
            out.push(img);
        }
        out
    }

    #[test]
    fn sha1_vectors() {
        assert_eq!(hex_upper(&sha1(b"", b"")), "DA39A3EE5E6B4B0D3255BFEF95601890AFD80709");
        assert_eq!(hex_upper(&sha1(b"abc", b"")), "A9993E364706816ABA3E25717850C26C9CD0D89D");
        assert_eq!(hex_upper(&sha1(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq", b"")), "84983E441C3BD26EBAAE4AA1F95129E5E54670F1");
        // 55, 56 and 64 bytes: the padding boundary cases.
        assert_eq!(hex_upper(&sha1(&[b'a'; 55], b"")), "C1C8BBDC22796E28C0E15163D20899B65621D65A");
        assert_eq!(hex_upper(&sha1(&[b'a'; 56], b"")), "C2DB330F6083854C99D4B5BFB6E8F29F201BE699");
        assert_eq!(hex_upper(&sha1(&[b'a'; 64], b"")), "0098BA824B5C16427BD7A1122A5A442A25EC644D");
        // The suffix hashes as if appended: across the block boundary and longer than a block.
        let data: Vec<u8> = (0..300u32).map(|i| (i * 7) as u8).collect();
        for (cut, extra) in [(0usize, 0usize), (63, 1), (64, 70), (127, 200), (300, 0), (250, 50)] {
            let (d, e) = data.split_at(cut);
            let e = &e[..extra.min(e.len())];
            let whole: Vec<u8> = d.iter().chain(e).copied().collect();
            assert_eq!(sha1(d, e), sha1(&whole, b""), "cut {cut} extra {extra}");
        }
    }

    #[test]
    fn level_sizes_round_down_to_one() {
        assert_eq!(tx_level_sizes(1000, 600), vec![(1000, 600), (500, 300), (250, 150), (125, 75), (62, 37), (31, 18), (15, 9), (7, 4), (3, 2), (1, 1)]);
        assert_eq!(tx_level_sizes(512, 512).len(), 10);
        assert_eq!(tx_level_sizes(1, 1), vec![(1, 1)]);
        assert_eq!(tx_level_sizes(8, 1), vec![(8, 1), (4, 1), (2, 1), (1, 1)]);
    }

    /// `bytes` with every `ExtraSamples = 1` (associated) turned into 2 (unassociated), so the
    /// decoder hands back the stored, premultiplied samples instead of un-premultiplying them.
    pub(crate) fn as_unassociated(bytes: &[u8]) -> Vec<u8> {
        let mut out = bytes.to_vec();
        let f = crate::codecs::tiff_ifd::File::parse(bytes).unwrap();
        for ifd in chain_ifds(&f) {
            if let Some(e) = ifd.get(338) {
                out[e.value_at as usize] = 2;
            }
        }
        out
    }

    /// Every layout and depth reads back through the TIFF decoder bit for bit, level by level.
    #[test]
    fn round_trip_every_level() {
        for layout in [ChannelLayout::Gray, ChannelLayout::GrayA, ChannelLayout::Rgb, ChannelLayout::Rgba] {
            for sample in SampleType::ALL {
                for deflate in [true, false] {
                    let levels = chain(gradient(150, 70, layout, sample));
                    let bytes = as_unassociated(&encode_tx(&levels, &TxEncodeOptions { deflate, ..Default::default() }).unwrap());
                    let info = crate::tiff_info(&bytes).unwrap();
                    assert_eq!(info.pages.len(), levels.len(), "{layout:?} {sample:?}");
                    for (i, want) in levels.iter().enumerate() {
                        let got = crate::decode_tiff_page(&bytes, i, &crate::DecodeOptions::default()).unwrap();
                        assert_eq!(got.dimensions(), want.dimensions(), "{layout:?} {sample:?} level {i}");
                        assert_eq!(got.layout(), layout, "{layout:?} {sample:?} level {i}");
                        assert_eq!(got.sample_type(), sample, "{layout:?} {sample:?} level {i}");
                        assert!(got.data() == want.data(), "{layout:?} {sample:?} deflate {deflate} level {i}");
                    }
                }
            }
        }
    }

    /// The directories of the main chain.
    fn chain_ifds(f: &crate::codecs::tiff_ifd::File<'_>) -> Vec<crate::codecs::tiff_ifd::Ifd> {
        let mut out = Vec::new();
        let mut at = f.first_ifd;
        while at != 0 {
            let ifd = f.ifd(at).unwrap();
            at = ifd.next;
            out.push(ifd);
        }
        out
    }

    fn ascii(f: &crate::codecs::tiff_ifd::File<'_>, ifd: &crate::codecs::tiff_ifd::Ifd, tag: u16) -> Option<String> {
        f.tag_bytes(ifd, tag).map(|b| String::from_utf8_lossy(b).trim_end_matches('\0').to_string())
    }

    #[test]
    fn tags_as_maketx_writes_them() {
        let levels = chain(gradient(100, 60, ChannelLayout::Rgba, SampleType::U8));
        let opts = TxEncodeOptions { wrap: (TxWrap::Periodic, TxWrap::Clamp), software: Some("PhotoCraft".into()), ..Default::default() };
        let bytes = encode_tx(&levels, &opts).unwrap();
        let f = crate::codecs::tiff_ifd::File::parse(&bytes).unwrap();
        let ifds = chain_ifds(&f);
        assert_eq!(ifds.len(), 7);
        for ifd in &ifds {
            assert!(ifd.get(254).is_none(), "no NewSubfileType");
            assert_eq!(f.tag_uints(ifd, 322, 1).unwrap(), vec![64]);
            assert_eq!(f.tag_uints(ifd, 259, 1).unwrap(), vec![8]);
            assert_eq!(f.tag_uints(ifd, 317, 1).unwrap(), vec![2]);
            assert_eq!(f.tag_uints(ifd, 338, 1).unwrap(), vec![1], "associated alpha");
            assert_eq!(ascii(&f, ifd, 33302).as_deref(), Some("Plain Texture"));
            assert_eq!(ascii(&f, ifd, 33303).as_deref(), Some("periodic,clamp"));
            assert_eq!(ascii(&f, ifd, 305).as_deref(), Some("PhotoCraft"));
            assert!(ascii(&f, ifd, 270).is_some_and(|d| d.starts_with("SHA-1=") && d.len() == 46));
            assert!((f.tag_f64(ifd, 33304).unwrap() - 100.0 / 60.0).abs() < 1e-6, "PixarFOVCOT = width / height");
        }
        let float = encode_tx(&chain(gradient(40, 40, ChannelLayout::Rgb, SampleType::F32)), &TxEncodeOptions::default()).unwrap();
        let f = crate::codecs::tiff_ifd::File::parse(&float).unwrap();
        let ifd = chain_ifds(&f).remove(0);
        assert_eq!(f.tag_uints(&ifd, 317, 1).unwrap(), vec![3]);
        assert_eq!(f.tag_uints(&ifd, 339, 3).unwrap(), vec![3, 3, 3]);
    }

    /// Same top level, different lower level: a different fingerprint (an OpenImageIO cache
    /// deduplicates by it).
    #[test]
    fn fingerprint_covers_every_level() {
        let desc = |levels: &[Image]| {
            let bytes = encode_tx(levels, &TxEncodeOptions::default()).unwrap();
            let f = crate::codecs::tiff_ifd::File::parse(&bytes).unwrap();
            let ifd = chain_ifds(&f).remove(0);
            ascii(&f, &ifd, 270).unwrap()
        };
        let top = Image::from_raw(2, 1, ChannelLayout::Gray, SampleType::U8, vec![0, 1]).unwrap();
        let a = [top.clone(), Image::from_raw(1, 1, ChannelLayout::Gray, SampleType::U8, vec![1]).unwrap()];
        let b = [top, Image::from_raw(1, 1, ChannelLayout::Gray, SampleType::U8, vec![0]).unwrap()];
        assert_ne!(desc(&a), desc(&b));
        assert_eq!(desc(&a), desc(&a.clone()));
    }

    /// More tiles in a row than one batch: the batches append in order.
    #[test]
    fn rows_wider_than_one_batch() {
        let levels = as_unassociated(
            &encode_tx(
                &chain(gradient(16 * (TILE_BATCH as u32 + 3) + 5, 40, ChannelLayout::Rgba, SampleType::U16)),
                &TxEncodeOptions { tile: 16, ..Default::default() },
            )
            .unwrap(),
        );
        let want = chain(gradient(16 * (TILE_BATCH as u32 + 3) + 5, 40, ChannelLayout::Rgba, SampleType::U16));
        for (i, w) in want.iter().enumerate() {
            let got = crate::decode_tiff_page(&levels, i, &crate::DecodeOptions::default()).unwrap();
            assert!(got.data() == w.data(), "level {i}");
        }
    }

    #[test]
    fn rejects_what_is_not_a_texture() {
        let rgb = gradient(16, 16, ChannelLayout::Rgb, SampleType::U8);
        assert!(encode_tx(&[], &TxEncodeOptions::default()).is_err());
        assert!(encode_tx(&[gradient(8, 8, ChannelLayout::Cmyk, SampleType::U8)], &TxEncodeOptions::default()).is_err());
        assert!(encode_tx(std::slice::from_ref(&rgb), &TxEncodeOptions { tile: 60, ..Default::default() }).is_err());
        assert!(encode_tx(std::slice::from_ref(&rgb), &TxEncodeOptions { tile: 0, ..Default::default() }).is_err());
        // A second level of the wrong size, depth or layout.
        assert!(encode_tx(&[rgb.clone(), gradient(9, 8, ChannelLayout::Rgb, SampleType::U8)], &TxEncodeOptions::default()).is_err());
        assert!(encode_tx(&[rgb.clone(), gradient(8, 8, ChannelLayout::Rgb, SampleType::U16)], &TxEncodeOptions::default()).is_err());
        assert!(encode_tx(&[rgb.clone(), gradient(8, 8, ChannelLayout::Rgba, SampleType::U8)], &TxEncodeOptions::default()).is_err());
        // One level is `--nomipmap`.
        assert!(encode_tx(&[rgb], &TxEncodeOptions::default()).is_ok());
        // A cancelled encode stops instead of finishing.
        let big = chain(gradient(200, 130, ChannelLayout::Rgb, SampleType::U8));
        assert!(encode_tx_with(&big, &TxEncodeOptions::default(), &|| true).is_err());
        assert!(encode_tx_with(&big, &TxEncodeOptions::default(), &|| false).is_ok());
    }
}
