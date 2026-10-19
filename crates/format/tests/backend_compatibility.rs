mod common;

use std::{hint::black_box, time::Instant};

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_format::{SaveOptions, load_from_bytes, save_to_bytes};
use photocraft_geom::Size;
use photocraft_raster::Surface;

#[test]
fn cross_backend_documents() {
    let directory = std::env::var_os("PHOTOCRAFT_BACKEND_FIXTURES").map(std::path::PathBuf::from);
    let docs: Vec<_> = [SampleType::U8, SampleType::U16, SampleType::F32].into_iter().map(|depth| common::rich_doc(ColorMode::Rgb, depth)).collect();
    for doc in docs {
        let depth = doc.depth;
        let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
        assert!(load_from_bytes(&bytes).unwrap() == doc, "round trip: {depth:?}");
        if let Some(directory) = &directory {
            let backend = if cfg!(feature = "native-zstd") { "native" } else { "rust" };
            std::fs::create_dir_all(directory).unwrap();
            std::fs::write(directory.join(format!("{backend}-{depth:?}.pcraft")), bytes).unwrap();
            let other = if backend == "native" { "rust" } else { "native" };
            let path = directory.join(format!("{other}-{depth:?}.pcraft"));
            if path.exists() {
                assert!(load_from_bytes(&std::fs::read(path).unwrap()).unwrap() == doc, "cross-backend: {depth:?}");
            }
        }
    }
}

#[test]
#[ignore = "manual 24 MP release benchmark; compare default and native-zstd"]
fn benchmark_backend() {
    for pattern in ["ramp", "noise"] {
        let mut doc = Document::new("benchmark", Size::new(6000, 4000), ColorMode::Rgb, SampleType::U8);
        let mut seed = 0x9e3779b9u32;
        let raw: Vec<u8> = (0..96_000_000usize)
            .map(|i| {
                if i % 4 == 3 {
                    255
                } else if pattern == "noise" {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    (seed >> 24) as u8
                } else {
                    ((i / 4 / 6000 + i / 4 * 13 + i % 4 * 50) % 256) as u8
                }
            })
            .collect();
        doc.layers.push(Layer::new("pixels", LayerContent::Raster(Surface::from_interleaved(doc.pixel_format(), doc.bounds(), &raw))));
        drop(raw);
        let opts = SaveOptions::default();
        let bytes = save_to_bytes(&doc, &opts).unwrap();
        assert!(load_from_bytes(&bytes).unwrap() == doc);
        for trial in 0..6 {
            let start = Instant::now();
            let encoded = black_box(save_to_bytes(black_box(&doc), &opts).unwrap());
            println!("native,{pattern}-save,{trial},{:.3},{}", start.elapsed().as_secs_f64() * 1000.0, encoded.len());
            let start = Instant::now();
            black_box(load_from_bytes(black_box(&bytes)).unwrap());
            println!("native,{pattern}-load,{trial},{:.3},{}", start.elapsed().as_secs_f64() * 1000.0, bytes.len());
        }
    }
}
