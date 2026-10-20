//! Pixel layers (`Rstr`) and placed images (`ImgN`). A placed image keeps its original file
//! (`DyBm.Bckg` → archive entry `c/N`), which is imported as is; other pixels are planar tiles of
//! 256 bytes × 256 rows per channel, each its own archive entry.

use std::collections::HashMap;

use crate::Error;
use crate::model::{Affine, Image, Pixels, Reader};
use crate::stream::{self, ObjId, Tag, Value};

/// Pixels decoded per layer (the content's bounding box, not the canvas).
const MAX_PIXELS: u64 = 64 << 20;
const TILE: usize = 256;
const TILE_BYTES: usize = TILE * TILE;

pub(crate) fn node_image(r: &mut Reader, id: ObjId, world: Affine) -> Result<Option<Image>, Error> {
    let s = r.s;
    let Some(bitmap) = s.obj(id, b"Bitm") else {
        r.warn("pixel layers without pixels");
        return Ok(None);
    };
    if !s.is(bitmap, b"DyBm") {
        r.warn("pixel layers stored in an unknown way");
        return Ok(None);
    }
    let width = s.int(bitmap, b"BmpW").and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
    let height = s.int(bitmap, b"BmpH").and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
    if width == 0 || height == 0 {
        return Ok(None);
    }
    // A placed image: the original file, byte for byte.
    let has_tiles = s.field(bitmap, b"Sta1").is_some();
    if let Some(name) = s.entry(bitmap, b"Bckg").filter(|n| !n.is_empty())
        && (s.is(id, b"ImgN") || !has_tiles)
    {
        match original(r, name) {
            Ok(bytes) => return Ok(Some(Image { width, height, pixels: Pixels::Encoded(bytes), transform: world })),
            Err(Error::Limit(e)) => return Err(Error::Limit(e)),
            Err(_) => r.warn("an embedded image file that could not be read (its cached pixels were used)"),
        }
    }
    // The content's bounding box, so a mostly empty canvas-sized layer stays small.
    let crop = s
        .field(id, b"BitR")
        .and_then(|v| if let Value::Ints(i) = v { Some(i.clone()) } else { None })
        .filter(|v| v.len() == 4 && v[0] > i64::from(i32::MIN) + 1)
        .map(|v| {
            (
                v[0].clamp(0, i64::from(width)) as u32,
                v[1].clamp(0, i64::from(height)) as u32,
                v[2].clamp(0, i64::from(width)) as u32,
                v[3].clamp(0, i64::from(height)) as u32,
            )
        })
        .unwrap_or((0, 0, width, height));
    if s.field(bitmap, b"Bckg").is_some() && has_tiles && !s.is(id, b"ImgN") {
        r.warn("edited placed images use their stored pixels");
    }
    decode(r, bitmap, crop, world)
}

/// A mask layer (`MRst`): one channel of coverage as grey pixels (white shows, black hides).
pub(crate) fn mask(r: &mut Reader, id: ObjId, world: Affine) -> Result<Option<Image>, Error> {
    let s = r.s;
    let Some(bitmap) = s.obj(id, b"Bitm") else { return Ok(None) };
    let width = s.int(bitmap, b"BmpW").and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
    let height = s.int(bitmap, b"BmpH").and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
    if width == 0 || height == 0 || !matches!(s.enumeration(bitmap, b"Frmt"), Some((6 | 7, _))) {
        r.warn("pixel masks in an unknown format (imported without them)");
        return Ok(None);
    }
    decode(r, bitmap, (0, 0, width, height), world)
}

/// An embedded document or symbol instance (`EmbN`), as the picture of it Affinity caches.
pub(crate) fn embedded(r: &mut Reader, id: ObjId, world: Affine) -> Result<Option<Image>, Error> {
    let s = r.s;
    let Some(cache) = s.obj(id, b"Bitm").and_then(|e| s.obj(e, b"Cach")) else { return Ok(None) };
    let Some(bbox) = s.obj(id, b"Bitm").and_then(|e| s.floats::<4>(e, b"ChBB")) else { return Ok(None) };
    let width = s.int(cache, b"BmpW").and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
    let height = s.int(cache, b"BmpH").and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
    if width == 0 || height == 0 {
        return Ok(None);
    }
    let [x0, y0, x1, y1] = bbox;
    // The instance's origin is the centre of the embedded content (checked against the
    // thumbnails of documents with embedded documents and SVG files).
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let place = Affine([(x1 - x0) / f64::from(width), 0.0, 0.0, (y1 - y0) / f64::from(height), x0 - cx, y0 - cy]).then(world);
    decode(r, cache, (0, 0, width, height), place)
}

/// Decode the tiles of a bitmap inside `crop` (pixels), placed by `world` (bitmap pixels to document).
fn decode(r: &mut Reader, bitmap: ObjId, crop: (u32, u32, u32, u32), world: Affine) -> Result<Option<Image>, Error> {
    let s = r.s;
    let (x0, y0, x1, y1) = crop;
    if x1 <= x0 || y1 <= y0 {
        return Ok(None);
    }
    let (w, h) = (x1 - x0, y1 - y0);
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        r.warn("pixel layers larger than 64 megapixels");
        return Ok(None);
    }
    let format = s.enumeration(bitmap, b"Frmt").map(|(f, _)| f);
    let (channels, bps) = match format {
        Some(0) => (4, 1),
        Some(1) => (4, 2),
        Some(6) => (1, 1),
        Some(7) => (1, 2),
        Some(4) => {
            r.warn("CMYK pixel layers (converted to RGB without the document's colour profile)");
            (5, 1)
        }
        _ => {
            r.warn("pixel layers in grey, Lab or 32-bit formats");
            return Ok(None);
        }
    };
    // A layer is materialized from its declared size, whatever the tiles hold: admit the output
    // and the planes it decodes through before either is allocated (#4960).
    let pixels = (w as usize).checked_mul(h as usize).ok_or(Error::Limit("pixel layer size"))?;
    let scratch = pixels.checked_mul(channels * bps).ok_or(Error::Limit("pixel layer size"))?;
    r.admit_pixels(pixels.checked_mul(4).ok_or(Error::Limit("pixel layer size"))?, scratch)?;
    let mut planes = Vec::with_capacity(channels);
    let mut cache = HashMap::new();
    for c in 1..=channels {
        planes.push(plane(r, bitmap, c, bps, (x0, y0, w, h), &mut cache)?);
    }
    drop(cache);
    let n = (w as usize) * (h as usize);
    let mut rgba = vec![0u8; n * 4];
    let sample = |p: &Vec<u8>, i: usize| -> f64 {
        if bps == 2 {
            f64::from(u16::from_le_bytes([p.get(2 * i).copied().unwrap_or(0), p.get(2 * i + 1).copied().unwrap_or(0)])) / 65535.0
        } else {
            f64::from(p.get(i).copied().unwrap_or(0)) / 255.0
        }
    };
    for (i, px) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let mut v = [0.0; 5];
        for (v, p) in v.iter_mut().zip(&planes) {
            *v = sample(p, i);
        }
        let (r2, g, b, a) = if channels == 1 {
            (v[0], v[0], v[0], 1.0)
        } else if channels == 5 {
            let k = 1.0 - v[3];
            ((1.0 - v[0]) * k, (1.0 - v[1]) * k, (1.0 - v[2]) * k, v[4])
        } else {
            (v[0], v[1], v[2], v[3])
        };
        px.copy_from_slice(&[r2, g, b, a].map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8));
    }
    let transform = Affine([1.0, 0.0, 0.0, 1.0, f64::from(x0), f64::from(y0)]).then(world);
    Ok(Some(Image { width: w, height: h, pixels: Pixels::Rgba8(rgba), transform }))
}

/// The original image file: entry `c/N` is a stream whose root (`Blck`) holds it as `Data`.
fn original(r: &mut Reader, name: &str) -> Result<Vec<u8>, Error> {
    let bytes = r.archive.read(name)?;
    let mut s = stream::parse(&bytes)?;
    let root = s.objects.get_mut(s.root).ok_or(Error::Malformed("embedded image"))?;
    match root.fields.iter_mut().find(|(t, _)| *t == Tag::of(b"Data")).map(|(_, v)| v) {
        Some(Value::Blob(b)) if !b.is_empty() => Ok(std::mem::take(b)),
        _ => Err(Error::Malformed("embedded image without data")),
    }
}

fn tag(prefix: &[u8; 3], c: usize) -> [u8; 4] {
    [prefix[0], prefix[1], prefix[2], b'0' + c as u8]
}

/// One channel of level 0, cropped to (x0, y0, w, h) pixels; `bps` bytes per sample.
fn plane(
    r: &mut Reader,
    bitmap: ObjId,
    c: usize,
    bps: usize,
    (x0, y0, w, h): (u32, u32, u32, u32),
    cache: &mut HashMap<String, Vec<u8>>,
) -> Result<Vec<u8>, Error> {
    let s = r.s;
    let width = s.int(bitmap, b"BmpW").and_then(|v| usize::try_from(v).ok()).unwrap_or(0);
    let height = s.int(bitmap, b"BmpH").and_then(|v| usize::try_from(v).ok()).unwrap_or(0);
    let tw = s.int(bitmap, &tag(b"TWi", c)).and_then(|v| usize::try_from(v).ok()).unwrap_or_else(|| width.div_ceil(TILE / bps));
    let th = s.int(bitmap, &tag(b"THi", c)).and_then(|v| usize::try_from(v).ok()).unwrap_or_else(|| height.div_ceil(TILE));
    let states = match s.field(bitmap, &tag(b"Sta", c)) {
        Some(Value::Array(a)) => a.as_slice(),
        _ => &[],
    };
    let tiles = match s.field(bitmap, &tag(b"Idx", c)) {
        Some(Value::Array(a)) => a.as_slice(),
        _ => &[],
    };
    let mut tiles = tiles.iter().filter_map(|v| if let Value::Obj(Some(o)) = v { Some(*o) } else { None });
    let row = (w as usize) * bps;
    let mut out = vec![0u8; row * h as usize];
    let (bx0, bx1) = (x0 as usize * bps, (x0 + w) as usize * bps);
    let (py0, py1) = (y0 as usize, (y0 + h) as usize);
    for (t, state) in states.iter().enumerate().take(tw.saturating_mul(th)) {
        let (ty, tx) = (t / tw.max(1), t % tw.max(1));
        let state = if let Value::UInt(u) = state { u8::try_from(*u).unwrap_or(255) } else { 255 };
        let fill = match state {
            0 | 1 => None,
            2 => Some(0xFF),
            4 => {
                let Some(blck) = tiles.next() else { return Err(Error::Malformed("missing pixel tile")) };
                let (ox, oy) = (tx * TILE, ty * TILE);
                if ox >= bx1 || ox + TILE <= bx0 || oy >= py1 || oy + TILE <= py0 {
                    continue;
                }
                let name = s.entry(blck, b"Data").ok_or(Error::Malformed("pixel tile without data"))?;
                if !cache.contains_key(name) {
                    cache.insert(name.to_owned(), r.archive.read(name)?);
                }
                let tile = tile_bytes(cache.get(name).ok_or(Error::Malformed("pixel tile"))?)?;
                copy(&mut out, row, (bx0, py0, bx1, py1), (ox, oy), Some(tile))?;
                continue;
            }
            3 => {
                r.warn("32-bit pixel layers");
                None
            }
            5 => {
                r.warn("pixel layers drawn from an embedded image (left empty)");
                None
            }
            _ => return Err(Error::Malformed("unknown pixel tile state")),
        };
        if fill.is_some() {
            copy(&mut out, row, (bx0, py0, bx1, py1), (tx * TILE, ty * TILE), None)?;
        }
    }
    Ok(out)
}

/// Borrow a raw tile or the blob inside an older file's one-field stream.
fn tile_bytes(data: &[u8]) -> Result<&[u8], Error> {
    let data = if data.len() == TILE_BYTES + 22 && data.starts_with(b"\x00\xffKS") {
        data.get(21..21 + TILE_BYTES).ok_or(Error::Malformed("pixel tile"))?
    } else {
        data
    };
    if data.len() != TILE_BYTES {
        return Err(Error::Malformed("pixel tile size"));
    }
    Ok(data)
}

/// Copy the overlap of a tile at byte column `ox`, row `oy` into the cropped plane.
/// Fill with white when no source pixels are supplied.
fn copy(out: &mut [u8], row: usize, (bx0, py0, bx1, py1): (usize, usize, usize, usize), (ox, oy): (usize, usize), tile: Option<&[u8]>) -> Result<(), Error> {
    let (left, right) = (ox.max(bx0), ox.saturating_add(TILE).min(bx1));
    let (top, bottom) = (oy.max(py0), oy.saturating_add(TILE).min(py1));
    if left >= right || top >= bottom {
        return Ok(());
    }
    for y in top..bottom {
        let start = (y - py0).checked_mul(row).and_then(|v| v.checked_add(left - bx0)).ok_or(Error::Malformed("pixel row"))?;
        let end = start.checked_add(right - left).ok_or(Error::Malformed("pixel row"))?;
        let dst = out.get_mut(start..end).ok_or(Error::Malformed("pixel row"))?;
        if let Some(tile) = tile {
            let start = (y - oy) * TILE + (left - ox);
            dst.copy_from_slice(tile.get(start..start + dst.len()).ok_or(Error::Malformed("pixel tile"))?);
        } else {
            dst.fill(0xFF);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::Limits;
    use crate::model::Kind;
    use crate::synth::{self, F, Method};

    /// A legacy one-field stream, with the tile or embedded image in its `Data` blob.
    fn blob(data: &[u8]) -> Vec<u8> {
        let mut bytes = b"\x00\xffKS\x01\x00kclB\x00\x00\x2dataD".to_vec();
        bytes.extend((data.len() as u32).to_le_bytes());
        bytes.extend(data);
        bytes.push(0);
        bytes
    }

    /// One image layer on one page, with optional archive entries for its pixels.
    fn document(class: &[u8; 4], bitmap: Vec<(Tag, F)>, entries: &[(&str, &[u8], Method)], crop: Option<[i32; 4]>) -> (Vec<u8>, Limits) {
        let mut fields = vec![(Tag::of(b"Bitm"), F::Def(4, vec![Tag::of(b"DyBm")], bitmap))];
        if let Some(crop) = crop {
            let mut field = vec![0x17];
            field.extend(Tag::of(b"BitR").0.to_le_bytes());
            field.extend(crop.into_iter().flat_map(i32::to_le_bytes));
            fields.push((Tag::of(b"BitR"), F::Raw(field)));
        }
        let layer = F::Def(3, vec![Tag::of(class)], fields);
        let page = F::Def(2, vec![Tag::of(b"Sprd")], vec![(Tag::of(b"Chld"), F::Shared(vec![layer]))]);
        let doc = synth::stream(&[(Tag::of(b"DocR"), F::Def(1, vec![Tag::of(b"DocN")], vec![(Tag::of(b"Chld"), F::Shared(vec![page]))]))]);
        let mut all = vec![("doc.dat", doc.as_slice(), Method::Zstd)];
        all.extend_from_slice(entries);
        // Allow one extraction per entry, even when several planes share a tile.
        let limits = Limits { max_total: doc.len() + entries.iter().map(|(_, bytes, _)| bytes.len()).sum::<usize>(), ..Limits::default() };
        (synth::container(&all, None), limits)
    }

    #[test]
    fn tile_views_and_clipped_rows_match_the_scalar_path() {
        let raw: Vec<u8> = (0..TILE_BYTES).map(|i| (i % 251) as u8).collect();
        let wrapped = blob(&raw);
        assert_eq!(tile_bytes(&raw).unwrap().as_ptr(), raw.as_ptr());
        assert_eq!(tile_bytes(&wrapped).unwrap().as_ptr(), wrapped[21..].as_ptr());
        for bad in [&raw[..TILE_BYTES - 1], &wrapped[..wrapped.len() - 1], &[0; TILE_BYTES + 22]] {
            assert!(tile_bytes(bad).is_err());
        }
        for crop in [(0, 0, 520, 270), (253, 254, 263, 260), (260, 260, 265, 265)] {
            let (bx0, py0, bx1, py1) = crop;
            let row = bx1 - bx0;
            for origin in [(0, 0), (256, 0), (0, 256), (256, 256), (512, 256)] {
                for tile in [Some(tile_bytes(&wrapped).unwrap()), None] {
                    let mut expected = vec![17; row * (py1 - py0)];
                    let mut actual = expected.clone();
                    let (ox, oy) = origin;
                    for y in oy.max(py0)..(oy + TILE).min(py1) {
                        for x in ox.max(bx0)..(ox + TILE).min(bx1) {
                            expected[(y - py0) * row + x - bx0] = tile.map_or(255, |t| t[(y - oy) * TILE + x - ox]);
                        }
                    }
                    copy(&mut actual, row, crop, origin, tile).unwrap();
                    assert_eq!(actual, expected, "{crop:?}, {origin:?}");
                }
            }
        }
        assert!(copy(&mut [], 1, (0, 0, 1, 1), (0, 0), None).is_err());
        assert!(copy(&mut [0], 1, (0, 0, 1, 1), (0, 0), Some(&[])).is_err());
        copy(&mut [], 0, (0, 0, 0, 0), (usize::MAX, usize::MAX), None).unwrap();
    }

    #[test]
    fn cached_tiles_preserve_all_supported_pixel_formats() {
        let raw: Vec<u8> = (0..TILE_BYTES).map(|i| [0, 1, 127, 128, 254, 255][i % 6]).collect();
        let other: Vec<u8> = raw.iter().map(|b| b ^ 0x55).collect();
        let wrapped = blob(&other);
        for (format, channels, bps) in [(0, 4, 1), (1, 4, 2), (4, 5, 1), (6, 1, 1), (7, 1, 2)] {
            let w = TILE / bps + 1;
            let mut fields = vec![(Tag::of(b"BmpW"), F::U32(w as u32)), (Tag::of(b"BmpH"), F::U32(2)), (Tag::of(b"Frmt"), F::Enum(format, 0))];
            for c in 1..=channels {
                fields.push((Tag::of(&tag(b"Sta", c)), F::U8s(vec![4, 2])));
                fields.push((Tag::of(&tag(b"Idx", c)), F::Objs(Tag::of(b"Blck"), vec![vec![(Tag::of(b"Data"), F::Entry(format!("d/{}", c % 2)))]])));
            }
            let (file, limits) = document(b"Rstr", fields, &[("d/1", &raw, Method::Zstd), ("d/0", &wrapped, Method::Zstd)], None);
            let doc = crate::model::read(&file, limits).unwrap();
            let Kind::Image(image) = &doc.spreads[0].nodes[0].kind else { panic!() };
            assert_eq!((image.width, image.height), (w as u32, 2));
            let mut expected = Vec::new();
            for y in 0..2 {
                for x in 0..w {
                    // The previous scalar conversion, including little-endian 16-bit samples.
                    let v: Vec<f64> = (1..=channels)
                        .map(|c| {
                            if x * bps >= TILE {
                                return 1.0;
                            }
                            let data = if c % 2 == 1 { &raw } else { &other };
                            let i = y * TILE + x * bps;
                            if bps == 2 { f64::from(u16::from_le_bytes([data[i], data[i + 1]])) / 65535.0 } else { f64::from(data[i]) / 255.0 }
                        })
                        .collect();
                    let rgba = if channels == 1 {
                        [v[0], v[0], v[0], 1.0]
                    } else if channels == 5 {
                        let k = 1.0 - v[3];
                        [(1.0 - v[0]) * k, (1.0 - v[1]) * k, (1.0 - v[2]) * k, v[4]]
                    } else {
                        [v[0], v[1], v[2], v[3]]
                    };
                    expected.extend(rgba.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8));
                }
            }
            assert_eq!(image.pixels, Pixels::Rgba8(expected), "format {format}");
        }
    }

    #[test]
    fn embedded_images_keep_the_first_data_field_and_fall_back_on_invalid_blobs() {
        let image = b"original image bytes";
        let mut valid = blob(image);
        valid.pop();
        valid.extend_from_slice(&blob(b"ignored")[12..]);
        let mut empty_first = blob(&[]);
        empty_first.pop();
        empty_first.extend_from_slice(&blob(image)[12..]);
        for (data, encoded) in
            [(valid, true), (empty_first, false), (blob(&[]), false), (synth::stream(&[(Tag::of(b"Data"), F::U8(7))]), false), (synth::stream(&[]), false)]
        {
            let fields = vec![
                (Tag::of(b"BmpW"), F::U32(2)),
                (Tag::of(b"BmpH"), F::U32(1)),
                (Tag::of(b"Frmt"), F::Enum(0, 0)),
                (Tag::of(b"Bckg"), F::Entry("c/1".into())),
            ];
            let (file, limits) = document(b"ImgN", fields, &[("c/1", &data, Method::Zstd)], None);
            let doc = crate::model::read(&file, limits).unwrap();
            let Kind::Image(result) = &doc.spreads[0].nodes[0].kind else { panic!() };
            assert_eq!(result.pixels, if encoded { Pixels::Encoded(image.to_vec()) } else { Pixels::Rgba8(vec![0; 8]) });
        }
    }

    /// Null links are skipped, but an off-crop tile still consumes its link.
    #[test]
    fn borrowed_tile_metadata_preserves_crop_links_and_invalid_states() {
        let base = || vec![(Tag::of(b"BmpW"), F::U32(768)), (Tag::of(b"BmpH"), F::U32(1)), (Tag::of(b"Frmt"), F::Enum(6, 0))];
        let mut fields = base();
        fields.push((Tag::of(b"Sta1"), F::U8s(vec![4, 4, 2])));
        fields.push((
            Tag::of(b"Idx1"),
            F::Shared(vec![
                F::Null,
                F::Def(5, vec![Tag::of(b"Blck")], vec![(Tag::of(b"Data"), F::Entry("missing".into()))]),
                F::Null,
                F::Def(6, vec![Tag::of(b"Blck")], vec![(Tag::of(b"Data"), F::Entry("tile".into()))]),
            ]),
        ));
        let (file, limits) = document(b"Rstr", fields, &[("tile", &[37; TILE_BYTES], Method::Zstd)], Some([256, 0, 513, 1]));
        let doc = crate::model::read(&file, limits).unwrap();
        let Kind::Image(image) = &doc.spreads[0].nodes[0].kind else { panic!() };
        let mut expected = [37, 37, 37, 255].repeat(256);
        expected.extend([255; 4]);
        assert_eq!(image.pixels, Pixels::Rgba8(expected));
        for state in [4u32, 256] {
            let mut field = vec![0x83];
            field.extend(Tag::of(b"Sta1").0.to_le_bytes());
            field.extend(1u32.to_le_bytes());
            field.extend(state.to_le_bytes());
            let mut fields = base();
            fields.push((Tag::of(b"Sta1"), F::Raw(field)));
            let (file, limits) = document(b"Rstr", fields, &[], None);
            assert!(matches!(crate::model::read(&file, limits), Err(Error::Malformed(_))));
        }
        // A tiny crop must not multiply the declared width past usize on 32-bit targets.
        let fields =
            vec![(Tag::of(b"BmpW"), F::U32(u32::MAX)), (Tag::of(b"BmpH"), F::U32(1)), (Tag::of(b"Frmt"), F::Enum(7, 0)), (Tag::of(b"Sta1"), F::U8s(vec![2]))];
        let (file, limits) = document(b"Rstr", fields, &[], Some([0, 0, 1, 1]));
        let doc = crate::model::read(&file, limits).unwrap();
        let Kind::Image(image) = &doc.spreads[0].nodes[0].kind else { panic!() };
        assert_eq!(image.pixels, Pixels::Rgba8(vec![255; 4]));
    }
}
