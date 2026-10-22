//! Small regression fixtures for #8266: snapshots must share and budget filter payloads.
use photocraft_doc::{ColorMode, Document, Layer, LayerContent, SampleType, Size, SmartFilter, SmartObject, SmartSource};
use photocraft_ops::{History, LayerTarget};
use serde_json::json;
use std::sync::Arc;

fn document() -> Document {
    let mut doc = Document::new("map", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
    let mut smart = SmartObject::new(SmartSource::Embedded { file_name: "fixture".into(), bytes: Arc::new(Vec::new()) }, Default::default(), None);
    smart.smart_filters.push(SmartFilter {
        command: "filter.distort.displace".into(),
        params: json!({"horizontal": 10, "__displacePsd": "ab".repeat(8192)}).into(),
        blend: photocraft_doc::BlendMode::Normal,
        opacity: 1.0,
        visible: true,
    });
    doc.layers.push(Layer::new("smart", LayerContent::Smart(smart)));
    doc
}

fn filter(doc: &Document) -> &SmartFilter {
    let LayerContent::Smart(smart) = &doc.layers[0].content else { panic!("fixture") };
    &smart.smart_filters[0]
}

fn payload(doc: &Document) -> &str {
    filter(doc).params["__displacePsd"].as_str().unwrap()
}

fn edit(history: &mut History, current: &mut Arc<Document>, change: impl FnOnce(&mut Document)) {
    let before = current.clone();
    let mut next = (**current).clone();
    change(&mut next);
    *current = Arc::new(next);
    history.record("edit", before, LayerTarget::default());
    history.trim(current);
}

#[test]
fn unrelated_edits_share_payload_allocation() {
    let mut current = Arc::new(document());
    let original = current.clone();
    let mut history = History::new(20);
    for i in 0..4 {
        edit(&mut history, &mut current, |d| d.name = format!("rename {i}"));
    }
    assert_eq!(history.past_len(), 4);
    for i in 0..4 {
        let old = history.state(i).unwrap();
        assert_eq!(payload(&old).as_ptr(), payload(&current).as_ptr(), "unrelated edits must not copy the embedded map");
    }
    assert_eq!(payload(&original).as_ptr(), payload(&current).as_ptr());
}

#[test]
fn shared_payload_counts_once_and_drives_trim() {
    let mut current = Arc::new(document());
    let mut history = History::new(20);
    let retained = history.pixel_bytes(&current);
    assert!(retained >= payload(&current).len(), "the history budget must include filter payloads");
    history.max_bytes = retained;
    for i in 0..4 {
        edit(&mut history, &mut current, |d| d.name = format!("rename {i}"));
        assert_eq!(history.pixel_bytes(&current), retained, "incremental counts must deduplicate shared params");
    }
    assert_eq!(history.past_len(), 4);
    assert_eq!(history.unique_bytes(&current), 0);
    history.max_bytes = 1;
    assert_eq!(history.trim(&current), 3, "over-budget maps must trigger the existing newest-state retention policy");
    assert_eq!(history.past_len(), 1);
    assert_eq!(history.pixel_bytes(&current), retained);
}

#[test]
fn replacement_undo_redo_and_pinned_snapshots_keep_the_right_payload() {
    let mut current = Arc::new(document());
    let pinned = current.clone();
    let mut history = History::new(20);
    let old_bytes = history.pixel_bytes(&current);
    assert!(old_bytes >= payload(&current).len());
    history.max_bytes = old_bytes * 4;
    edit(&mut history, &mut current, |d| d.name = "renamed".into());
    edit(&mut history, &mut current, |d| {
        let LayerContent::Smart(smart) = &mut d.layers[0].content else { panic!("fixture") };
        smart.smart_filters[0].params = json!({"horizontal": 20, "__displacePsd": "cd".repeat(4096)}).into();
    });
    let replacement = current.clone();
    let new_bytes = History::default().pixel_bytes(&replacement);
    assert_eq!(history.pixel_bytes(&current), old_bytes + new_bytes);
    assert_eq!(history.extra_bytes(&current, &[&pinned]), 0);
    current = history.undo(current).unwrap().0;
    assert_eq!(payload(&current), payload(&pinned));
    assert_eq!(history.pixel_bytes(&current), old_bytes + new_bytes, "redo retains the replacement");
    current = history.redo(current).unwrap().0;
    assert_eq!(payload(&current), payload(&replacement));
    assert_eq!(payload(&pinned), "ab".repeat(8192), "old snapshots remain unchanged");
    assert_eq!(history.unique_bytes_besides(&current, &[&pinned]), 0);
    history.clear();
    assert_eq!(history.pixel_bytes(&current), new_bytes);
    assert_eq!(history.extra_bytes(&current, &[&pinned]), old_bytes);
}

#[test]
fn aliased_filters_and_snapshots_are_counted_once() {
    let doc = document();
    let expected = History::default().pixel_bytes(&doc);
    assert!(expected >= payload(&doc).len());
    let mut aliased = doc.clone();
    let LayerContent::Smart(smart) = &mut aliased.layers[0].content else { panic!("fixture") };
    smart.smart_filters.push(smart.smart_filters[0].clone());
    let pinned = Arc::new(aliased.clone());
    let mut current = Arc::new(aliased);
    let mut history = History::new(20);
    history.max_bytes = expected;
    for i in 0..3 {
        edit(&mut history, &mut current, |d| d.name = format!("rename {i}"));
    }
    assert_eq!(history.pixel_bytes(&current), expected);
    assert_eq!(history.extra_bytes(&current, &[&pinned, &pinned]), 0);
    assert_eq!(history.trim_with(&current, &[&pinned]), 0);
    history.clear();
    let empty = Document::new("empty", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
    assert_eq!(history.extra_bytes(&empty, &[&pinned, &pinned]), expected);
}

#[test]
fn cleared_history_retains_filter_payloads_until_the_next_edit() {
    let mut current = Arc::new(document());
    let mut history = History::new(20);
    edit(&mut history, &mut current, |d| d.name = "renamed".into());
    edit(&mut history, &mut current, |d| {
        let LayerContent::Smart(smart) = &mut d.layers[0].content else { panic!("fixture") };
        smart.smart_filters[0].params = json!({"__displacePsd": "cd".repeat(4096)}).into();
    });
    let retained = history.pixel_bytes(&current);
    let unique = history.unique_bytes(&current);
    assert!(unique >= 16384, "old filter payload is retained by undo");
    assert!(history.clear_states());
    assert_eq!(history.pixel_bytes(&current), retained);
    assert_eq!(history.unique_bytes(&current), unique, "undoable Clear History still owns the old payload");
    let (restored, _) = history.undo(current.clone()).unwrap();
    current = restored;
    assert_eq!(history.pixel_bytes(&current), retained);
    assert!(history.clear_states());
    edit(&mut history, &mut current, |d| d.name = "after clear".into());
    assert_eq!(history.unique_bytes(&current), 0);
    assert_eq!(history.pixel_bytes(&current), History::default().pixel_bytes(&current));
}
