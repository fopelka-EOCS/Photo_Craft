//! The DCP profile label must be translated in every shipped catalog, including new locales.
//! Kept independent of the UI harness so this audit can also run without a renderer.
use std::fs;
use std::path::Path;

/// Every shipped locale provides exactly one nonempty translation of the camera-profile label.
#[test]
fn camera_profile_label_is_present_once_in_every_shipped_catalog() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/i18n");
    let mut catalogs = 0;
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|extension| extension != "tsv") {
            continue;
        }
        catalogs += 1;
        let text = fs::read_to_string(&path).unwrap();
        let translations: Vec<_> = text.lines().filter_map(|line| line.strip_prefix("\tProfile\t")).collect();
        assert_eq!(translations.len(), 1, "{}: Profile needs exactly one translation", path.display());
        assert!(!translations[0].trim().is_empty(), "{}: Profile must not fall back to English", path.display());
    }
    assert!(catalogs >= 20, "the audit must cover all shipped catalogs, including the nine locales named in review");
}
