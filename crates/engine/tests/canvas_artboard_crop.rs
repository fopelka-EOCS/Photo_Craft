use photocraft_doc::{Artboard, ColorMode, Document, Group, Layer, LayerContent, Rect, SampleType, Size};
use photocraft_engine::Session;
use serde_json::json;

/// Build a synthetic document whose artboards carry presets so crop preservation can be checked.
fn fixture(rects: &[Rect]) -> Session {
    let mut doc = Document::new("Synthetic canvas", Size::new(100, 80), ColorMode::Rgb, SampleType::U8);
    for rect in rects {
        let mut board = Artboard::new(*rect);
        board.preset = "original preset".into();
        doc.layers.push(Layer::new("Page", LayerContent::Group(Group { children: vec![], expanded: true, artboard: Some(board) })));
    }
    let mut session = Session::new();
    session.add_document(doc, None);
    session
}

/// A sole canvas board follows both crop modes and rotations, with exact undo and redo.
#[test]
fn canvas_artboard_follows_crop_and_undo_including_rotated_and_nondestructive_crops() {
    for delete in [false, true] {
        for angle in [0.0, 17.0] {
            let mut s = fixture(&[Rect::new(0, 0, 100, 80)]);
            let before = s.active().unwrap().doc.clone();
            s.execute("image.crop", json!({"x":20,"y":10,"width":60,"height":40,"angle":angle,"deleteCroppedPixels":delete})).unwrap();
            let doc = &s.active().unwrap().doc;
            assert_eq!(doc.size, Size::new(60, 40));
            assert_eq!(doc.artboards()[0].2.rect, doc.bounds());
            assert!(doc.artboards()[0].2.preset.is_empty());
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc, before);
            assert!(s.redo());
            assert_eq!(s.active().unwrap().doc.artboards()[0].2.rect, Rect::new(0, 0, 60, 40));
        }
    }
}

/// Independent boards retain their layout and preset when the canvas origin moves.
#[test]
fn crop_preserves_partial_and_multiple_artboard_layouts_and_presets() {
    for rects in [vec![Rect::new(10, 10, 50, 50)], vec![Rect::new(0, 0, 100, 80), Rect::new(60, 10, 90, 50)]] {
        let mut s = fixture(&rects);
        s.execute("image.crop", json!({"x":20,"y":10,"width":60,"height":40,"deleteCroppedPixels":false})).unwrap();
        for ((_, _, board), original) in s.active().unwrap().doc.artboards().iter().zip(rects) {
            assert_eq!(board.rect, original.translate(-20, -10));
            assert_eq!(board.preset, "original preset");
        }
    }
}

/// Trimming transparent margins fits the sole canvas board and remains reversible.
#[test]
fn trim_fits_a_canvas_artboard_to_visible_pixels() {
    let mut doc = Document::new("Trim", Size::new(100, 80), ColorMode::Rgb, SampleType::U8);
    let mut board = Artboard::new(doc.bounds());
    board.background = photocraft_doc::ArtboardBackground::Transparent;
    let mut surface = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    surface.write_region(Rect::new(20, 10, 80, 50), &[1.0, 0.0, 0.0, 1.0].repeat(60 * 40));
    doc.layers.push(Layer::new(
        "Page",
        LayerContent::Group(Group { children: vec![Layer::new("Paint", LayerContent::Raster(surface))], expanded: true, artboard: Some(board) }),
    ));
    let before = doc.clone();
    let mut s = Session::new();
    s.add_document(doc, None);
    s.execute("image.trim", json!({"basedOn":"transparent"})).unwrap();
    assert_eq!(s.active().unwrap().doc.bounds(), Rect::new(0, 0, 60, 40));
    assert_eq!(s.active().unwrap().doc.artboards()[0].2.rect, Rect::new(0, 0, 60, 40));
    assert!(s.undo());
    assert_eq!(*s.active().unwrap().doc, before);
}

/// Target-size resampling keeps the canvas board aligned with the final size and resolution.
#[test]
fn crop_target_resampling_keeps_the_page_board_fitted() {
    for angle in [0.0, 17.0] {
        let mut s = fixture(&[Rect::new(0, 0, 100, 80)]);
        s.execute(
            "image.crop",
            json!({"x":20,"y":10,"width":60,"height":40,"angle":angle,
            "targetWidth":30,"targetHeight":20,"resolution":144}),
        )
        .unwrap();
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.bounds(), Rect::new(0, 0, 30, 20));
        assert_eq!(doc.artboards()[0].2.rect, doc.bounds());
        assert_eq!(doc.resolution_dpi, 144.0);
    }
}
