//! A real Displace descriptor survives sharing, editing and native storage (#8266).
use photocraft_doc::{BlendMode, ColorMode, Document, Layer, LayerContent, SampleType, Size, SmartObject, SmartSource};
use photocraft_io::smart_map::{displace_embedded_map, filter_from_item, item_for_filter};
use photocraft_psd::descriptor::{Descriptor, Id, Value};
use serde_json::json;
use std::sync::Arc;

#[test]
fn embedded_displace_template_shares_and_roundtrips_without_losing_unknown_fields() {
    let side = 16u32;
    let mut map = Vec::new();
    map.extend(side.to_le_bytes());
    map.extend(side.to_le_bytes());
    map.extend(3u16.to_le_bytes());
    map.extend((0..side * side).map(|i| (i % 251) as u8));
    map.extend((0..side * side).map(|i| ((i * 3 + 7) % 251) as u8));
    let en = |t, v| Value::Enumerated { type_id: Id::new(t), value: Id::new(v) };
    let item = Descriptor::new("filterFX")
        .with("enab", Value::Boolean(true))
        .with("filterID", Value::Integer(1148416108))
        .with("unknown", Value::RawData(vec![7, 8, 9]))
        .with(
            "Fltr",
            Value::Descriptor(
                Descriptor::new("Dspl")
                    .with("HrzS", Value::Integer(10))
                    .with("VrtS", Value::Integer(10))
                    .with("DspM", en("DspM", "StrF"))
                    .with("UndA", en("UndA", "RptE"))
                    .with("EmbF", Value::Boolean(true))
                    .with("DspD", Value::RawData(map)),
            ),
        );
    let filter = filter_from_item(&item);
    assert_eq!(filter.command, "filter.distort.displace");
    let pixels = displace_embedded_map(&filter.params).unwrap();
    assert_eq!(pixels.data.len(), (side * side * 3) as usize);
    let mut edited = filter.clone();
    assert!(Arc::ptr_eq(&edited.params, &filter.params));
    Arc::make_mut(&mut edited.params)["horizontal"] = json!(25.0);
    edited.blend = BlendMode::Multiply;
    assert_eq!(filter.params["horizontal"].as_f64(), Some(10.0));
    assert_eq!(displace_embedded_map(&edited.params).unwrap().data, pixels.data);

    let mut doc = Document::new("Displace", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
    let mut smart = SmartObject::new(SmartSource::Embedded { file_name: "synthetic".into(), bytes: Arc::new(Vec::new()) }, Default::default(), None);
    smart.smart_filters = vec![filter, edited];
    doc.layers.push(Layer::new("smart", LayerContent::Smart(smart)));
    let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let loaded = photocraft_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(loaded, doc);
    let LayerContent::Smart(smart) = &loaded.layers[0].content else { panic!("fixture") };
    for f in &smart.smart_filters {
        let exported = item_for_filter(f).unwrap();
        assert_eq!(exported.get("unknown"), item.get("unknown"));
        let imported = filter_from_item(&exported);
        assert_eq!(imported.command, f.command);
        assert_eq!(imported.params["horizontal"], f.params["horizontal"]);
        assert_eq!((imported.blend, imported.opacity, imported.visible), (f.blend, f.opacity, f.visible));
        assert_eq!(displace_embedded_map(&f.params).unwrap().data, pixels.data);
    }
}
