//! Visible-surface picking maps native brush events into the bound texture document.
use crate::{
    PhotocraftApp,
    canvas::{self, ToolEvent},
    state::Tool,
};
use egui::{Modifiers, Pos2, Response};
use photocraft_uv::{
    Scene,
    prepared::Geometry,
    preview::{Camera, Materials},
};
use serde_json::json;

#[derive(Clone, Copy)]
struct Sample {
    triangle: usize,
    shell: u32,
    point: [f64; 2],
}
#[derive(Default)]
pub(crate) struct State {
    held: bool,
    wrapped: bool,
    material: Option<usize>,
    doc: Option<photocraft_doc::DocId>,
    tool: Option<Tool>,
    last: Option<Sample>,
    screen: Option<Pos2>,
}
pub(crate) fn pending(state: &State) -> bool {
    state.doc.is_some() && state.last.is_some()
}
fn mods() -> Modifiers {
    Modifiers::NONE
}
fn end_segment(app: &mut PhotocraftApp, state: &mut State) {
    if let Some(last) = state.last.take()
        && app.drag.is_some()
    {
        if app.session.active().is_none_or(|st| Some(st.doc.id) != state.doc) {
            app.drag = None;
            app.live_stroke = None;
            app.session.clear_live_stroke();
            return;
        }
        let deferred = std::mem::replace(&mut app.defer_live_stroke, false);
        canvas::feed_live_stroke(app);
        app.defer_live_stroke = deferred;
        canvas::tool_event(app, ToolEvent::Up { x: last.point[0], y: last.point[1] }, mods());
    }
}
pub(crate) fn finish(app: &mut PhotocraftApp, state: &mut State) {
    end_segment(app, state);
    if state.wrapped {
        let _ = app.run("paint.uvWrap", json!({"end":true}));
    }
    *state = State::default();
    app.uv_paint_key = None;
}
fn adjacent(scene: &Scene, a: usize, b: usize) -> bool {
    if a == b {
        return true;
    }
    let (Some(a), Some(b)) = (scene.triangles.get(a), scene.triangles.get(b)) else { return false };
    a.corners.iter().filter(|a| b.corners.iter().any(|b| a.vertex == b.vertex && a.uv.iter().zip(b.uv).all(|(a, b)| (a - b).abs() < 1e-6))).count() >= 2
}
struct View<'a> {
    wrap: bool,
    geometry: &'a Geometry,
    scene: &'a Scene,
    camera: Camera,
    maps: &'a Materials<'a>,
    size: [u32; 2],
    rect: egui::Rect,
}
fn sample(app: &mut PhotocraftApp, state: &mut State, view: &View<'_>, p: Pos2, source: bool) {
    let at = (p - view.rect.min) / view.rect.size();
    let hit = view.rect.contains(p).then(|| view.geometry.hit(view.scene, view.camera, view.maps, [f64::from(at.x), f64::from(at.y)], view.size)).flatten();
    let Some(hit) = hit else {
        end_segment(app, state);
        return;
    };
    let Some(triangle) = view.geometry.triangles.get(hit.triangle) else { return };
    if hit.uv.iter().any(|v| !(0.0..=1.0).contains(v)) {
        end_segment(app, state);
        return;
    }
    let material = triangle.material;
    // One physical gesture targets one material/document, so it has one predictable Undo.
    if state.material.is_some_and(|m| m != material) {
        end_segment(app, state);
        return;
    }
    let bindings = app.session.uv.map_bindings(app.session.uv.current_channel());
    let Some(id) = bindings.get(material).copied().flatten() else {
        app.ui.status = tl!("Assign a texture to this material before painting").into();
        end_segment(app, state);
        return;
    };
    if state.material.is_none() {
        let changed = app.session.active().is_none_or(|st| st.doc.id != id);
        if app.run("uv.material.select", json!({"material":material})).is_err() {
            return;
        }
        if changed {
            use photocraft_engine::channel_cmds::ChannelTarget;
            app.ui.mask_target = app.session.active().is_some_and(|st| st.channel_view.target == ChannelTarget::LayerMask);
            app.ui.vector_mask_target = app.session.active().is_some_and(|st| st.channel_view.target == ChannelTarget::VectorMask);
            app.sync_mask_targets();
        }
        state.material = Some(material);
        state.doc = Some(id);
    }
    if app.session.active().is_none_or(|st| Some(st.doc.id) != state.doc) {
        finish(app, state);
        return;
    }
    let Some(st) = app.session.active() else { return };
    let point = [
        (hit.uv[0] * f64::from(st.doc.size.width)).min(f64::from(st.doc.size.width) - 1e-6),
        ((1.0 - hit.uv[1]) * f64::from(st.doc.size.height)).min(f64::from(st.doc.size.height) - 1e-6),
    ];
    if view.wrap && matches!(app.active_tool(), Tool::Brush | Tool::Pencil | Tool::Eraser) && !source {
        end_segment(app, state);
        let tool = app.active_tool();
        let target = canvas::paint_target(app);
        let result = app.run("paint.uvWrap", json!({"triangle":hit.triangle,"position":hit.position,"pressure":app.stylus.pressure(),"erase":tool == Tool::Eraser,"pencil":tool == Tool::Pencil || crate::eraser_ui::pencil_mode(app, tool),"target":target}));
        if let Err(error) = result {
            app.ui.status = error;
            app.ui.status_error = true;
        }
        state.wrapped = true;
        return;
    }
    if state.last.is_some_and(|last| {
        last.shell != triangle.shell
            || (!adjacent(view.scene, last.triangle, hit.triangle)
                && (last.point[0] - point[0]).hypot(last.point[1] - point[1]) > f64::from(st.doc.size.width.max(st.doc.size.height)) * 0.02)
    }) {
        end_segment(app, state);
    }
    let pressure = app.stylus.pressure();
    if source {
        crate::retouch_ui::set_source(app, point[0], point[1]);
        return;
    }
    let event = if state.last.is_some() && app.drag.is_some() {
        ToolEvent::Move { x: point[0], y: point[1], pressure }
    } else {
        ToolEvent::Down { x: point[0], y: point[1], pressure }
    };
    canvas::tool_event(app, event, mods());
    state.last = app.drag.is_some().then_some(Sample { triangle: hit.triangle, shell: triangle.shell, point });
}
/// Ctrl+RMB owns its drag until release, including after Ctrl is released.
pub(crate) fn resize_brush(app: &mut PhotocraftApp, ui: &egui::Ui, response: &Response, enabled: bool) -> bool {
    let mut handled = app.brush_resize.is_some_and(|r| r.screen);
    if !enabled
        || app.ui.brush_picker.is_some()
        || !crate::brush_resize::applies(app.active_tool())
        || !ui.input(|i| i.focused)
        || !crate::menus::modal_allows(app, "paint.stroke")
        || app.ui.transform.is_some()
    {
        if handled {
            app.brush_resize = None;
        }
        return handled;
    }
    crate::paint_mouse::sync_tool_brush(app);
    for event in ui.input(|i| i.events.clone()) {
        let active = app.brush_resize.is_some_and(|r| r.screen);
        match event {
            egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed: true, modifiers }
                if modifiers.ctrl
                    && !modifiers.alt
                    && response.rect.contains(pos)
                    && ui.ctx().layer_id_at(pos).is_none_or(|layer| layer == ui.layer_id())
                    && app.drag.is_none() =>
            {
                handled |=
                    crate::brush_resize::pointer_screen(app, ToolEvent::Down { x: f64::from(pos.x), y: f64::from(pos.y), pressure: 1.0 }, modifiers, true);
            }
            egui::Event::PointerMoved(pos) if active => {
                crate::brush_resize::pointer_screen(app, ToolEvent::Move { x: f64::from(pos.x), y: f64::from(pos.y), pressure: 1.0 }, Modifiers::NONE, false);
                handled = true;
            }
            egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed: false, .. } if active => {
                crate::brush_resize::pointer_screen(app, ToolEvent::Up { x: f64::from(pos.x), y: f64::from(pos.y) }, Modifiers::NONE, false);
                handled = true;
            }
            egui::Event::PointerGone if active => {
                app.brush_resize = None;
                handled = true;
            }
            _ => {}
        }
    }
    if app.brush_resize.is_some_and(|r| r.screen) && !ui.input(|i| i.pointer.secondary_down()) {
        app.brush_resize = None;
    }
    handled
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn input(
    app: &mut PhotocraftApp,
    ui: &egui::Ui,
    response: &Response,
    state: &mut State,
    geometry: &Geometry,
    scene: &Scene,
    camera: Camera,
    maps: &Materials<'_>,
    size: [u32; 2],
    enabled: bool,
    wrap: bool,
) {
    app.stylus.use_pressure = app.session.prefs().tools.use_tablet_pressure;
    app.stylus.set_pressure_curve(&app.session.prefs().tools.pressure_curve.clone());
    app.stylus.update_for_frame(ui.ctx().cumulative_frame_nr(), &ui.input(|i| i.events.clone()));
    crate::stylus::Stylus::sync_eraser_tool(app);
    let tool = if ui.input(|i| i.modifiers.ctrl) && matches!(app.ui.tool, Tool::CloneStamp | Tool::Healing) { app.ui.tool } else { app.active_tool() };
    if app.brush_resize.is_some_and(|r| r.screen) {
        finish(app, state);
        let view = View { wrap, geometry, scene, camera, maps, size, rect: response.rect };
        cursor(app, ui, &view);
        if let Some(at) = ui.input(|i| i.pointer.latest_pos()) {
            canvas::draw_readout(ui.ctx(), "uv-brush-size", at, [tl!("Size:")], [format!("{} px", app.session.tools.brush.size.round())]);
        }
        return;
    }
    if !enabled
        || app.ui.brush_picker.is_some()
        || !crate::menus::modal_allows(app, "paint.stroke")
        || app.ui.transform.is_some()
        || (!canvas::strokes_live(tool) && tool != Tool::Eyedropper)
        || state.tool.is_some_and(|old| old != tool)
        || ui.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused)
    {
        finish(app, state);
        return;
    }
    let view = View { wrap, geometry, scene, camera, maps, size, rect: response.rect };
    crate::paint_mouse::sync_tool_brush(app);
    let events = ui.input(|i| i.events.clone());
    let deferred = app.defer_live_stroke;
    app.defer_live_stroke = true;
    for event in events {
        match event {
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers }
                if !modifiers.alt && response.contains_pointer() =>
            {
                finish(app, state);
                static GESTURES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                app.uv_paint_key = Some(format!("uv3d-{}", GESTURES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
                state.held = true;
                state.tool = Some(tool);
                state.screen = Some(pos);
                let source = modifiers.ctrl && matches!(tool, Tool::CloneStamp | Tool::Healing);
                sample(app, state, &view, pos, source);
                if source {
                    finish(app, state);
                }
            }
            egui::Event::PointerMoved(pos) if state.held => {
                let previous = state.screen.replace(pos).unwrap_or(pos);
                let steps = ((pos - previous).length() / 1.5).ceil().clamp(1.0, 128.0) as usize;
                for i in 1..=steps {
                    sample(app, state, &view, previous.lerp(pos, i as f32 / steps as f32), false);
                }
            }
            egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: false, .. } => finish(app, state),
            egui::Event::PointerGone => finish(app, state),
            _ => {}
        }
    }
    if state.held && !ui.input(|i| i.pointer.primary_down()) {
        finish(app, state);
    }
    if state.held
        && !state.wrapped
        && let Some(last) = state.last
    {
        canvas::tool_event(app, ToolEvent::Move { x: last.point[0], y: last.point[1], pressure: app.stylus.pressure() }, mods());
    }
    app.defer_live_stroke = deferred;
    if !deferred {
        canvas::feed_live_stroke(app);
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        if tool != Tool::Eyedropper {
            cursor(app, ui, &view);
            if tool == Tool::CloneStamp {
                crate::uv_clone::preview(app, ui, geometry, scene, camera, maps, size, response.rect);
            }
        }
    }
    if state.held {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }
}

fn cursor(app: &PhotocraftApp, ui: &egui::Ui, view: &View<'_>) {
    let Some(p) =
        app.brush_resize.filter(|r| r.screen).map(|r| egui::pos2(r.anchor[0] as f32, r.anchor[1] as f32)).or_else(|| ui.input(|i| i.pointer.hover_pos()))
    else {
        return;
    };
    let at = (p - view.rect.min) / view.rect.size();
    let Some(hit) = view.geometry.hit(view.scene, view.camera, view.maps, [f64::from(at.x), f64::from(at.y)], view.size) else { return };
    let Some(t) = view.geometry.triangles.get(hit.triangle) else { return };
    let bindings = app.session.uv.map_bindings(app.session.uv.current_channel());
    let Some(st) = bindings.get(t.material).copied().flatten().and_then(|id| app.session.documents().iter().find(|st| st.doc.id == id)) else { return };
    let tex = t.uv.map(|uv| [f64::from(uv[0]) * f64::from(st.doc.size.width), -f64::from(uv[1]) * f64::from(st.doc.size.height)]);
    let [u, v] = [0, 1].map(|axis| [tex[1][axis] - tex[0][axis], tex[2][axis] - tex[0][axis]]);
    let det = u[0] * v[1] - u[1] * v[0];
    if det.abs() < 1e-12 {
        return;
    }
    if view.wrap && matches!(app.active_tool(), Tool::Brush | Tool::Pencil | Tool::Eraser) {
        let sub = |a: [f64; 3], b: [f64; 3]| std::array::from_fn::<_, 3, _>(|i| a[i] - b[i]);
        let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
        let norm = |a: [f64; 3]| {
            let n = a.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-20);
            a.map(|v| v / n)
        };
        let positions = t.positions.map(|p| p.map(f64::from));
        let area = cross(sub(positions[1], positions[0]), sub(positions[2], positions[0])).iter().map(|v| v * v).sum::<f64>().sqrt();
        let scale = app.session.uv.wrap_pixels_per_unit().map(|unit| unit * view.geometry.diameter).unwrap_or_else(|| (det.abs() / area.max(1e-24)).sqrt());
        let radius = f64::from(app.session.tools.brush.size) * 0.5 / scale.max(1e-20);
        let u = norm(t.tangent.map(f64::from));
        let v = cross(t.face.map(f64::from), u).map(|v| v * det.signum());
        let center = std::array::from_fn::<_, 3, _>(|i| (hit.position[i] - view.geometry.center[i]) / view.geometry.diameter);
        let points: Option<Vec<_>> = (0..48)
            .map(|i| {
                let angle = f64::from(i) * std::f64::consts::TAU / 48.0;
                let p = std::array::from_fn(|j| center[j] + radius * (u[j] * angle.cos() + v[j] * angle.sin()));
                let q = view.geometry.project_normalized(view.camera, p, view.size)?;
                Some(view.rect.min + egui::vec2(q[0] as f32 * view.rect.width(), q[1] as f32 * view.rect.height()))
            })
            .collect();
        if let Some(points) = points {
            paint_cursor_ring(ui, view, p, points);
        }
        return;
    }
    let screen = t.positions.map(|p| view.geometry.project_normalized(view.camera, p.map(f64::from), view.size));
    let [Some(a), Some(b), Some(c)] = screen else {
        return;
    };
    let screen = [a, b, c].map(|p| [p[0] * f64::from(view.rect.width()), p[1] * f64::from(view.rect.height())]);
    let radius = f64::from(app.session.tools.brush.size) * 0.5;
    let points: Vec<_> = (0..48)
        .map(|i| {
            let a = f64::from(i) * std::f64::consts::TAU / 48.0;
            let (x, y) = (a.cos() * radius, a.sin() * radius);
            let (a, b) = ((x * v[1] - y * u[1]) / det, (y * u[0] - x * v[0]) / det);
            p + egui::vec2(
                (a * (screen[1][0] - screen[0][0]) + b * (screen[2][0] - screen[0][0])) as f32,
                (a * (screen[1][1] - screen[0][1]) + b * (screen[2][1] - screen[0][1])) as f32,
            )
        })
        .collect();
    paint_cursor_ring(ui, view, p, points);
}
fn paint_cursor_ring(ui: &egui::Ui, view: &View<'_>, p: Pos2, points: Vec<Pos2>) {
    if points.iter().any(|q| !q.is_finite() || q.distance(p) > 2048.0) {
        return;
    }
    let painter = ui.painter().with_clip_rect(view.rect);
    painter.add(egui::Shape::closed_line(points.clone(), egui::Stroke::new(2.5, egui::Color32::BLACK)));
    painter.add(egui::Shape::closed_line(points, egui::Stroke::new(1.0, egui::Color32::WHITE)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_uv::prepared::rotate;
    fn setup(tool: Tool) -> (PhotocraftApp, std::sync::Arc<Scene>, Geometry) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj");
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("uv.model.import", json!({"path":path})).unwrap();
        app.run("uv.texture.create", json!({"width":128,"height":128})).unwrap();
        app.run("tools.setColors", json!({"foreground":[1.0,0.0,0.0,1.0]})).unwrap();
        app.run("tools.setBrush", json!({"brush":{"size":6.0,"hardness":1.0}})).unwrap();
        app.ui.tool = tool;
        let scene = app.session.uv.scene.clone().unwrap();
        let geometry = Geometry::new(&scene, app.session.uv.topology.as_ref().unwrap()).unwrap();
        (app, scene, geometry)
    }
    fn screen(view: &View<'_>, tri: usize) -> Pos2 {
        let t = &view.geometry.triangles[tri];
        let p: [f64; 3] = std::array::from_fn(|i| t.positions.iter().map(|p| f64::from(p[i])).sum::<f64>() / 3.0);
        let p = rotate(p.into(), view.camera);
        let c = view.geometry.camera_values(view.camera, view.size);
        view.rect.min
            + egui::vec2(
                ((p.x + f64::from(c[6])) * f64::from(c[4]) + 1.0) as f32 * 0.5 * view.rect.width(),
                (1.0 - (p.y + f64::from(c[7])) * f64::from(c[5])) as f32 * 0.5 * view.rect.height(),
            )
    }
    fn make_view<'a>(scene: &'a Scene, g: &'a Geometry, maps: &'a Materials<'a>) -> View<'a> {
        View {
            wrap: false,
            geometry: g,
            scene,
            camera: Camera { yaw: 0.0, pitch: 0.0, ..Default::default() },
            maps,
            size: [600, 400],
            rect: egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 400.0)),
        }
    }

    #[test]
    fn clone_takes_source_from_3d_despite_temporary_ctrl_move_and_generates_native_overlay() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (mut app, scene, g) = setup(Tool::CloneStamp);
        let maps = Materials::default();
        let view = make_view(&scene, &g, &maps);
        let at = screen(&view, 0);
        app.tool_override = Some(Tool::Move);
        sample(&mut app, &mut State::default(), &view, at, true);
        assert!(app.session.presets.clone.active().source.is_some());
        assert!(app.drag.is_none());
        app.tool_override = None;
        let source = app.session.presets.clone.active().source.unwrap();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            assert!(crate::canvas::clone_preview_texture(&mut app, ui.painter(), source, None).is_some());
        });
        output.textures_delta.clear();
        let before = app.session.active().unwrap().history.past_len();
        let mut paint = State::default();
        sample(&mut app, &mut paint, &view, at, false);
        finish(&mut app, &mut paint);
        assert!(app.session.active().unwrap().history.past_len() <= before + 1);
    }

    #[test]
    fn uv_clone_preview_projects_to_visible_model_and_ctrl_click_samples() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (mut app, scene, g) = setup(Tool::CloneStamp);
        app.ui.workspace = "3D".into();
        app.run("tools.setBrush", json!({"brush":{"size":36.0,"hardness":1.0}})).unwrap();
        app.run("select.rect", json!({"x":0,"y":0,"width":32,"height":32})).unwrap();
        app.run("edit.fill", json!({"color":"#00ff00"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        crate::retouch_ui::set_source(&mut app, 16.0, 16.0);
        app.ui.uv_workbench.split_view = true;
        app.ui.uv_workbench.view_3d = true;
        app.ui.uv_workbench.channel = photocraft_uv::preview::Channel::Color;
        app.ui.uv_workbench.yaw = 0.0;
        app.ui.uv_workbench.pitch = 0.0;
        let maps = Materials::default();
        let view = make_view(&scene, &g, &maps);
        let local = screen(&view, 0);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 900.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            app.set_wgpu(cc.wgpu_render_state.clone().unwrap());
            app
        });
        h.step();
        h.step();
        let rect = h.state().uv_cache.viewport.unwrap();
        let p = rect.min + egui::vec2(local.x / 600.0 * rect.width(), local.y / 400.0 * rect.height());
        h.event(egui::Event::PointerMoved(p));
        h.step();
        // Native source preview is generated while hovering over the visible mesh.
        assert!(h.state().clone_preview.is_some());
        let ctrl = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
        h.event(egui::Event::ModifiersChanged(ctrl));
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: ctrl });
        h.step();
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: ctrl });
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        h.step();
        assert_ne!(h.state().session.presets.clone.active().source.unwrap(), [16.0, 16.0]);
        assert!(h.state().drag.is_none());
        crate::retouch_ui::set_source(h.state_mut(), 16.0, 16.0);
        h.step();
        if let Ok(path) = std::env::var("PHOTOCRAFT_UV_CLONE_SCREENSHOT") {
            // Documentation captures must not expose the checkout's absolute user path.
            let st = h.state_mut().session.active_mut().unwrap();
            st.path = st.path.as_deref().and_then(|p| std::path::Path::new(p).file_name()).map(|name| name.to_string_lossy().into_owned());
            h.run_steps(2);
            h.render().unwrap().save(path).unwrap();
        }
    }

    #[test]
    #[ignore = "explicit seam-crossing 4K native paint benchmark"]
    fn benchmark_uv_seam_4k_refresh() {
        let _gpu = photocraft_testkit::gpu_lock();
        for wrap in [false, true] {
            let (mut app, scene, g) = setup(Tool::Brush);
            app.run("uv.texture.create", json!({"width":4096,"height":4096,"depth":16})).unwrap();
            app.run("tools.setBrush", json!({"brush":{"size":32.0,"hardness":0.5}})).unwrap();
            let maps = Materials::default();
            let mut view = make_view(&scene, &g, &maps);
            view.wrap = wrap;
            let mut cache = crate::uv_lighting::SurfaceMaps::default();
            crate::uv_lighting::refresh(&mut app, &mut cache).unwrap();
            let material = app.session.uv.material;
            let mut paint = State::default();
            app.uv_paint_key = Some("seam-benchmark".into());
            let mut elapsed = Vec::new();
            let mut largest = 0;
            for frame in 0..8 {
                let begin = std::time::Instant::now();
                app.defer_live_stroke = true;
                for i in 0..4 {
                    let x = if i % 2 == 0 { -0.003 } else { 0.003 };
                    let world = [x, frame as f64 * 0.002, if x > 0.0 { x * 0.6 } else { 0.0 }];
                    let normalized = std::array::from_fn(|j| (world[j] - g.center[j]) / g.diameter);
                    let at = g.project_normalized(view.camera, normalized, view.size).unwrap();
                    let p = view.rect.min + egui::vec2(at[0] as f32 * view.rect.width(), at[1] as f32 * view.rect.height());
                    sample(&mut app, &mut paint, &view, p, false);
                }
                app.defer_live_stroke = false;
                canvas::feed_live_stroke(&mut app);
                crate::uv_lighting::refresh(&mut app, &mut cache).unwrap();
                let pixels = cache.regions[0][material].as_ref().map_or(0, |(_, r)| r.iter().map(|r| u64::from(r[2]) * u64::from(r[3])).sum::<u64>());
                largest = largest.max(pixels);
                elapsed.push(begin.elapsed().as_secs_f64() * 1000.0);
            }
            finish(&mut app, &mut paint);
            assert!(largest > 0 && largest < 1_048_576, "seam must edit both islands without full-map refresh: {largest}");
            println!(
                "4K {} seam crossing, 4 samples/frame: {:.2} ms average, {:.2} ms max; max {} recomposited texels",
                if wrap { "Paint Wrap" } else { "Paint UV" },
                elapsed.iter().sum::<f64>() / elapsed.len() as f64,
                elapsed.iter().copied().fold(0.0, f64::max),
                largest
            );
        }
    }
    #[test]
    fn eyedropper_samples_bound_surface_and_first_undo_finishes_live_stroke() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (mut app, scene, g) = setup(Tool::Brush);
        let maps = Materials::default();
        let view = make_view(&scene, &g, &maps);
        let before = app.session.active().unwrap().doc.clone();
        app.uv_paint_key = Some("first-undo".into());
        let first = screen(&view, 0);
        let mut paint = State::default();
        sample(&mut app, &mut paint, &view, first, false);
        assert!(app.live_stroke.is_some());
        app.uv_cache.paint_for_test(paint);
        app.run("edit.undo", json!({})).unwrap();
        assert!(app.live_stroke.is_none());
        assert_eq!(app.session.active().unwrap().doc.layers[0].surface(), before.layers[0].surface());
        app.ui.tool = Tool::Eyedropper;
        app.run("tools.setColors", json!({"foreground":"#000000"})).unwrap();
        sample(&mut app, &mut State::default(), &view, first, false);
        assert!(app.session.tools.foreground[..3].iter().all(|v| *v > 0.99));
    }
    #[test]
    fn uv_brush_previews_live_splits_shells_and_coalesces_one_undo() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (mut app, scene, g) = setup(Tool::Brush);
        let maps = Materials::default();
        let view = make_view(&scene, &g, &maps);
        let before = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.past_len();
        app.uv_paint_key = Some("one-uv-gesture".into());
        let mut state = State::default();
        let first = screen(&view, 0);
        sample(&mut app, &mut state, &view, first, false);
        assert!(app.live_stroke.is_some());
        let index = app.session.active_index().unwrap();
        let (preview, key) = canvas::display_doc(&mut app, index);
        assert_ne!(key, 0);
        assert_ne!(preview.layers[0].surface().unwrap(), before.layers[0].surface().unwrap());
        sample(&mut app, &mut state, &view, screen(&view, 2), false);
        finish(&mut app, &mut state);
        let after = app.session.active().unwrap().doc.clone();
        assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
        assert_eq!(after.layers[0].surface().unwrap().rgba(64, 64), [1.0; 4], "UV gap must not acquire a connecting stroke");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers[0].surface().unwrap(), before.layers[0].surface().unwrap());
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers[0].surface().unwrap(), after.layers[0].surface().unwrap());
    }
    #[test]
    fn uv_paint_respects_selection_and_hidden_materials_and_eraser() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (mut app, scene, g) = setup(Tool::Brush);
        let maps = Materials::default();
        let view = make_view(&scene, &g, &maps);
        let before = app.session.active().unwrap().doc.clone();
        app.run("uv.mask.apply", json!({})).unwrap();
        let history = app.session.active().unwrap().history.past_len();
        let mut state = State::default();
        sample(&mut app, &mut state, &view, screen(&view, 0), false);
        finish(&mut app, &mut state);
        assert_eq!(before.layers[0].surface().unwrap(), app.session.active().unwrap().doc.layers[0].surface().unwrap());
        assert_eq!(app.session.active().unwrap().history.past_len(), history, "masked-out live brush must not add an empty Undo step");
        app.run("select.deselect", json!({})).unwrap();
        app.ui.tool = Tool::Eraser;
        sample(&mut app, &mut state, &view, screen(&view, 0), false);
        finish(&mut app, &mut state);
        assert!(app.session.active().unwrap().doc.layers[0].surface().unwrap().read_region(before.bounds()).iter().any(|v| *v < 0.9));
        let hidden = Materials { hidden: std::collections::BTreeSet::from([scene.triangles[0].material]), ..Default::default() };
        let view = make_view(&scene, &g, &hidden);
        let revision = app.session.active().unwrap().revision;
        sample(&mut app, &mut state, &view, screen(&view, 0), false);
        finish(&mut app, &mut state);
        assert_eq!(app.session.active().unwrap().revision, revision);
    }
}
