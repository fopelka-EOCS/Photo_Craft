//! Project image shelf: immutable previews and typed drags into native editable layers.
use crate::{PhotocraftApp, theme::Tokens};
use egui::{Pos2, Rect, Sense, TextureHandle, TextureOptions, vec2};
use serde_json::json;
#[derive(Default)]
pub(crate) struct Cache {
    images: std::collections::BTreeMap<u64, (std::sync::Arc<Vec<u8>>, TextureHandle)>,
    target: Option<(Rect, u64, egui::LayerId)>,
    #[cfg(test)]
    layers_target: Option<Rect>,
}
#[derive(Clone)]
struct Drag {
    id: u64,
}
fn place(app: &mut PhotocraftApp, id: u64) {
    crate::uv_ui::finish_paint(app);
    if app.ui.transform.is_some() {
        crate::transform_tool::commit(app);
    }
    if let Err(error) = app.run("uv.asset.place", json!({"asset":id})) {
        app.ui.uv_workbench.status = error;
    }
}
pub(crate) fn layers_drop(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> bool {
    if app.ui.workspace != "3D" {
        return false;
    }
    let rect = ui.available_rect_before_wrap().intersect(ui.clip_rect());
    #[cfg(test)]
    {
        app.uv_cache.library.layers_target = Some(rect);
    }
    let over = ui.input(|i| i.pointer.latest_pos()).is_some_and(|p| rect.contains(p) && ui.ctx().layer_id_at(p).is_none_or(|top| top == ui.layer_id()));
    if egui::DragAndDrop::has_payload_of_type::<Drag>(ui.ctx()) && over {
        ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(1.0, Tokens::get(ui.ctx()).accent), egui::StrokeKind::Inside);
        if ui.input(|i| i.pointer.any_released())
            && let Some(payload) = egui::DragAndDrop::take_payload::<Drag>(ui.ctx())
        {
            place(app, payload.id);
            return true;
        }
    }
    false
}
pub(crate) fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if app.ui.workspace != "3D" {
        ui.weak(tl!("Project images are available in the 3D workspace"));
        return;
    }
    if app.session.uv.scene.is_none() {
        app.uv_cache.library.target = None;
        ui.weak(tl!("Import a 3D model first"));
        return;
    }
    let bounds = ui.available_rect_before_wrap().intersect(ui.clip_rect());
    app.uv_cache.library.target = Some((bounds, ui.ctx().cumulative_frame_nr(), ui.layer_id()));
    let t = Tokens::get(ui.ctx());
    ui.weak(tl!("Drop images here, then drag them into Layers"));
    if ui.button(tl!("Add images…")).clicked() {
        let r = app.ask_file(
            crate::file_dialog::FileDialogRequest::Open {
                multiple: true,
                initial_dir: None,
                extensions: Some(["png", "jpg", "jpeg", "tif", "tiff", "tga", "exr", "bmp", "dds", "pcraft"].into_iter().map(str::to_owned).collect()),
            },
            |app, answer| {
                let crate::file_dialog::FileDialogAnswer::Paths(paths) = answer else {
                    return Err(tl!("A local file is required").into());
                };
                let mut errors = Vec::new();
                for path in paths {
                    if let Err(e) = app.run("uv.asset.import", json!({"path":path})) {
                        errors.push(e);
                    }
                }
                if errors.is_empty() { Ok(serde_json::Value::Null) } else { Err(errors.join("\n")) }
            },
        );
        if let Err(e) = r {
            app.ui.uv_workbench.status = e;
        }
    }
    let assets = app.session.uv.assets.clone();
    app.uv_cache.library.images.retain(|id, _| assets.iter().any(|a| a.id == *id));
    let mut remove = None;
    for asset in assets {
        let handle =
            if let Some((_, handle)) = app.uv_cache.library.images.get(&asset.id).filter(|(bytes, _)| std::sync::Arc::ptr_eq(bytes, &asset.source.bytes)) {
                Some(handle.clone())
            } else {
                let sz = asset.document.size;
                let scale = (64.0 / sz.width.max(sz.height).max(1) as f32).min(1.0);
                let (w, h) = ((sz.width as f32 * scale).round().max(1.0) as u32, (sz.height as f32 * scale).round().max(1.0) as u32);
                let px = photocraft_compose::render_reduced(&asset.document, w, h);
                app.session.color.canvas_display(&asset.document).ok().map(|display| {
                    let rgba = display.to_rgba8(&px);
                    let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba.pixels);
                    let handle = ui.ctx().load_texture(format!("uv-asset-{}", asset.id), image, TextureOptions::LINEAR);
                    app.uv_cache.library.images.insert(asset.id, (asset.source.bytes.clone(), handle.clone()));
                    handle
                })
            };
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 72.0), Sense::click_and_drag());
        ui.painter().rect_filled(rect, 3.0, t.chrome);
        if let Some(image) = handle {
            let size = image.size_vec2();
            let s = (60.0 / size.x.max(size.y)).min(1.0);
            let area = Rect::from_center_size(rect.left_center() + vec2(36.0, 0.0), size * s);
            ui.painter().image(image.id(), area, Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        let text = ui.painter().layout(asset.source.name.clone(), egui::FontId::proportional(12.0), t.text, (rect.width() - 84.0).max(20.0));
        ui.painter().galley(rect.min + vec2(76.0, 12.0), text, t.text);
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, crate::i18n::fmt(tl!("Image: {name}"), &[("name", &asset.source.name)])));
        response.dnd_set_drag_payload(Drag { id: asset.id });
        if response.double_clicked() && app.session.active().is_some() {
            place(app, asset.id);
        }
        response.context_menu(|ui| {
            if ui.add_enabled(app.session.active().is_some(), egui::Button::new(tl!("Add to Layers"))).clicked() {
                place(app, asset.id);
                ui.close();
            }
            if ui.button(tl!("Remove from library")).clicked() {
                remove = Some(asset.id);
                ui.close();
            }
        });
    }
    if let Some(id) = remove
        && let Err(e) = app.run("uv.asset.remove", json!({"asset":id}))
    {
        app.ui.uv_workbench.status = e;
    }
}
pub(crate) fn file_drop(app: &mut PhotocraftApp, ctx: &egui::Context, at: Option<Pos2>, files: &mut Vec<egui::DroppedFileHandle>) {
    if app.ui.workspace != "3D" || ctx.viewport_id() != egui::ViewportId::ROOT {
        return;
    }
    let Some((rect, frame, layer)) = app.uv_cache.library.target else {
        return;
    };
    let at = at.or_else(|| ctx.input(|i| i.pointer.latest_pos()));
    if at.is_some_and(|p| ctx.layer_id_at(p).is_some_and(|top| top != layer)) {
        return;
    }
    if frame.saturating_add(1) < ctx.cumulative_frame_nr() || !at.or_else(|| ctx.input(|i| i.pointer.latest_pos())).is_some_and(|p| rect.contains(p)) {
        return;
    }
    let mut errors = Vec::new();
    for file in files.drain(..) {
        let path = file.path();
        let result = if path.is_absolute() {
            app.run("uv.asset.import", json!({"path":path}))
        } else {
            crate::read_dropped(&*file)
                .and_then(|bytes| photocraft_engine::uv_asset_cmds::import_bytes(&mut app.session, &path.to_string_lossy(), bytes).map_err(|e| e.to_string()))
        };
        if let Err(e) = result {
            errors.push(e);
        }
    }
    app.ui.uv_workbench.status = errors.join("\n");
    if !errors.is_empty() {
        app.ui.status = app.ui.uv_workbench.status.clone();
        app.ui.status_error = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    #[derive(Debug)]
    struct Drop(std::path::PathBuf);
    impl egui::DroppedFile for Drop {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            Err("uses native path".into())
        }
    }
    #[test]
    fn uv_library_os_drop_preview_and_drag_into_editable_layers() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut session = photocraft_engine::Session::new();
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.fbx");
        session.execute("uv.model.import", json!({"path":fixture})).unwrap();
        let mut app = PhotocraftApp::new(session, Default::default());
        app.ui.workspace = "3D".into();
        crate::menus::apply_workspace(&mut app);
        app.ui.dock_tabs.properties = 3;
        let mut h = Harness::builder().with_size(vec2(1200.0, 900.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            app.set_wgpu(cc.wgpu_render_state.clone().unwrap());
            app
        });
        h.run_steps(4);
        let target = h.state().uv_cache.library.target.unwrap().0.center();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/checker.png");
        let docs = h.state().session.documents().len();
        h.hover_at(target);
        h.run_steps(1);
        h.input_mut().dropped_files.push(std::sync::Arc::new(Drop(source)));
        h.run_steps(4);
        assert_eq!(h.state().session.uv.assets.len(), 1);
        assert_eq!(h.state().session.documents().len(), docs, "shelf import does not open extra texture tabs");
        assert_eq!(h.state().uv_cache.library.images.len(), 1);
        let before = h.state().session.active().unwrap().doc.layers.len();
        let start = h.get_by_label("Image: checker.png").rect().center();
        h.hover_at(start);
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        h.event(egui::Event::PointerMoved(start + vec2(-30.0, 10.0)));
        h.run_steps(2);
        assert!(egui::DragAndDrop::has_payload_of_type::<Drag>(&h.ctx));
        let end = h.state().uv_cache.library.layers_target.unwrap().center();
        h.event(egui::Event::PointerMoved(end));
        h.run_steps(2);
        h.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(4);
        let st = h.state().session.active().unwrap();
        assert_eq!(st.doc.layers.len(), before + 1);
        assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "checker");
        assert_eq!(st.selected_layers().len(), 1);
        assert!(matches!(st.doc.layer(st.active_layer.unwrap()).unwrap().content, photocraft_doc::LayerContent::Raster(_)));
        if let Ok(path) = std::env::var("PHOTOCRAFT_UV_LIBRARY_SCREENSHOT") {
            h.render().unwrap().save(path).unwrap();
        }
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), before);
    }
}
