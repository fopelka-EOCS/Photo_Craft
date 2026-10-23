//! Reject image decoders without enforceable budgets before Hayro sees a page.
//! This deliberately supports a conservative subset: unfiltered, Flate and JPEG
//! streams. JPX/JBIG2/CCITT/LZW and inline images fail explicitly, not as missing
//! artwork. PDF dictionaries alone cannot bound codec-internal allocations.
use std::io::Read;

use super::error;
use crate::IoError;
use hayro::hayro_syntax::{
    Pdf,
    object::{Array, Dict, Name, Object, Stream},
};
use photocraft_raster::Interrupt;

// Keep total estimated system use below 85% of physical RAM. The remaining
// headroom is shared by decoded streams, renderer/cache copies and output tiles.
const FALLBACK_HEADROOM: u64 = 1024 * 1024 * 1024;

/// Estimate additional memory available while reserving 15% of physical RAM; use a fallback if RAM is unknown.
fn headroom(total: u64, available: u64) -> u64 {
    if total == 0 {
        return FALLBACK_HEADROOM;
    }
    let ceiling = ((u128::from(total) * 85) / 100) as u64;
    available.min(total).saturating_sub(total - ceiling)
}

/// Refresh available native RAM for admission estimates; never query native system APIs on the web.
fn memory_headroom() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut system = sysinfo::System::new();
        system.refresh_memory_specifics(sysinfo::MemoryRefreshKind::nothing().with_ram());
        headroom(system.total_memory(), system.available_memory())
    }
    #[cfg(target_arch = "wasm32")]
    {
        // This older branch compiles import helpers on the web, but its public
        // entry points reject before parsing. Do not query native APIs there.
        headroom(0, 0)
    }
}

/// Reserve room for four estimated stream copies within the current available-memory allowance.
fn stream_budget() -> usize {
    usize::try_from(memory_headroom() / 4).unwrap_or(usize::MAX / 4)
}

/// Admit estimated stream copies plus raster/tile storage; this cannot constrain third-party allocations.
pub(super) fn check_render_memory(decoded: usize, pixels: u64) -> Result<(), IoError> {
    // Four copies of streams plus render pixels, copied RGBA and tile storage.
    let estimate = (decoded as u64).saturating_mul(4).saturating_add(pixels.saturating_mul(12));
    if estimate > memory_headroom() {
        return Err(error("PDF import would exceed the 85% system RAM budget; close other applications, select fewer pages or reduce resolution"));
    }
    Ok(())
}
const IMAGE_PIXELS: u64 = 16_000_000;
const TOTAL_IMAGE_PIXELS: u64 = 64_000_000;
const OBJECTS: usize = 100_000;

/// Reject zero, excessive-side and excessive-area embedded images before decoding.
fn dimensions(w: u32, h: u32) -> Result<u64, IoError> {
    let pixels = u64::from(w) * u64::from(h);
    if w == 0 || h == 0 || w > 16_384 || h > 16_384 || pixels > IMAGE_PIXELS {
        return Err(error("embedded PDF image exceeds the 16,384-pixel side or 16-megapixel decode limit"));
    }
    Ok(pixels)
}

/// Translate cooperative cancellation into the import error shared by the caller.
fn cancelled(ctl: &Interrupt) -> Result<(), IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)
}

/// Check the actual JPEG frame, not only the PDF's Width/Height declaration.
fn jpeg_dimensions(bytes: &[u8]) -> Result<u64, IoError> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return Err(error("invalid embedded JPEG"));
    }
    let mut pos = 2usize;
    while let Some(&byte) = bytes.get(pos) {
        if byte != 0xff {
            return Err(error("invalid embedded JPEG marker"));
        }
        while bytes.get(pos) == Some(&0xff) {
            pos += 1;
        }
        let marker = *bytes.get(pos).ok_or_else(|| error("truncated embedded JPEG"))?;
        pos += 1;
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        if marker == 1 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let len = bytes.get(pos..pos + 2).ok_or_else(|| error("truncated embedded JPEG segment"))?;
        let [a, b] = *len else {
            return Err(error("invalid JPEG segment length"));
        };
        let len = usize::from(u16::from_be_bytes([a, b]));
        if len < 2 {
            return Err(error("invalid embedded JPEG segment"));
        }
        let segment = bytes.get(pos..pos + len).ok_or_else(|| error("truncated embedded JPEG segment"))?;
        if (0xc0..=0xcf).contains(&marker) && ![0xc4, 0xc8, 0xcc].contains(&marker) {
            let frame = segment.get(2..8).ok_or_else(|| error("truncated embedded JPEG frame"))?;
            let [depth, h0, h1, w0, w1, components] = *frame else {
                return Err(error("invalid JPEG frame"));
            };
            if depth != 8 || ![1, 3, 4].contains(&components) {
                return Err(error("unsupported embedded JPEG color depth"));
            }
            return dimensions(u32::from(u16::from_be_bytes([w0, w1])), u32::from(u16::from_be_bytes([h0, h1])));
        }
        pos += len;
    }
    Err(error("embedded JPEG has no supported frame"))
}

/// Decode Flate incrementally with cancellation checks and a hard bound on the accumulated output.
fn inflate(bytes: &[u8], ctl: &Interrupt, limit: usize) -> Result<Vec<u8>, IoError> {
    let mut decoder = flate2::read::ZlibDecoder::new(bytes);
    let mut out = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        cancelled(ctl)?;
        let n = decoder.read(&mut chunk).map_err(|_| error("invalid PDF Flate stream"))?;
        if n == 0 {
            break;
        }
        if out.len().saturating_add(n) > limit {
            return Err(error("PDF decoded stream exceeds its memory budget or declared image size"));
        }
        out.try_reserve(n).map_err(|_| error("not enough memory to decode PDF stream"))?;
        out.extend_from_slice(chunk.get(..n).ok_or_else(|| error("invalid decoded stream length"))?);
    }
    Ok(out)
}

/// Bound predictor row geometry and reject parameter arrays that could bypass single-filter validation.
fn predictor(dict: &Dict<'_>) -> Result<(), IoError> {
    let params = dict.get::<Dict<'_>>(b"DecodeParms").or_else(|| dict.get::<Dict<'_>>(b"DP"));
    // Filter arrays are rejected below; don't let an array of parameters evade checks.
    if dict.get::<Array<'_>>(b"DecodeParms").is_some() || dict.get::<Array<'_>>(b"DP").is_some() {
        return Err(error("PDF stream filter parameter arrays are not supported by bounded import"));
    }
    if let Some(p) = params {
        let columns = p.get::<u32>(b"Columns").unwrap_or(1);
        let colors = p.get::<u32>(b"Colors").unwrap_or(1);
        let bits = p.get::<u32>(b"BitsPerComponent").unwrap_or(8);
        if columns == 0 || columns > 16_384 || !(1..=4).contains(&colors) || ![1, 2, 4, 8, 16].contains(&bits) {
            return Err(error("PDF predictor exceeds decode limits"));
        }
    }
    Ok(())
}

/// Accept supported image color spaces while bounding nested indexed spaces to prevent unbounded recursion.
fn image_colorspace(space: Object<'_>, depth: u8) -> Result<(), IoError> {
    if depth > 2 {
        return Err(error("PDF image color space nesting exceeds decode limits"));
    }
    let supported = match space {
        Object::Name(n) => matches!(n.as_ref(), b"DeviceGray" | b"DeviceRGB" | b"DeviceCMYK"),
        Object::Array(a) => {
            let mut items = a.iter::<Object<'_>>();
            match items.next().and_then(Object::into_name).as_ref().map(|n| n.as_ref()) {
                Some(b"ICCBased") => items.next().and_then(Object::into_stream).is_some_and(|s| [1, 3, 4].contains(&s.dict().get::<u32>(b"N").unwrap_or(0))),
                Some(b"Indexed") => {
                    if let Some(base) = items.next() {
                        image_colorspace(base, depth + 1)?;
                    } else {
                        return Err(error("missing PDF indexed color space"));
                    }
                    true
                }
                _ => false,
            }
        }
        _ => false,
    };
    if !supported {
        return Err(error("PDF image color space is not supported by bounded import"));
    }
    Ok(())
}

/// Recognize the BI operator while skipping strings, names, hex strings and comments that merely contain its letters.
fn has_inline_image(data: &[u8]) -> bool {
    let delimiter = |b: u8| b.is_ascii_whitespace() || b"()<>[]{}/%".contains(&b);
    let mut pos = 0;
    while let Some(&byte) = data.get(pos) {
        pos += 1;
        match byte {
            b'%' => {
                while data.get(pos).is_some_and(|b| !matches!(b, b'\r' | b'\n')) {
                    pos += 1;
                }
                continue;
            }
            b'(' => {
                // Literal strings can contain nested parentheses and escaped delimiters.
                // Drawing labels such as [(U) -20 (BI) 10 (NG)] are text, not operators.
                let mut depth = 1usize;
                while let Some(&b) = data.get(pos) {
                    pos += 1;
                    match b {
                        b'\\' => pos = pos.saturating_add(1).min(data.len()),
                        b'(' => depth = depth.saturating_add(1),
                        b')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                continue;
            }
            b'<' => {
                if data.get(pos) == Some(&b'<') {
                    pos += 1;
                } else {
                    while let Some(&b) = data.get(pos) {
                        pos += 1;
                        if b == b'>' {
                            break;
                        }
                    }
                }
                continue;
            }
            b if delimiter(b) => continue,
            _ => {}
        }
        let start = pos - 1;
        while data.get(pos).is_some_and(|b| !delimiter(*b)) {
            pos += 1;
        }
        let Some(token) = data.get(start..pos) else { return false };
        let mut bytes = token.iter().copied();
        // Hayro normalizes name escapes while reading operators too. Cover those
        // spellings even though conforming PDF content uses the literal BI token.
        let mut next = || {
            let b = bytes.next()?;
            if b != b'#' {
                return Some(b);
            }
            let a = char::from(bytes.next()?).to_digit(16)? as u8;
            let b = char::from(bytes.next()?).to_digit(16)? as u8;
            Some((a << 4) | b)
        };
        if next() == Some(b'B') && next() == Some(b'I') && next().is_none() {
            return true;
        }
    }
    false
}

/// Validate one stream and its decoder, update aggregate image costs, and return its admitted byte cost.
fn stream(stream: &Stream<'_>, ctl: &Interrupt, images: &mut u64, budget: usize) -> Result<usize, IoError> {
    cancelled(ctl)?;
    let dict = stream.dict();
    let is_image = dict.get::<Name<'_>>(b"Subtype").is_some_and(|n| n.as_ref() == b"Image");
    let mut limit = budget;
    if is_image {
        if let Some(space) = dict.get::<Object<'_>>(b"ColorSpace") {
            image_colorspace(space, 0)?;
        }
        let height = dict.get::<u32>(b"Height").unwrap_or(0);
        let pixels = dimensions(dict.get::<u32>(b"Width").unwrap_or(0), height)?;
        // Up to four 16-bit components and one predictor byte per row. A small
        // declared image must not acquire the entire machine's stream budget.
        limit = limit.min(usize::try_from(pixels * 8 + u64::from(height)).unwrap_or(usize::MAX));
        *images = images.saturating_add(pixels);
        if *images > TOTAL_IMAGE_PIXELS {
            return Err(error("PDF embedded images exceed 64 megapixels in total"));
        }
        if ![1, 2, 4, 8, 16].contains(&dict.get::<u32>(b"BitsPerComponent").unwrap_or(1)) {
            return Err(error("unsupported PDF image bit depth"));
        }
    }
    // Inspect names via the parser so escaped names and indirect filters cannot bypass this.
    if dict.get::<Array<'_>>(b"Filter").is_some() || dict.get::<Array<'_>>(b"F").is_some() {
        return Err(error("PDF filter chains are not supported by bounded import; resave with Flate or JPEG compression"));
    }
    let name = dict.get::<Name<'_>>(b"Filter").or_else(|| dict.get::<Name<'_>>(b"F"));
    let filter = name.as_ref().map(|n| n.as_ref());
    if filter.is_none() && (dict.contains_key(b"Filter") || dict.contains_key(b"F")) {
        return Err(error("invalid PDF stream filter"));
    }
    if !matches!(filter, None | Some(b"FlateDecode" | b"Fl" | b"DCTDecode" | b"DCT")) {
        return Err(error(
            "PDF image/stream codec has no bounded decoder (including JPEG2000, JBIG2, CCITT and LZW). Resave the PDF using JPEG or Flate compression.",
        ));
    }
    predictor(dict)?;
    if !is_image && dict.get::<Dict<'_>>(b"DecodeParms").or_else(|| dict.get::<Dict<'_>>(b"DP")).is_some_and(|p| p.get::<u32>(b"Predictor").unwrap_or(1) != 1) {
        return Err(error("PDF predictors are only supported on image streams"));
    }
    let raw = stream.raw_data();
    if raw.len() > budget {
        return Err(error("PDF stream exceeds the 85% system RAM budget"));
    }
    let decoded = match filter {
        Some(b"FlateDecode" | b"Fl") => std::borrow::Cow::Owned(inflate(&raw, ctl, limit)?),
        Some(b"DCTDecode" | b"DCT") => {
            if !is_image {
                return Err(error("JPEG filter on a non-image PDF stream"));
            }
            // Both dimensions are bounded independently; count the larger codec frame too.
            *images = images.saturating_add(jpeg_dimensions(&raw)?);
            if *images > TOTAL_IMAGE_PIXELS {
                return Err(error("PDF embedded images exceed the decode budget"));
            }
            return Ok(raw.len());
        }
        _ => raw,
    };
    if decoded.len() > limit {
        return Err(error("PDF decoded stream exceeds its memory budget or declared image size"));
    }
    // Inline images can occur in page, form, pattern, appearance and Type3 streams.
    // Reject the operator before interpretation, including when nested in resources.
    // Skip strings and comments so text labels containing BI are not mistaken for operators.
    if !is_image && has_inline_image(&decoded) {
        return Err(error("Inline PDF images are not supported by bounded import; resave them as image XObjects"));
    }
    Ok(decoded.len())
}

/// Inspect all available streams under object, aggregate image and decoded-byte limits before rendering.
pub(super) fn preflight(pdf: &Pdf, ctl: &Interrupt) -> Result<usize, IoError> {
    if pdf.len() > OBJECTS {
        return Err(error("PDF exceeds 100,000 objects"));
    }
    let budget = stream_budget();
    let (mut total, mut images) = (0usize, 0u64);
    for object in pdf.objects() {
        cancelled(ctl)?;
        if let Some(s) = object.into_stream() {
            total = total.saturating_add(stream(&s, ctl, &mut images, budget.saturating_sub(total))?);
            if total > budget {
                return Err(error("PDF decoded streams exceed the 85% system RAM budget"));
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    #[test]
    fn budget_keeps_fifteen_percent_of_ram_and_accounts_for_other_apps() {
        assert_eq!(super::headroom(1000, 1000), 850);
        assert_eq!(super::headroom(1000, 400), 250);
        assert_eq!(super::headroom(1000, 150), 0);
        assert_eq!(super::headroom(1000, 100), 0);
        assert_eq!(super::headroom(0, 0), super::FALLBACK_HEADROOM);
        assert!(super::headroom(u64::MAX, u64::MAX) < u64::MAX);
    }

    #[test]
    fn larger_streams_work_but_the_selected_budget_is_still_enforced() {
        use std::io::Write;
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        for _ in 0..1025 {
            encoder.write_all(&[b' '; 65536]).unwrap();
        }
        let bytes = encoder.finish().unwrap();
        let ctl = photocraft_raster::Interrupt::default();
        assert!(super::inflate(&bytes, &ctl, 64 * 1024 * 1024).is_err());
        assert_eq!(super::inflate(&bytes, &ctl, 85 * 1024 * 1024).unwrap().len(), 1025 * 65536);
    }

    #[test]
    fn inline_image_check_ignores_text_strings_and_comments() {
        for data in [
            b"BT [(U) -20 (BI) 10 (NG)] TJ ET".as_slice(),
            b"BT (BI) Tj (nested (BI) text) Tj ET",
            br"BT (escaped \(BI\) and \\ BI) Tj ET",
            b"% BI /W 100\r\nBT <4249> Tj ET",
            b"(unfinished BI",
            b"<4249",
        ] {
            assert!(!super::has_inline_image(data), "{data:?}");
        }
        for data in [b"(BI) BI/W 1".as_slice(), b"% BI\nBI/W 1", b"<4249>BI/W 1", br"(escaped \) BI) BI/W 1", b"<< /Key (BI) >> BI/W 1"] {
            assert!(super::has_inline_image(data), "{data:?}");
        }
    }

    #[test]
    fn inline_image_check_covers_operator_delimiters() {
        for data in [b"BI /W 1".as_slice(), b"q BI/W 1", b"q\nBI\n/W 1", b"BI", b"(x)BI/W 1", b"q B#49/W 1", b"q #42I/W 1"] {
            assert!(super::has_inline_image(data));
        }
        assert!(!super::has_inline_image(b"BT (BINDER) Tj ET"));
    }
}
