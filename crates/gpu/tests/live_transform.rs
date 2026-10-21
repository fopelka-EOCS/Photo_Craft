//! A layer moving through a live transform (`photocraft_compose::live`) composites on the GPU as
//! on the CPU: in its place in the stack, with its blend mode, opacity, fill, unlinked mask and
//! clipping, under the layers above it.

use std::sync::Arc;

use photocraft_color::{BlendMode, Color, ColorMode, PixelFormat, SampleType};
use photocraft_compose::live::{self, LiveMesh, LiveTransform};
use photocraft_doc::{Document, Layer, LayerMask, Size};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

fn gpu() -> Option<(wgpu::Device, wgpu::Queue, photocraft_gpu::Compositor)> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
    if photocraft_gpu::Compositor::preferred_acc_format(&adapter) != wgpu::TextureFormat::Rgba32Float {
        return None;
    }
    let comp = photocraft_gpu::Compositor::try_new_with_format(&device, wgpu::TextureFormat::Rgba32Float).ok()?;
    Some((device, queue, comp))
}

/// A grid over `src` (grown by a texel, so the edge fades as a resampled layer's does) mapped
/// through `f` (source point → document point).
fn grid(src: Rect, n: usize, f: impl Fn(f32, f32) -> [f32; 2]) -> LiveMesh {
    let (x0, y0, x1, y1) = (src.x0 as f32 - 1.0, src.y0 as f32 - 1.0, src.x1 as f32 + 1.0, src.y1 as f32 + 1.0);
    let mut m = LiveMesh::default();
    for j in 0..=n {
        for i in 0..=n {
            let (u, v) = (x0 + (x1 - x0) * i as f32 / n as f32, y0 + (y1 - y0) * j as f32 / n as f32);
            m.pos.push(f(u, v));
            m.uv.push([u, v]);
        }
    }
    for j in 0..n {
        for i in 0..n {
            let a = (j * (n + 1) + i) as u32;
            let (b, c, d) = (a + 1, a + n as u32 + 2, a + n as u32 + 1);
            m.indices.extend_from_slice(&[a, b, c, a, c, d]);
        }
    }
    m
}

/// Turned by `deg` about (cx, cy), scaled by `s`, moved by (dx, dy).
fn turn(deg: f32, s: f32, c: [f32; 2], d: [f32; 2]) -> impl Fn(f32, f32) -> [f32; 2] {
    let (sin, cos) = deg.to_radians().sin_cos();
    move |x, y| {
        let (x, y) = ((x - c[0]) * s, (y - c[1]) * s);
        [c[0] + x * cos - y * sin + d[0], c[1] + x * sin + y * cos + d[1]]
    }
}

/// A layer with a diagonal colour ramp over `r`.
fn ramp(name: &str, fmt: PixelFormat, r: Rect) -> Layer {
    let mut s = Surface::new(fmt);
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let t = ((x - r.x0) as f32 / r.width() as f32 + (y - r.y0) as f32 / r.height() as f32) / 2.0;
            s.write_pixel(x, y, &photocraft_raster::from_rgba(&fmt, [t, 1.0 - t, 0.3, 1.0]));
        }
    }
    Layer::new(name, photocraft_doc::LayerContent::Raster(s))
}

/// The GPU render of `doc` prepared for `live` against the CPU render of it applied: within a
/// few 8-bit codes (hardware bilinear weights and the half-float source) everywhere.
fn check(g: &mut (wgpu::Device, wgpu::Queue, photocraft_gpu::Compositor), doc: &Document, live: &LiveTransform, what: &str) {
    let cpu = photocraft_compose::flatten(&live::apply(doc, live));
    let prepared = live::prepare(doc, live);
    g.2.set_live(Some((prepared.id, live.clone())));
    let out = photocraft_gpu::render_to_vec(&mut g.2, &g.0, &g.1, &prepared, prepared.bounds()).unwrap_or_else(|e| panic!("{what}: {e}"));
    g.2.set_live(None);
    let diff = |c: &[f32; 4], o: &[f32; 4]| (0..4).map(|k| if k == 3 { (c[3] - o[3]).abs() } else { (c[k] * c[3] - o[k] * o[3]).abs() }).fold(0.0f32, f32::max);
    let worst = cpu.px.iter().zip(&out).map(|(c, o)| diff(c, o)).fold(0.0f32, f32::max);
    assert!(worst <= 3.0 / 255.0, "{what}: max diff {:.2}/255", worst * 255.0);
    // The moving layer really moved: the GPU result differs from the untransformed document.
    let before = photocraft_compose::flatten(doc);
    assert!(before.px.iter().zip(&out).any(|(b, o)| diff(b, o) > 0.1), "{what}: nothing moved");
}

#[test]
fn a_moving_layer_composites_in_place_with_its_blend_mode() {
    let Some(mut g) = gpu() else { return };
    for depth in [SampleType::U8, SampleType::U16] {
        let mut d = Document::with_background("live", Size::new(96, 72), ColorMode::Rgb, depth, Color::WHITE);
        let fmt = d.pixel_format();
        d.layers.push(ramp("below", fmt, Rect::new(0, 0, 96, 36)));
        let mut moving = ramp("moving", fmt, Rect::new(20, 14, 60, 50));
        moving.blend = BlendMode::Difference;
        moving.opacity = 0.85;
        moving.fill_opacity = 0.9;
        let id = moving.id;
        let source = Arc::new(moving.surface().unwrap().clone());
        d.layers.push(moving);
        // Clipped to the moving layer: it must follow the moved pixels.
        let mut clip = ramp("clipped", fmt, Rect::new(0, 30, 96, 72));
        clip.clipped = true;
        clip.blend = BlendMode::Screen;
        d.layers.push(clip);
        // Above it all: it stays above the moving layer.
        let mut top = ramp("top", fmt, Rect::new(40, 0, 96, 72));
        top.blend = BlendMode::Multiply;
        top.opacity = 0.6;
        d.layers.push(top);
        let mesh = Arc::new(grid(Rect::new(20, 14, 60, 50), 8, turn(23.0, 1.3, [40.0, 32.0], [7.5, -4.0])));
        let live = LiveTransform { layer: id, source, rest: None, mesh };
        check(&mut g, &d, &live, &format!("{depth:?} difference, clipped and covered"));
    }
}

#[test]
fn a_selection_moves_over_the_pixels_that_stay_and_an_unlinked_mask_stays_put() {
    let Some(mut g) = gpu() else { return };
    let mut d = Document::with_background("sel", Size::new(80, 60), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let fmt = d.pixel_format();
    let mut l = ramp("l", fmt, Rect::new(10, 10, 70, 50));
    l.blend = BlendMode::Multiply;
    let mut m = LayerMask::reveal_all();
    m.surface.fill_rect(Rect::new(0, 0, 40, 60), &[0.25]);
    m.linked = false;
    l.mask = Some(m);
    let id = l.id;
    // The selected part (left half) moves; the right half stays.
    let full = l.surface().unwrap().clone();
    let mut source = Surface::new(fmt);
    source.write_region(Rect::new(10, 10, 40, 50), &full.read_region(Rect::new(10, 10, 40, 50)));
    let mut rest = Surface::new(fmt);
    rest.write_region(Rect::new(40, 10, 70, 50), &full.read_region(Rect::new(40, 10, 70, 50)));
    d.layers.push(l);
    let mesh = Arc::new(grid(Rect::new(10, 10, 40, 50), 4, turn(-12.0, 0.8, [25.0, 30.0], [22.0, 3.0])));
    let live = LiveTransform { layer: id, source: Arc::new(source), rest: Some(Arc::new(rest)), mesh };
    check(&mut g, &d, &live, "selection over the rest, unlinked mask");
}

/// A plain layer (nothing clipped, Normal, no mask), which the planner otherwise blends straight
/// from its own pixels in one pass (`Kernel::LayerBlend`), still draws the moved pixels, for 8, 16
/// and 32-bit documents.
#[test]
fn a_plain_layer_moves_too() {
    let Some(mut g) = gpu() else { return };
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut d = Document::with_background("plain", Size::new(64, 48), ColorMode::Rgb, depth, Color::WHITE);
        let fmt = d.pixel_format();
        let l = ramp("plain", fmt, Rect::new(8, 8, 40, 32));
        let id = l.id;
        let source = Arc::new(l.surface().unwrap().clone());
        d.layers.push(l);
        let mesh = Arc::new(grid(Rect::new(8, 8, 40, 32), 4, turn(10.0, 1.1, [24.0, 20.0], [12.0, 6.0])));
        let live = LiveTransform { layer: id, source, rest: None, mesh };
        check(&mut g, &d, &live, &format!("{depth:?} plain"));
    }
}

#[test]
fn a_styled_layer_or_pass_through_group_is_left_to_the_cpu() {
    let mut d = Document::with_background("fx", Size::new(32, 32), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let fmt = d.pixel_format();
    let mut l = ramp("fx", fmt, Rect::new(4, 4, 20, 20));
    l.effects.items = vec![photocraft_doc::Effect::default_drop_shadow()];
    let id = l.id;
    let source = Arc::new(l.surface().unwrap().clone());
    d.layers.push(l);
    let live = LiveTransform { layer: id, source, rest: None, mesh: Arc::new(grid(Rect::new(4, 4, 20, 20), 2, turn(0.0, 1.0, [0.0; 2], [3.0, 3.0]))) };
    let prepared = live::prepare(&d, &live);
    assert!(photocraft_gpu::plan::plan_with(&prepared, Some(&live)).is_err());
    assert!(photocraft_gpu::plan::plan_with(&prepared, None).is_ok());
}

/// `cargo test --release -p photocraft-gpu --test live_transform live_bench -- --ignored --nocapture`:
/// a 24 MP Difference layer turning a little each frame, composited on the GPU over a 1600×1000
/// view (with a readback, so an upper bound on a frame), against the CPU fallback.
#[test]
#[ignore]
fn live_bench() {
    let Some(mut g) = gpu() else { return };
    let (w, h) = (6000, 4000);
    let mut d = Document::with_background("bench", Size::new(w, h), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let fmt = d.pixel_format();
    let mut l = ramp("moving", fmt, Rect::new(0, 0, w as i32, h as i32));
    l.blend = BlendMode::Difference;
    let id = l.id;
    let source = Arc::new(l.surface().unwrap().clone());
    d.layers.push(l);
    d.layers.push(ramp("top", fmt, Rect::new(0, 0, w as i32, 600)));
    let view = Rect::new(2200, 1500, 3800, 2500);
    let mut gpu_ms = Vec::new();
    let mut cpu_ms = 0.0;
    let n = 12;
    for k in 0..n {
        let t0 = std::time::Instant::now();
        let mesh = Arc::new(grid(Rect::new(0, 0, w as i32, h as i32), 32, turn(0.2 * k as f32, 1.0, [3000.0, 2000.0], [0.0, 0.0])));
        let live = LiveTransform { layer: id, source: source.clone(), rest: None, mesh };
        let prepared = live::prepare(&d, &live);
        let prep = t0.elapsed().as_secs_f64() * 1e3;
        g.2.set_live(Some((prepared.id, live.clone())));
        let t0 = std::time::Instant::now();
        photocraft_gpu::render_to_vec(&mut g.2, &g.0, &g.1, &prepared, view).unwrap();
        gpu_ms.push(t0.elapsed().as_secs_f64() * 1e3 + prep);
        if k < 3 {
            let t0 = std::time::Instant::now();
            let _ = photocraft_compose::render(&live::apply(&d, &live), view);
            cpu_ms += t0.elapsed().as_secs_f64() * 1e3 / 3.0;
        }
    }
    let steady = gpu_ms[1..].iter().sum::<f64>() / (gpu_ms.len() - 1) as f64;
    eprintln!("live transform {w}x{h}, 1600x1000 view: GPU first {:.1} ms, steady {steady:.1} ms per frame; CPU fallback {cpu_ms:.1} ms", gpu_ms[0]);
}
