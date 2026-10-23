//! Shared raster PDF writer, lifted from the printing path. Print, artboard export,
//! Save As and binders use the same page/object/xref implementation.
use std::io::Write as _;

/// The image placed on a print page.
pub struct PrintImage {
    pub width: u32,
    pub height: u32,
    /// 1 (gray), 3 (RGB) or 4 (CMYK, 1 = full ink), 8-bit interleaved.
    pub channels: usize,
    pub data: Vec<u8>,
    pub icc: Option<Vec<u8>>,
}

/// Printer marks around the image.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Marks {
    pub corner_crop: bool,
    pub center_crop: bool,
    pub registration: bool,
    /// Photoshop's Functions › Bleed, in points: corner crop marks sit this far inside the image
    /// edge, so the trim cuts through the image and leaves no paper-white sliver. 0 = on the edge.
    pub bleed: f64,
}

/// Gap between the image edge and a crop mark, and the mark's length (pt).
pub const MARK_GAP: f64 = 6.0;
pub const MARK_LEN: f64 = 18.0;

/// Corner crop marks around `(x, y, w, h)` as eight `(from, to)` segments, a horizontal then a
/// vertical one per corner. Each starts `MARK_GAP` outside the image (so it never prints on it)
/// and lies on the trim line, `bleed` inside the edge. The inset is symmetric, so this holds for
/// both bottom-up (PDF) and top-down (Windows) coordinates.
pub fn corner_crop_segments(bleed: f64, (x, y, w, h): (f64, f64, f64, f64)) -> [((f64, f64), (f64, f64)); 8] {
    let mut out = [((0.0, 0.0), (0.0, 0.0)); 8];
    for (i, (cx, cy, dx, dy)) in [(x, y, -1.0, -1.0), (x + w, y, 1.0, -1.0), (x, y + h, -1.0, 1.0), (x + w, y + h, 1.0, 1.0)].into_iter().enumerate() {
        let (tx, ty) = (cx - dx * bleed, cy - dy * bleed);
        out[2 * i] = ((cx + dx * MARK_GAP, ty), (cx + dx * (MARK_GAP + MARK_LEN), ty));
        out[2 * i + 1] = ((tx, cy + dy * MARK_GAP), (tx, cy + dy * (MARK_GAP + MARK_LEN)));
    }
    out
}

/// One page: paper (pt), image box (pt, origin bottom-left), marks and texts.
pub struct PrintPage {
    pub paper: (f64, f64),
    pub image: PrintImage,
    pub rect: (f64, f64, f64, f64),
    pub marks: Marks,
    pub description: Option<String>,
    pub label: Option<String>,
    /// Mirror the complete print page horizontally for transfer printing.
    pub emulsion_down: bool,
}

/// Encode print labels in WinAnsi with fixed-width octal escapes for non-ASCII bytes.
// ISO 32000-1, Annex D.2: the standard Helvetica font below uses WinAnsiEncoding,
// not UTF-8. Escape non-ASCII bytes as three octal digits so the content stays ASCII
// and a following digit cannot become part of the escape. Other scripts still need
// an embedded font; retain the existing '?' fallback for unsupported characters.
fn pdf_string(s: &str) -> String {
    let mut o = String::from("(");
    for c in s.chars() {
        let byte = match c {
            '\u{20}'..='\u{7e}' | '\u{a0}'..='\u{ff}' => c as u8,
            '€' => 0x80,
            '‚' => 0x82,
            'ƒ' => 0x83,
            '„' => 0x84,
            '…' => 0x85,
            '†' => 0x86,
            '‡' => 0x87,
            'ˆ' => 0x88,
            '‰' => 0x89,
            'Š' => 0x8a,
            '‹' => 0x8b,
            'Œ' => 0x8c,
            'Ž' => 0x8e,
            '‘' => 0x91,
            '’' => 0x92,
            '“' => 0x93,
            '”' => 0x94,
            '•' => 0x95,
            '–' => 0x96,
            '—' => 0x97,
            '˜' => 0x98,
            '™' => 0x99,
            'š' => 0x9a,
            '›' => 0x9b,
            'œ' => 0x9c,
            'ž' => 0x9e,
            'Ÿ' => 0x9f,
            _ => b'?',
        };
        match byte {
            b'(' | b')' | b'\\' => {
                o.push('\\');
                o.push(char::from(byte));
            }
            0x80..=0xff => o.push_str(&format!("\\{byte:03o}")),
            _ => o.push(char::from(byte)),
        }
    }
    o.push(')');
    o
}

/// Encode a byte slice with the shared lossless PDF stream compressor.
fn deflate(data: &[u8]) -> Vec<u8> {
    // Fast: print PDFs are transient and large; level 1 is ~4× quicker than the default.
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

/// Vector marks around `(x, y, w, h)`.
fn marks_ops(m: Marks, (x, y, w, h): (f64, f64, f64, f64)) -> String {
    let mut s = String::from("q 0 G 0.3 w\n");
    let (gap, len) = (MARK_GAP, MARK_LEN);
    let line = |s: &mut String, x0: f64, y0: f64, x1: f64, y1: f64| s.push_str(&format!("{x0:.2} {y0:.2} m {x1:.2} {y1:.2} l S\n"));
    if m.corner_crop {
        for ((x0, y0), (x1, y1)) in corner_crop_segments(m.bleed, (x, y, w, h)) {
            line(&mut s, x0, y0, x1, y1);
        }
    }
    if m.center_crop {
        let (mx, my) = (x + w / 2.0, y + h / 2.0);
        line(&mut s, mx, y - gap, mx, y - gap - len);
        line(&mut s, mx, y + h + gap, mx, y + h + gap + len);
        line(&mut s, x - gap, my, x - gap - len, my);
        line(&mut s, x + w + gap, my, x + w + gap + len, my);
    }
    if m.registration {
        let r = 5.0;
        let off = gap + len / 2.0;
        for (cx, cy) in [(x + w / 2.0, y - off), (x + w / 2.0, y + h + off), (x - off, y + h / 2.0), (x + w + off, y + h / 2.0)] {
            // Circle (four Béziers) plus crosshair.
            let k = 0.5523 * r;
            s.push_str(&format!(
                "{:.2} {cy:.2} m {:.2} {:.2} {:.2} {:.2} {cx:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {cy:.2} c {:.2} {:.2} {:.2} {:.2} {cx:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {cy:.2} c S\n",
                cx + r, cx + r, cy + k, cx + k, cy + r, cy + r, cx - k, cy + r, cx - r, cy + k, cx - r, cx - r, cy - k, cx - k, cy - r, cy - r, cx + k, cy - r, cx + r, cy - k, cx + r
            ));
            line(&mut s, cx - r * 1.6, cy, cx + r * 1.6, cy);
            line(&mut s, cx, cy - r * 1.6, cx, cy + r * 1.6);
        }
    }
    s.push_str("Q\n");
    s
}

/// Encoded 8-bit raster. `alpha` is a Flate-compressed 8-bit soft mask.
pub struct RasterImage {
    pub width: u32,
    pub height: u32,
    pub channels: usize,
    pub data: Vec<u8>,
    pub jpeg: bool,
    pub icc: Option<Vec<u8>>,
    pub alpha: Option<Vec<u8>>,
}

/// A page with its own media size and PDF painting operators.
pub struct RasterPage {
    pub paper: (f64, f64),
    pub image: RasterImage,
    pub content: String,
}

/// Wrap encoded bytes in a PDF stream object with an exact byte length.
fn stream(dict: &str, bytes: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict} /Length {} >>\nstream\n", bytes.len()).into_bytes();
    out.extend_from_slice(bytes);
    out.extend_from_slice(b"\nendstream");
    out
}

/// One implementation of catalog, page tree, image resources, metadata and xref.
pub fn write_pdf(pages: &[RasterPage], xmp: Option<&str>) -> Vec<u8> {
    let mut objects = vec![Vec::new(), Vec::new()];
    let mut kids = Vec::new();
    for page in pages {
        let id = objects.len() + 1;
        let (content_id, image_id, font_id) = (id + 1, id + 2, id + 3);
        let img = &page.image;
        let icc_id = id + 4;
        let mask_id = id + 4 + usize::from(img.icc.is_some());
        let device = match img.channels {
            1 => "/DeviceGray",
            4 => "/DeviceCMYK",
            _ => "/DeviceRGB",
        };
        let cs = if img.icc.is_some() { format!("[/ICCBased {icc_id} 0 R]") } else { device.into() };
        let mask = if img.alpha.is_some() { format!(" /SMask {mask_id} 0 R") } else { String::new() };
        let filter = if img.jpeg { "/DCTDecode" } else { "/FlateDecode" };
        let (pw, ph) = page.paper;
        kids.push(format!("{id} 0 R"));
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.6} {ph:.6}] /Resources << /XObject << /Im0 {image_id} 0 R >> /Font << /F1 {font_id} 0 R >> >> /Contents {content_id} 0 R >>").into_bytes());
        objects.push(stream("/Filter /FlateDecode", &deflate(page.content.as_bytes())));
        objects.push(stream(
            &format!("/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {cs} /BitsPerComponent 8 /Filter {filter}{mask}", img.width, img.height),
            &img.data,
        ));
        objects.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec());
        if let Some(icc) = &img.icc {
            objects.push(stream(&format!("/N {} /Alternate {device} /Filter /FlateDecode", img.channels), &deflate(icc)));
        }
        if let Some(alpha) = &img.alpha {
            objects.push(stream(
                &format!(
                    "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode",
                    img.width, img.height
                ),
                alpha,
            ));
        }
    }
    let metadata = if let Some(xmp) = xmp {
        let id = objects.len() + 1;
        objects.push(stream("/Type /Metadata /Subtype /XML", xmp.as_bytes()));
        format!(" /Metadata {id} 0 R")
    } else {
        String::new()
    };
    if let Some(catalog) = objects.get_mut(0) {
        *catalog = format!("<< /Type /Catalog /Pages 2 0 R{metadata} >>").into_bytes();
    }
    if let Some(tree) = objects.get_mut(1) {
        *tree = format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).into_bytes();
    }
    let mut out = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
    out
}

/// A print page, retaining paper background, color channels, ICC, marks and labels.
pub fn print_pdf(page: &PrintPage) -> Vec<u8> {
    let (pw, ph) = page.paper;
    let (x, y, w, h) = page.rect;
    // An explicit white page, so every viewer (and rasteriser) shows paper, not transparency.
    let mut content = format!("q 1 g 0 0 {pw:.2} {ph:.2} re f Q\n");
    if page.emulsion_down {
        content.push_str(&format!("q -1 0 0 1 {pw:.2} 0 cm\n"));
    }
    content.push_str(&format!("q {w:.3} 0 0 {h:.3} {x:.3} {y:.3} cm /Im0 Do Q\n"));
    content.push_str(&marks_ops(page.marks, page.rect));
    let any_marks = page.marks.corner_crop || page.marks.center_crop || page.marks.registration;
    let pad = if any_marks { 30.0 } else { 6.0 };
    if let Some(l) = page.label.as_deref().filter(|l| !l.is_empty()) {
        content.push_str(&format!("BT /F1 8 Tf 0 g {:.2} {:.2} Td {} Tj ET\n", x, y + h + pad, pdf_string(l)));
    }
    if let Some(d) = page.description.as_deref().filter(|d| !d.is_empty()) {
        content.push_str(&format!("BT /F1 8 Tf 0 g {:.2} {:.2} Td {} Tj ET\n", x, y - pad - 8.0, pdf_string(d)));
    }

    if page.emulsion_down {
        content.push_str("Q\n");
    }
    let img = &page.image;
    write_pdf(
        &[RasterPage {
            paper: page.paper,
            content,
            image: RasterImage {
                width: img.width,
                height: img.height,
                channels: img.channels,
                data: deflate(&img.data),
                jpeg: false,
                icc: img.icc.clone(),
                alpha: None,
            },
        }],
        None,
    )
}

/// Artboards retain their existing JPEG encoding and physical sizes.
pub fn raster_pdf(pages: &[(u32, u32, f32, Vec<u8>)]) -> Vec<u8> {
    let pages: Vec<_> = pages
        .iter()
        .map(|(w, h, dpi, jpeg)| {
            let k = 72.0 / f64::from(dpi.max(1.0));
            let (pw, ph) = (f64::from(*w) * k, f64::from(*h) * k);
            RasterPage {
                paper: (pw, ph),
                content: format!("q {pw:.6} 0 0 {ph:.6} 0 0 cm /Im0 Do Q"),
                image: RasterImage { width: *w, height: *h, channels: 3, data: jpeg.clone(), jpeg: true, icc: None, alpha: None },
            }
        })
        .collect();
    write_pdf(&pages, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn print_strings_keep_winansi_accents_and_punctuation() {
        assert_eq!(pdf_string("Café – résumé"), r"(Caf\351 \226 r\351sum\351)");
        assert_eq!(pdf_string("€ ‘quote’ “double” • Œœ Šš Žž Ÿ"), r"(\200 \221quote\222 \223double\224 \225 \214\234 \212\232 \216\236 \237)");
        assert_eq!(pdf_string("é7"), r"(\3517)", "octal escapes must have three digits before a digit");
    }

    #[test]
    fn print_strings_keep_pdf_delimiters_safe_and_fallback_bounded() {
        assert_eq!(pdf_string(r"a(b)\c"), r"(a\(b\)\\c)");
        assert_eq!(pdf_string("\n\r\t\0\u{7f}\u{81}中😀"), "(????????)");
        assert_eq!(pdf_string(""), "()");
    }
}
