//! #8266: both save paths must honor the reader's manifest budget before publication.
use photocraft_doc::{ColorMode, Document, Layer, LayerContent, SampleType, Size, SmartFilter, SmartObject, SmartSource};
use photocraft_format::{FormatError, LoadOptions, PcraftWriter, SaveOptions, load_from_bytes, load_from_bytes_with, load_path};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

fn document() -> Document {
    let mut doc = Document::new("map", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
    let mut smart = SmartObject::new(SmartSource::Embedded { file_name: "fixture".into(), bytes: Arc::new(Vec::new()) }, Default::default(), None);
    smart.smart_filters.push(SmartFilter {
        command: "filter.distort.displace".into(),
        params: json!({"horizontal": 10, "__displacePsd": "ab".repeat(8192), "unknownTemplateSetting": [1, 2, 3]}).into(),
        blend: photocraft_doc::BlendMode::Normal,
        opacity: 1.0,
        visible: true,
    });
    doc.layers.push(Layer::new("smart", LayerContent::Smart(smart)));
    doc
}

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-manifest-limit-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn manifest_limit_is_inclusive_and_matches_reader() {
    let doc = document();
    let mut writer = PcraftWriter::new();
    let (_, stats) = writer.save_zip(&doc, &SaveOptions::default()).unwrap();
    writer.set_manifest_byte_limit(stats.manifest_bytes);
    let (bytes, _) = writer.save_zip(&doc, &SaveOptions::default()).unwrap();
    let limits = LoadOptions { max_manifest_bytes: stats.manifest_bytes, ..Default::default() };
    assert_eq!(load_from_bytes_with(&bytes, &limits).unwrap(), doc);
    writer.set_manifest_byte_limit(stats.manifest_bytes - 1);
    assert!(matches!(writer.save_zip(&doc, &SaveOptions::default()), Err(FormatError::LimitExceeded(_))));
}

fn check_preserved_save(directory: bool) {
    let tmp = Temp::new();
    let path = tmp.0.join("saved.pcraft");
    if directory {
        std::fs::create_dir(&path).unwrap();
    }
    let doc = document();
    let mut writer = PcraftWriter::new();
    let stats = writer.save_path(&doc, &path, &SaveOptions::default()).unwrap();
    let manifest_path = if directory { path.join("manifest.json") } else { path.clone() };
    let old_bytes = std::fs::read(&manifest_path).unwrap();
    writer.set_manifest_byte_limit(stats.manifest_bytes);
    let mut larger = doc.clone();
    larger.name.push_str(" extra bytes");
    assert!(matches!(writer.save_path(&larger, &path, &SaveOptions::default()), Err(FormatError::LimitExceeded(_))), "directory={directory}");
    assert_eq!(std::fs::read(&manifest_path).unwrap(), old_bytes, "failed save must preserve the previous file");
    assert_eq!(load_path(&path).unwrap(), doc);
    // A rejected save must leave the reusable writer usable too.
    writer.save_path(&doc, &path, &SaveOptions::default()).unwrap();
    assert_eq!(load_path(&path).unwrap(), doc);
}

#[test]
fn oversized_save_preserves_previous_zip() {
    check_preserved_save(false);
}

#[test]
fn oversized_save_preserves_previous_directory() {
    check_preserved_save(true);
}

#[test]
fn filter_params_keep_the_legacy_json_shape_and_roundtrip() {
    // This is the existing native SmartFilter JSON representation, before sharing was added.
    let legacy =
        json!({"command":"filter.distort.displace", "params":{"__displacePsd":"abcdef", "horizontal":10}, "blend":"Normal", "opacity":1.0, "visible":true});
    let filter: SmartFilter = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(&filter).unwrap(), legacy);
    let doc = document();
    let (bytes, _) = PcraftWriter::new().save_zip(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
}
