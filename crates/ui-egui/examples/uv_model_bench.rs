//! Profile a user-supplied local model through the actual UI/texture refresh path.
//! Assets are read in place and are never bundled into the repository.
#[cfg(all(feature = "uv-workbench", not(target_arch = "wasm32")))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use photocraft_ui_egui::{PhotocraftApp, Services};
    use serde_json::json;
    let path = std::env::args().nth(1).ok_or("Supply a local model path")?;
    let output = std::env::args().nth(2);
    let mut session = photocraft_engine::Session::new();
    let start = std::time::Instant::now();
    session.execute("uv.model.import", json!({"path":path}))?;
    println!("Import: {:.2}s; triangles={}", start.elapsed().as_secs_f64(), session.uv.scene.as_ref().map_or(0, |s| s.triangles.len()));
    for doc in session.documents() {
        println!("Document: {} {}x{} {:?}", doc.doc.name, doc.doc.size.width, doc.doc.size.height, doc.doc.depth);
    }
    let capture = std::sync::Arc::new(std::sync::Mutex::new(None));
    let slot = capture.clone();
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1920.0, 1080.0))
        .with_pixels_per_point(1.0)
        .wgpu_setup(photocraft_ui_egui::gpu_canvas::wgpu_setup())
        .build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(session, Services::default());
            if let Some(rs) = &cc.wgpu_render_state {
                if let Ok(mut slot) = slot.lock() {
                    *slot = Some(rs.clone());
                }
                if let Err(error) = app.set_uv_wgpu(rs.clone()) {
                    app.ui.uv_workbench.status = error;
                }
            }
            app.ui.uv_workbench.view_3d = true;
            app.ui.uv_workbench.antialias = false;
            app
        });
    let rs = capture.lock().map_err(|_| "GPU capture lock")?.clone().ok_or("No wgpu render state")?;
    println!("Adapter: {:?}", rs.adapter.get_info());
    if rs.adapter.get_info().device_type == eframe::wgpu::DeviceType::Cpu {
        return Err("A hardware GPU is required for viewport performance measurements".into());
    }
    // kittest otherwise advertises a 2048px texture limit, even on a hardware adapter.
    harness.input_mut().max_texture_side = Some(rs.device.limits().max_texture_dimension_2d as usize);
    println!("Viewport backend texture limit: {}", rs.device.limits().max_texture_dimension_2d);
    for size in [egui::vec2(1920.0, 1080.0), egui::vec2(2560.0, 1440.0), egui::vec2(3840.0, 2160.0)] {
        harness.set_size(size);
        let mut times = Vec::new();
        for frame in 0..40 {
            harness.state_mut().ui.uv_workbench.yaw += 0.015;
            let start = std::time::Instant::now();
            harness.step();
            rs.device.poll(eframe::wgpu::PollType::Wait { submission_index: None, timeout: None })?;
            if !harness.state().uv_gpu_active() {
                return Err(format!("GPU fallback: {}", harness.state().ui.uv_workbench.status).into());
            }
            if frame >= 8 {
                times.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        times.sort_by(f64::total_cmp);
        println!("Window {}x{} UI+completed GPU: median {:.2} ms; p95 {:.2} ms", size.x, size.y, times[times.len() / 2], times[times.len() * 95 / 100]);
    }
    // Real Maya Alt+LMB input, including switching from an idle antialiased frame.
    harness.state_mut().ui.uv_workbench.fast_navigation = true;
    harness.state_mut().ui.uv_workbench.antialias = true;
    harness.step();
    rs.device.poll(eframe::wgpu::PollType::Wait { submission_index: None, timeout: None })?;
    let alt = egui::Modifiers { alt: true, ..Default::default() };
    harness.event(egui::Event::ModifiersChanged(alt));
    harness.event(egui::Event::PointerMoved(egui::pos2(600.0, 500.0)));
    harness.event(egui::Event::PointerButton { pos: egui::pos2(600.0, 500.0), button: egui::PointerButton::Primary, pressed: true, modifiers: alt });
    harness.step();
    let mut drag = Vec::new();
    let yaw = harness.state().ui.uv_workbench.yaw;
    for frame in 0..40 {
        harness.event(egui::Event::PointerMoved(egui::pos2(602.0 + frame as f32 * 2.0, 500.0)));
        let start = std::time::Instant::now();
        harness.step();
        rs.device.poll(eframe::wgpu::PollType::Wait { submission_index: None, timeout: None })?;
        if frame >= 8 {
            drag.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    if (harness.state().ui.uv_workbench.yaw - yaw).abs() < 0.1 {
        return Err("Maya drag did not rotate the model".into());
    }
    drag.sort_by(f64::total_cmp);
    println!("4K window, fast Maya Alt+LMB (1600px frame): median {:.2} ms; p95 {:.2} ms", drag[drag.len() / 2], drag[drag.len() * 95 / 100]);
    harness.event(egui::Event::PointerButton { pos: egui::pos2(680.0, 500.0), button: egui::PointerButton::Primary, pressed: false, modifiers: alt });
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::default()));
    harness.step();
    for (material, (color, normal)) in harness.state().uv_gpu_texture_sizes().iter().enumerate() {
        println!("GPU material {material}: color={color:?} normal={normal:?}");
        for (size, id) in [(*color, harness.state().session.uv.bindings.get(material)), (*normal, harness.state().session.uv.normal_bindings.get(material))] {
            if let Some(Some(id)) = id
                && let Some(doc) = harness.state().session.documents().iter().find(|st| st.doc.id == *id)
                && size != [doc.doc.size.width, doc.doc.size.height]
            {
                return Err("Resident texture was resized".into());
            }
        }
    }
    if let Some(output) = output {
        harness.render()?.save(&output)?;
        println!("Saved local screenshot: {output}");
    }
    Ok(())
}
#[cfg(any(not(feature = "uv-workbench"), target_arch = "wasm32"))]
fn main() {
    eprintln!("Native build with uv-workbench required");
}
