//! Public PSD/PSB/layered-TIFF saves must carry the document folder into nested linked
//! sources while emitting complete current global text data at both document levels.
//! Every input is generated in a private temporary folder; no customer file or font is used.

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, SmartObject, SmartSource, TextLayer};
use photocraft_geom::{Affine, Size};
use photocraft_io::{ExportOptions, export, import, linked};
use photocraft_raster::Surface;
use photocraft_text::engine_data::Value as E;

static FIXTURE_ID: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    folder: PathBuf,
    document: Document,
    leaf_png: Vec<u8>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Only these two generated inputs are ever written. Never walk or recursively
        // remove a directory, even when a test fails halfway through fixture creation.
        for name in ["leaf.png", "linked.pcraft"] {
            let _ = std::fs::remove_file(self.folder.join(name));
        }
        let _ = std::fs::remove_dir(&self.folder);
    }
}

fn document(name: &str) -> Document {
    Document::new(name, Size::new(64, 32), ColorMode::Rgb, SampleType::U8)
}

fn pixels(doc: &Document) -> Surface {
    let mut surface = Surface::new(doc.pixel_format());
    surface.fill_rect(doc.bounds(), &[1.0, 0.0, 0.0, 1.0]);
    surface
}

fn add_text(doc: &mut Document, name: &str, value: &str) {
    doc.layers.push(Layer::new(
        name,
        LayerContent::Text(TextLayer {
            text: value.into(),
            font_family: "Inter".into(),
            size_pt: 12.0,
            transform: Affine::translate(4.0, 18.0),
            ..Default::default()
        }),
    ));
}

fn add_link(doc: &mut Document, name: &str, path: &str) {
    let cache = pixels(doc);
    let smart = SmartObject::new(SmartSource::Linked { path: path.into() }, Affine::IDENTITY, Some(cache));
    doc.layers.push(Layer::new(name, LayerContent::Smart(smart)));
}

fn fixture() -> Fixture {
    let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
    let folder = std::env::temp_dir().join(format!("photocraft-text-linked-{}-{id}", std::process::id()));
    // create_dir, rather than create_dir_all, refuses to reuse a stale folder.
    std::fs::create_dir(&folder).unwrap();
    let mut fixture = Fixture { folder, document: document("outer"), leaf_png: Vec::new() };

    let mut leaf = document("leaf");
    leaf.layers.push(Layer::new("red leaf", LayerContent::Raster(pixels(&leaf))));
    fixture.leaf_png = export(&leaf, "leaf.png", &Default::default()).unwrap().bytes;
    std::fs::write(fixture.folder.join("leaf.png"), &fixture.leaf_png).unwrap();

    let mut nested = document("linked");
    add_text(&mut nested, "nested type", "Nested v2");
    add_link(&mut nested, "relative leaf", "leaf.png");
    let bytes = photocraft_format::save_to_bytes(&nested, &Default::default()).unwrap();
    std::fs::write(fixture.folder.join("linked.pcraft"), bytes).unwrap();

    add_text(&mut fixture.document, "outer type", "Outer v2 \u{1f600}");
    add_link(&mut fixture.document, "relative source", "linked.pcraft");
    fixture
}

fn assert_current_type(doc: &Document, layer_name: &str, expected: &str) {
    let layer = doc.layers.iter().find(|l| l.name == layer_name).expect("live text layer kept");
    let LayerContent::Text(text) = &layer.content else { panic!("{layer_name} became pixels") };
    assert_eq!(text.text, expected);
    let raw = text.psd_raw.as_deref().expect("local TySh data kept");
    let index = photocraft_text::psd::text_index(raw).expect("local text object index") as usize;
    let data = doc.metadata.psd_global_blocks.iter().find(|(_, key, _)| key == b"Txt2").expect("complete global Txt2 kept");
    let global = photocraft_text::psd::parse_txt2(&data.2).expect("global Txt2 parses");
    let objects = global.path(&["1", "1"]).and_then(E::as_array).expect("global text objects");
    let value = objects.get(index).and_then(|o| o.path(&["0", "0"])).and_then(E::as_str);
    assert_eq!(value, Some(format!("{expected}\r").as_str()), "global text must describe the current local text");
}

fn embedded(doc: &Document, layer_name: &str) -> linked::LinkedFile {
    let layer = doc.layers.iter().find(|l| l.name == layer_name).expect("linked layer kept");
    let LayerContent::Smart(smart) = &layer.content else { panic!("{layer_name} was written as pixels") };
    let SmartSource::Linked { path } = &smart.source else { panic!("import must address its embedded source by UUID") };
    linked::find_linked_file(&doc.metadata, path).expect("embedded linked source bytes kept")
}

fn check(extension: &str) {
    let fixture = fixture();
    let name = format!("outer.{extension}");
    let options = ExportOptions {
        tiff_layers: extension == "tif",
        image_previews: false,
        document_path: Some(fixture.folder.join("outer.pcraft")),
        ..Default::default()
    };
    let saved = export(&fixture.document, &name, &options).unwrap();
    assert!(!saved.warnings.iter().any(|w| w.contains("written as pixels")), "{extension}: {:?}", saved.warnings);
    let outer = import(&name, &saved.bytes).unwrap().document;
    assert_current_type(&outer, "outer type", "Outer v2 \u{1f600}");

    // Losing opts.document_path in public PSD/PSB or layered TIFF export makes this
    // relative source a pixel fallback instead of an embedded, editable smart object.
    let source = embedded(&outer, "relative source");
    assert_eq!(source.file_name, "linked.psb");
    let nested = import(&source.file_name, &source.bytes).unwrap().document;
    assert_current_type(&nested, "nested type", "Nested v2");

    // Losing the path in convert_source's nested .pcraft -> PSB conversion makes
    // this second relative link fail even if the outer linked.pcraft was found.
    let leaf = embedded(&nested, "relative leaf");
    assert_eq!(leaf.file_name, "leaf.png");
    assert_eq!(leaf.bytes, fixture.leaf_png, "nested relative source must embed the actual PNG bytes");
}

#[test]
fn psd_keeps_current_text_and_nested_relative_linked_sources() {
    check("psd");
}

#[test]
fn psb_keeps_current_text_and_nested_relative_linked_sources() {
    check("psb");
}

#[test]
fn layered_tiff_keeps_current_text_and_nested_relative_linked_sources() {
    check("tif");
}
