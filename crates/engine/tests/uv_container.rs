#![cfg(feature = "uv-workbench")]
use photocraft_engine::Session;
use serde_json::json;
fn fixture() -> Session {
    let mut s = Session::new();
    s.execute("uv.workspace.activate", json!({"active":true})).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj");
    s.execute("uv.model.import", json!({"path":source})).unwrap();
    s.execute("uv.texture.create", json!({"width":32,"height":32,"depth":16})).unwrap();
    s
}
#[test]
fn container_embeds_editable_maps_library_and_sky_without_exporting_sources() {
    let mut s = fixture();
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.png");
    s.active_mut().unwrap().saved_to(source.to_string_lossy().into_owned());
    s.execute("edit.fill", json!({"color":"#ff0000"})).unwrap();
    s.execute("file.uv.saveAllTextures", json!({})).unwrap();
    let original = std::fs::read(&source).unwrap();
    let asset = s.execute("uv.asset.import", json!({"path":source})).unwrap()["asset"].as_u64().unwrap();
    let before = s.active().unwrap().doc.layers.len();
    let result = s.execute("uv.asset.place", json!({"asset":asset})).unwrap();
    assert_eq!(result["smartObject"], false);
    assert_eq!(s.active().unwrap().doc.layers.len(), before + 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.len(), before);
    s.execute("edit.fill", json!({"color":"#00ff00"})).unwrap();
    let sky = temp.path().join("sky.hdr");
    let hdr = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 1 +X 1\n\x80\x80\x80\x81";
    std::fs::write(&sky, hdr).unwrap();
    let project = temp.path().join("scene.pcuv");
    s.execute("uv.project.save", json!({"path":project,"view":{"environment":sky}})).unwrap();
    assert!(!s.uv.has_unsaved_project(s.uv_documents()));
    assert!(!s.active().unwrap().is_dirty());
    assert!(s.uv.needs_export(s.active().unwrap()));
    assert!(std::fs::read(&source).unwrap() == original);
    // A project-only save is sufficient; remove all image dependencies before opening it.
    std::fs::remove_file(&source).unwrap();
    std::fs::remove_file(&sky).unwrap();
    let mut restored = Session::new();
    restored.execute("uv.workspace.activate", json!({"active":true})).unwrap();
    restored.execute("uv.project.load", json!({"path":project})).unwrap();
    assert!(!restored.uv.has_unsaved_project(restored.uv_documents()));
    assert!(!restored.active().unwrap().is_dirty());
    assert!(restored.uv.needs_export(restored.active().unwrap()));
    assert_eq!(restored.uv.assets.len(), 1);
    assert!(*restored.uv.assets[0].source.bytes == original);
    assert_eq!(restored.uv.environment.as_ref().unwrap().bytes.as_slice(), hdr);
    let px = photocraft_compose::render(&restored.active().unwrap().doc, photocraft_geom::Rect::from_xywh(2, 2, 1, 1)).px[0];
    assert!(px[1] > 0.99 && px[0] < 0.01);
    restored.execute("uv.project.save", json!({"path":project})).unwrap(); // Reuses embedded sky, not its missing path.
    restored.execute("uv.asset.place", json!({"asset":asset})).unwrap();
    assert!(restored.uv.has_unsaved_project(restored.uv_documents()));
    restored.execute("file.uv.saveAllTextures", json!({})).unwrap();
    assert!(restored.uv.has_unsaved_project(restored.uv_documents()), "export does not save layers/project edits");
    restored.execute("uv.project.save", json!({"path":project})).unwrap();
    assert!(!restored.uv.has_unsaved_project(restored.uv_documents()));
}
#[test]
fn document_banks_keep_identity_undo_and_normal_documents_when_project_closes() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":16,"height":16})).unwrap();
    let normal = s.active().unwrap().doc.id;
    s.execute("edit.fill", json!({"color":"#112233"})).unwrap();
    let history = s.active().unwrap().history.clone();
    s.execute("uv.workspace.activate", json!({"active":true})).unwrap();
    assert!(s.documents().is_empty());
    let mesh = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj");
    s.execute("uv.model.import", json!({"path":mesh})).unwrap();
    s.execute("uv.texture.create", json!({"width":32,"height":32})).unwrap();
    let texture = s.active().unwrap().doc.id;
    s.execute("edit.fill", json!({"color":"#ff0000"})).unwrap();
    for _ in 0..3 {
        s.execute("uv.workspace.activate", json!({"active":false})).unwrap();
        assert_eq!(s.documents().len(), 1);
        assert_eq!(s.active().unwrap().doc.id, normal);
        assert_eq!(s.active().unwrap().history.past_len(), history.past_len());
        assert!(s.uv_documents().iter().any(|st| st.doc.id == texture));
        s.execute("uv.workspace.activate", json!({"active":true})).unwrap();
        assert_eq!(s.active().unwrap().doc.id, texture);
    }
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("uv.project.close", json!({})).unwrap();
    s.execute("uv.workspace.activate", json!({"active":false})).unwrap();
    assert_eq!(s.active().unwrap().doc.id, normal);
}
#[test]
fn container_commands_reject_bad_resources_and_failed_save_keeps_work_dirty() {
    let mut s = fixture();
    for (id, p) in [
        ("uv.workspace.activate", json!({})),
        ("uv.workspace.activate", json!({"active":"yes"})),
        ("uv.asset.import", json!({})),
        ("uv.asset.import", json!({"path":4})),
        ("uv.asset.place", json!({})),
        ("uv.asset.place", json!({"asset":999})),
        ("uv.asset.remove", json!({"asset":999})),
    ] {
        assert!(s.execute(id, p).is_err(), "{id}");
    }
    let id = s.active().unwrap().doc.id;
    assert!(photocraft_engine::uv_asset_cmds::import_bytes(&mut s, "broken.png", vec![0; 32]).is_err());
    assert_eq!(s.active().unwrap().doc.id, id);
    assert!(s.uv.assets.is_empty());
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("scene.pcuv");
    s.execute("uv.project.save", json!({"path":project})).unwrap();
    s.execute("edit.fill", json!({"color":"#ff0000"})).unwrap();
    assert!(s.execute("uv.project.save", json!({"path":project.join("bad.pcuv")})).is_err());
    assert!(s.uv.has_unsaved_project(s.uv_documents()));
    assert!(s.active().unwrap().is_dirty());
}

#[test]
fn parked_uv_shells_are_untouched_by_normal_deselect_and_reload_does_not_accumulate_docs() {
    let mut s = fixture();
    s.execute("uv.shell.select", json!({"shells":[0]})).unwrap();
    s.execute("uv.mask.apply", json!({"guard":0})).unwrap();
    let selected = s.uv.selected.clone();
    s.execute("uv.workspace.activate", json!({"active":false})).unwrap();
    s.execute("file.new", json!({"width":16,"height":16})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    assert_eq!(s.uv.selected, selected);
    s.execute("uv.workspace.activate", json!({"active":true})).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("repeat.pcuv");
    s.execute("uv.project.save", json!({"path":project})).unwrap();
    let count = s.documents().len();
    for _ in 0..3 {
        s.execute("uv.project.load", json!({"path":project})).unwrap();
        assert_eq!(s.documents().len(), count);
        assert!(!s.uv.has_unsaved_project(s.uv_documents()));
    }
}

#[test]
fn restored_external_writeback_requires_explicit_rebinding() {
    use serde_json::json;
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("project");
    std::fs::create_dir(&dir).unwrap();
    let texture = root.path().join("map.png");
    let project = dir.join("scene.pcuv");
    let mut s = photocraft_engine::Session::new();
    let model = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj");
    s.execute("uv.model.import", json!({"path":model})).unwrap();
    s.execute("uv.texture.create", json!({"width":8,"height":8})).unwrap();
    s.active_mut().unwrap().saved_to(texture.to_string_lossy().into_owned());
    s.execute("uv.project.save", json!({"path":project})).unwrap();
    s.execute("uv.project.load", json!({"path":project})).unwrap();
    s.execute("edit.fill", json!({"color":"#ff0000"})).unwrap();
    assert_eq!(s.execute("file.uv.saveAllTextures", json!({})).unwrap()["saved"], 0);
    assert!(!texture.exists());
    s.execute("uv.texture.bind", json!({"material":0})).unwrap();
    assert_eq!(s.execute("file.uv.saveAllTextures", json!({})).unwrap()["saved"], 1);
    assert!(texture.is_file());
}
