//! Camera colour matrices from measured spectral sensitivities, for raws that carry no colour
//! matrix of their own.
//!
//! Ported from LightCraft (storytold/lightcraft `crates/raw/src/spectral.rs`, MIT OR Apache-2.0,
//! same organisation). [`find`] looks a camera up by its Exif make and model in a table that
//! LightCraft's `tools/spectral_camera_table.py` generates from rawtoaces-data (Academy Software
//! Foundation, Apache-2.0): per model, DNG `ColorMatrix1/2` and `ForwardMatrix1/2` for Standard
//! Illuminant A and D65, fitted to the camera's measured spectral sensitivities over
//! rawtoaces-data's training reflectances. [`Camera::color_info`] turns them into a [`ColorInfo`],
//! and the DNG model in [`crate::color`] does the rest. Other names of a covered camera (Rebel
//! T3i for EOS 600D) come from rawtoaces-data's alias list, and other bodies built on a covered
//! camera's sensor from [`SAME_SENSOR`]. No number comes from Adobe or from a GPL raw decoder.
//!
//! [`crate::develop`] uses them in place of the neutral fallback (camera channels as sRGB
//! primaries) when no look fitted to the camera's JPEG is used, and [`crate::fit_look`] keeps
//! their colour when the camera's JPEG is black and white and fits only its tone curve.

use crate::color::{Calibration, ColorInfo, IDENTITY, Mat3, illuminant_temperature};
use crate::sensor::Sensor;

#[path = "spectral_table.rs"]
mod table;

/// DNG `CalibrationIlluminant` codes of the two matrices: Standard Illuminant A, D65.
const ILLUMINANTS: [u32; 2] = [17, 21];

/// One camera's colour matrices.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub make: &'static str,
    pub model: &'static str,
    /// XYZ → camera under Standard Illuminant A and D65 (`ColorMatrix1/2`).
    pub color_matrix: [Mat3; 2],
    /// White-balanced camera → XYZ D50 (`ForwardMatrix1/2`).
    pub forward_matrix: [Mat3; 2],
}

impl Camera {
    /// The matrices as a DNG colour description (no as-shot white: that stays the file's).
    pub fn color_info(&self) -> ColorInfo {
        let calibrations = (0..2)
            .map(|i| Calibration {
                temperature: ILLUMINANTS.get(i).and_then(|&c| illuminant_temperature(c)).unwrap_or(5000.0),
                color_matrix: self.color_matrix.get(i).copied().unwrap_or(IDENTITY),
                forward_matrix: self.forward_matrix.get(i).copied(),
                camera_calibration: IDENTITY,
            })
            .collect();
        ColorInfo { calibrations, ..ColorInfo::default() }
    }
}

/// `(make, model as sold, table model)`: bodies sharing a table camera's sensor that
/// rawtoaces-data's alias list doesn't name, kept here because the table is generated.
/// Fujifilm's 26.1 MP X-Trans CMOS 4 bodies use the X-T3's sensor (an X-T30 raw shot in Acros
/// embeds a black-and-white JPEG, which gives no colour to fit; without matrices it opened in
/// the muted neutral fallback).
const SAME_SENSOR: &[(&str, &str, &str)] = &[
    ("Fujifilm", "X-T30", "X-T3"),
    ("Fujifilm", "X-T30 II", "X-T3"),
    ("Fujifilm", "X-Pro3", "X-T3"),
    ("Fujifilm", "X100V", "X-T3"),
    ("Fujifilm", "X-S10", "X-T3"),
    ("Fujifilm", "X-E4", "X-T3"),
];

/// Model names compared without case, the brand, a leading `EOS` or a trailing `DIGITAL`
/// (Exif "Canon EOS 400D DIGITAL" is the table's "EOS 400D").
fn key(brand: &str, model: &str) -> String {
    let mut words: Vec<String> = model.split_whitespace().map(str::to_uppercase).collect();
    if words.first().is_some_and(|w| *w == brand) {
        words.remove(0);
    }
    if words.first().is_some_and(|w| w == "EOS") {
        words.remove(0);
    }
    if words.len() > 1 && words.last().is_some_and(|w| w == "DIGITAL") {
        words.pop();
    }
    words.join(" ")
}

/// First word of a make, upper case ("NIKON CORPORATION" → "NIKON").
fn brand(make: &str) -> String {
    make.split_whitespace().next().unwrap_or_default().to_uppercase()
}

/// The table's camera for a file's Exif make and model, directly or through an alias. Without a
/// make, the model alone must match.
pub fn find(make: Option<&str>, model: &str) -> Option<&'static Camera> {
    let make = make.map(brand).filter(|b| !b.is_empty());
    let same = |row_make: &str, row_model: &str| {
        let row_brand = brand(row_make);
        let wanted = key(&row_brand, model);
        make.as_ref().is_none_or(|m| *m == row_brand) && !wanted.is_empty() && key(&row_brand, row_model) == wanted
    };
    match table::ALIASES.iter().chain(SAME_SENSOR).find(|(m, alias, _)| same(m, alias)) {
        Some((m, _, target)) => table::CAMERAS.iter().find(|c| brand(c.make) == brand(m) && c.model == *target),
        None => table::CAMERAS.iter().find(|c| same(c.make, c.model)),
    }
}

/// Every camera in the table.
pub fn cameras() -> &'static [Camera] {
    table::CAMERAS
}

/// The table's matrices for a colour sensor without colour calibration of its own.
pub(crate) fn for_sensor(s: &Sensor) -> Option<&'static Camera> {
    if !s.color.calibrations.is_empty() || s.is_monochrome() {
        return None;
    }
    find(s.make.as_deref(), s.model.as_deref()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{D50, apply};

    #[test]
    fn exif_names_find_their_rows() {
        for (make, model, row) in [
            ("Canon", "Canon EOS 5D Mark III", "EOS 5D Mark III"),
            ("Canon", "Canon EOS 600D", "EOS 600D"),
            ("Canon", "Canon EOS DIGITAL REBEL XTi", "Digital Rebel XTi"),
            ("NIKON CORPORATION", "NIKON D850", "D850"),
            ("SONY", "ILCE-7M3", "ILCE-7M3"),
            ("FUJIFILM", "X-T4", "X-T4"),
            ("Panasonic", "DC-GX9", "DC-GX9"),
        ] {
            assert_eq!(find(Some(make), model).map(|c| c.model), Some(row), "{make} {model}");
        }
        assert_eq!(find(None, "NIKON D850").map(|c| c.model), Some("D850"));
    }

    #[test]
    fn aliases_and_same_sensor_bodies_reach_their_camera() {
        for (make, model, row) in [
            ("Canon", "Canon EOS 400D DIGITAL", "Digital Rebel XTi"),
            ("Canon", "Canon EOS REBEL T3i", "EOS 600D"),
            ("Canon", "Canon EOS R6 Mark II", "EOS R6m2"),
            ("NIKON CORPORATION", "NIKON D800", "D800E"),
            ("SONY", "ILCE-7RM4A", "ILCE-7RM4"),
            ("FUJIFILM", "GFX100S", "GFX 100"),
            ("FUJIFILM", "X-T30", "X-T3"),
            ("FUJIFILM", "X-T30 II", "X-T3"),
            ("FUJIFILM", "X100V", "X-T3"),
        ] {
            assert_eq!(find(Some(make), model).map(|c| c.model), Some(row), "{make} {model}");
        }
        for (make, alias, _) in table::ALIASES.iter().chain(SAME_SENSOR) {
            assert!(find(Some(make), alias).is_some(), "{make} {alias}");
        }
    }

    #[test]
    fn other_cameras_and_other_makes_find_nothing() {
        assert_eq!(find(Some("Canon"), "Canon EOS 7D"), None);
        assert_eq!(find(Some("OLYMPUS IMAGING CORP."), "E-M5"), None);
        assert_eq!(find(Some("Canon"), "D850"), None);
        assert_eq!(find(Some("SONY"), "X-T4"), None);
        assert_eq!(find(None, ""), None);
        assert_eq!(find(Some(""), ""), None);
    }

    #[test]
    fn develop_uses_the_matrices_in_place_of_the_neutral_fallback() {
        let (w, h) = (64, 48);
        let data: Vec<u16> = (0..w * h).map(|i| (200 + (i * 37) % 3000) as u16).collect();
        let b = crate::testgen::tiff_ep("NIKON CORPORATION", w, h, &data, [0, 1, 1, 2], 12, vec![]);
        let mut s = crate::decode(&b, &crate::Limits::default()).unwrap();
        let fallback = |w: &String| w.contains("sRGB primaries");
        let plain = crate::develop_sensor(&s, &Default::default()).unwrap();
        assert!(plain.warnings.iter().any(fallback), "{:?}", plain.warnings);

        s.make = Some("FUJIFILM".into());
        s.model = Some("X-T30".into());
        let d = crate::develop_sensor(&s, &Default::default()).unwrap();
        assert!(d.warnings.iter().any(|w| w.contains("spectral sensitivities of the Fujifilm X-T3")), "{:?}", d.warnings);
        assert!(!d.warnings.iter().any(fallback), "{:?}", d.warnings);
        assert_ne!(d.rgb, plain.rgb);

        // A file's own calibration wins; monochrome data takes none.
        let own = find(Some("SONY"), "ILCE-7M3").unwrap().color_info();
        s.color = own;
        assert!(for_sensor(&s).is_none());
        s.color = ColorInfo::default();
        s.cfa = None;
        assert!(for_sensor(&s).is_none());
    }

    #[test]
    fn every_row_is_a_usable_dng_profile() {
        assert_eq!(cameras().len(), 52);
        for c in cameras() {
            assert_eq!(find(Some(c.make), c.model), Some(c), "{} {}", c.make, c.model);
            for m in c.color_matrix.iter().chain(&c.forward_matrix) {
                assert!(m.iter().flatten().all(|v| v.is_finite() && v.abs() < 4.0), "{}", c.model);
                assert!(crate::color::invert(m).is_some(), "{}", c.model);
            }
            // A forward matrix maps white-balanced white to D50 (DNG spec, chapter 6).
            for fm in &c.forward_matrix {
                let w = apply(fm, [1.0; 3]);
                assert!((0..3).all(|i| (w[i] - D50[i]).abs() < 1e-3), "{} {w:?}", c.model);
            }
            // Daylight: the balanced camera → XYZ matrix exists and keeps white white.
            let info = c.color_info();
            assert_eq!(info.calibrations.len(), 2);
            let n = info.xy_to_neutral(crate::color::D65_XY).unwrap();
            let m = info.balanced_to_xyz_d50(n).unwrap();
            let w = apply(&m, [1.0; 3]);
            assert!((0..3).all(|i| (w[i] - D50[i]).abs() < 2e-3), "{} {w:?}", c.model);
        }
    }
}
