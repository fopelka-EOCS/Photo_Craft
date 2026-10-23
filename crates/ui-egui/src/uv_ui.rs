//! Optional native 3D / UV workspace. All edits dispatch engine commands.
use crate::{PhotocraftApp, theme::Tokens};
use egui::{Color32, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, pos2, vec2};
use photocraft_uv::preview::{self, Camera, Channel, Image};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::{Duration, Instant};

/// View-only navigation preference, independent of texture and mesh editing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrbitPivot {
    #[default]
    ViewCenter,
    Cursor,
    Fixed,
}
impl OrbitPivot {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::ViewCenter => tl!("View center → surface"),
            Self::Cursor => tl!("Under cursor"),
            Self::Fixed => tl!("Fixed"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub open: bool,
    pub show_overlay: bool,
    pub select_shells: bool,
    pub paint_wrap: bool,
    #[serde(skip)]
    pub picker_3d: bool,
    #[serde(skip)]
    pub picker_viewport: Option<egui::ViewportId>,
    pub model_path: String,
    pub texture_path: String,
    pub project_path: String,
    pub uv_set: usize,
    pub guard: f64,
    pub horizontal: f64,
    pub vertical: f64,
    pub feather: f64,
    pub open_borders: bool,
    pub shaded: bool,
    pub channel: Channel,
    pub channel_2d: Channel,
    pub shell_enclosed: bool,
    pub view_3d: bool,
    pub split_view: bool,
    pub swap_views: bool,
    pub split_ratio: f32,
    pub floating_3d: bool,
    pub floating_2d: bool,
    pub import_settings: bool,
    pub perspective: bool,
    pub antialias: bool,
    pub fast_navigation: bool,
    pub orbit_pivot: OrbitPivot,
    pub cull_backfaces: bool,
    pub flip_faces: bool,
    pub lighting: photocraft_uv::lighting::Settings,
    pub normal_strength: f64,
    pub flip_normal_y: bool,
    pub pan: [f64; 2],
    pub target: Option<[f64; 3]>,
    pub yaw: f64,
    pub pitch: f64,
    pub zoom: f64,
    pub status: String,
    pub overlap: Vec<usize>,
}
impl Default for State {
    fn default() -> Self {
        let c = Camera::default();
        Self {
            open: false,
            show_overlay: true,
            select_shells: false,
            paint_wrap: false,
            picker_3d: false,
            picker_viewport: None,
            model_path: String::new(),
            texture_path: String::new(),
            project_path: String::new(),
            uv_set: 0,
            guard: 8.0,
            horizontal: 0.0,
            vertical: 0.0,
            feather: 0.0,
            open_borders: false,
            shaded: true,
            channel: Channel::Material,
            channel_2d: Channel::Color,
            shell_enclosed: false,
            view_3d: false,
            split_view: false,
            swap_views: false,
            split_ratio: 0.5,
            floating_3d: false,
            floating_2d: false,
            import_settings: false,
            perspective: false,
            antialias: true,
            fast_navigation: true,
            orbit_pivot: OrbitPivot::default(),
            cull_backfaces: true,
            flip_faces: false,
            lighting: Default::default(),
            normal_strength: 1.0,
            flip_normal_y: false,
            pan: [0.0; 2],
            target: None,
            yaw: c.yaw,
            pitch: c.pitch,
            zoom: c.zoom,
            status: String::new(),
            overlap: Vec::new(),
        }
    }
}

impl State {
    fn camera(&self) -> Camera {
        Camera {
            perspective: self.perspective,
            yaw: self.yaw,
            pitch: self.pitch,
            zoom: self.zoom,
            pan: self.pan,
            target: self.target,
            shaded: self.shaded,
            channel: self.channel,
            cull_backfaces: self.cull_backfaces,
            flip_faces: self.flip_faces,
            authored_normals: true,
            normal_strength: self.normal_strength,
            flip_normal_y: self.flip_normal_y,
        }
    }
}

#[derive(Default)]
pub(crate) struct Cache {
    pub(crate) gpu: Option<crate::uv_gpu::Renderer>,
    pub(crate) library: crate::uv_library::Cache,
    pub(crate) project: crate::uv_project::Ui,
    pub(crate) workspace_views: std::collections::HashMap<photocraft_doc::DocId, crate::state::View>,
    masked: std::collections::BTreeSet<usize>,
    pub(crate) lighting: crate::uv_lighting::Cache,
    geometry: Option<(std::sync::Arc<photocraft_uv::Scene>, std::sync::Arc<photocraft_uv::prepared::Geometry>)>,
    textures: Vec<Option<Image>>,
    normals: Vec<Option<Image>>,
    opacity: Vec<Option<Image>>,
    opacity_keys: Vec<(u64, u64, u64)>,
    normal_keys: Vec<(u64, u64, u64)>,

    keys: Vec<(u64, u64, u64, u64)>,
    snapshots: Vec<Option<std::sync::Arc<photocraft_doc::Document>>>,
    normal_snapshots: Vec<Option<std::sync::Arc<photocraft_doc::Document>>>,
    frame: Option<preview::Frame>,
    frame_handle: Option<TextureHandle>,
    rendered: Option<(u64, Camera, usize, usize, bool)>,
    refreshed: Option<Instant>,
    texture_side: usize,
    navigation: Option<Navigation>,
    wheel_active: Option<Instant>,
    display_channel: Channel,
    pub(crate) viewport: Option<Rect>,
    pub(crate) viewport_id: Option<egui::ViewportId>,
    pub(crate) area: Option<Rect>,
    pub(crate) shell_tool_owner: Option<crate::state::Tool>,
    paint: crate::uv_paint::State,
    shell_box_3d: Option<crate::uv_box::Drag>,
    shell_box_2d: Option<crate::uv_box::Drag>,
    pub(crate) mask_settings_pending: bool,
    source_checked: Option<Instant>,
    source_generation: u64,
    source_message: String,
    // Immutable slot previews: keyed by source document, never by its live paint revision.
    thumbnails: std::collections::BTreeMap<(u64, bool), TextureHandle>,
    texture_targets: Vec<(Rect, usize, Channel)>,
    texture_target_frame: u64,
}

#[cfg(test)]
impl Cache {
    pub(crate) fn paint_for_test(&mut self, state: crate::uv_paint::State) {
        self.paint = state;
    }
}

pub(crate) fn pending_paint(app: &PhotocraftApp) -> bool {
    crate::uv_paint::pending(&app.uv_cache.paint)
}

pub(crate) fn finish_paint(app: &mut PhotocraftApp) {
    let mut paint = std::mem::take(&mut app.uv_cache.paint);
    crate::uv_paint::finish(app, &mut paint);
    app.uv_cache.paint = paint;
}
pub(crate) fn reimport(app: &mut PhotocraftApp) {
    finish_paint(app);
    match app.run("uv.model.reimport", json!({"uvSet":app.ui.uv_workbench.uv_set})) {
        Ok(result) => {
            app.ui.uv_workbench.overlap.clear();
            let message: String = if result["uvChanged"] == true {
                tl!("Mesh reimported. UVs changed: check existing seam protection masks.").into()
            } else {
                tl!("Mesh reimported; textures and layers preserved.").into()
            };
            app.ui.uv_workbench.status = if result["uvChanged"] == true { message.clone() } else { String::new() };
            app.ui.status = message;
            app.ui.status_error = false;
            app.uv_cache.source_message.clear();
            app.uv_cache.source_checked = None;
        }
        Err(error) => app.ui.uv_workbench.status = error,
    }
}
fn check_source(app: &mut PhotocraftApp) {
    if app.session.uv.scene.is_none() || app.drag.is_some() {
        return;
    }
    if app.uv_cache.source_generation == app.session.uv.generation && app.uv_cache.source_checked.is_some_and(|at| at.elapsed() < Duration::from_secs(1)) {
        return;
    }
    app.uv_cache.source_checked = Some(Instant::now());
    app.uv_cache.source_generation = app.session.uv.generation;
    if let Ok(result) = app.run("uv.model.checkSource", json!({})) {
        app.uv_cache.source_message = if result["available"] == false {
            tl!("The source model is unavailable. The current mesh is retained.").into()
        } else if result["changed"] == true {
            tl!("The source model changed — click ReImport").into()
        } else {
            String::new()
        };
    }
}

fn command(app: &mut PhotocraftApp, state: &mut State, id: &str, params: serde_json::Value) -> bool {
    match app.run(id, params) {
        Ok(_) => {
            if id == "uv.layer.create"
                && let Err(e) = app.run("channel.target.composite", json!({}))
            {
                state.status = e;
                return false;
            }
            state.status = String::new();
            true
        }
        Err(e) => {
            state.status = e.to_string();
            false
        }
    }
}

pub(crate) fn patch_image(target: &mut Option<Image>, size: [u32; 2], rect: photocraft_geom::Rect, pixels: Vec<[f32; 4]>) -> Result<(), String> {
    let [w, h] = size.map(|v| v as usize);
    if pixels.len() != rect.width() as usize * rect.height() as usize {
        return Err("Invalid UV preview update".into());
    }
    if rect == photocraft_geom::Rect::from_xywh(0, 0, size[0], size[1]) {
        *target = Some(Image { width: w, height: h, rgba: pixels });
        return Ok(());
    }
    let image = target.as_mut().filter(|im| im.width == w && im.height == h).ok_or("Missing UV preview cache")?;
    if rect.is_empty() {
        return Ok(());
    }
    if rect.x0 < 0 || rect.y0 < 0 || rect.x1 as usize > w || rect.y1 as usize > h {
        return Err("UV preview update exceeds image".into());
    }
    for (row, src) in pixels.chunks_exact(rect.width() as usize).enumerate() {
        let start = (rect.y0 as usize + row) * w + rect.x0 as usize;
        image.rgba.get_mut(start..start + src.len()).ok_or("Invalid UV preview cache")?.copy_from_slice(src);
    }
    Ok(())
}
fn region(rect: photocraft_geom::Rect) -> [u32; 4] {
    if rect.is_empty() { [0; 4] } else { [rect.x0 as u32, rect.y0 as u32, rect.width(), rect.height()] }
}

fn refresh(app: &mut PhotocraftApp, ctx: &egui::Context, cache: &mut Cache) -> Result<(), String> {
    let map_kind = if matches!(cache.display_channel, Channel::Material | Channel::Normal) { Channel::Color } else { cache.display_channel };
    let mut bindings = app.session.uv.map_bindings(map_kind).to_vec();
    cache.masked.clear();
    for (material, id) in bindings.iter_mut().enumerate() {
        if let Some(index) = crate::uv_mask::source(app, material) {
            *id = app.session.documents().get(index).map(|st| st.doc.id);
            cache.masked.insert(material);
        }
    }
    let slots = app.session.uv.all_bindings().filter(|id| id.is_some()).count().max(1);
    let gpu_limit = cache.gpu.as_ref().map(|g| g.max_texture_side());
    if let Some(limit) = gpu_limit {
        let mut bytes = 0u64;
        for id in app.session.uv.all_bindings().flatten() {
            if let Some(st) = app.session.documents().iter().find(|st| st.doc.id == *id) {
                if st.doc.size.width > limit || st.doc.size.height > limit {
                    return Err(crate::i18n::fmt(
                        tl!("Source map {width} × {height} exceeds the GPU limit of {limit}. Maps are not downscaled automatically."),
                        &[("width", &st.doc.size.width.to_string()), ("height", &st.doc.size.height.to_string()), ("limit", &limit.to_string())],
                    ));
                }
                bytes = bytes.saturating_add(u64::from(st.doc.size.width) * u64::from(st.doc.size.height) * 16);
            }
        }
        if bytes > 4 * 1024 * 1024 * 1024 {
            return Err(tl!("Source maps require more than 4 GiB of CPU cache. Remove unused material bindings; maps are not downscaled automatically.").into());
        }
    }
    // GPU uses the source size. The small float-map budget belongs only to the CPU fallback.
    let texture_side = gpu_limit.map_or_else(|| (16_777_216.0 / slots as f64).sqrt().floor().clamp(1.0, 2048.0), f64::from);
    let mut textures_changed = cache.texture_side != texture_side as usize;
    if textures_changed {
        cache.keys.fill((0, 0, 0, 0));
        cache.normal_keys.fill((0, 0, 0));
        cache.texture_side = texture_side as usize;
    }
    let opacity_bindings = app.session.uv.map_bindings(Channel::Opacity).to_vec();
    if cache.opacity.len() != bindings.len() {
        cache.opacity = vec![None; bindings.len()];
        cache.opacity_keys = vec![(0, 0, 0); bindings.len()];
    }
    for material in 0..bindings.len() {
        let index = opacity_bindings.get(material).copied().flatten().and_then(|id| app.session.documents().iter().position(|st| st.doc.id == id));
        let Some(index) = index else {
            if let Some(dst) = cache.opacity.get_mut(material) {
                *dst = None;
            }
            if let Some(key) = cache.opacity_keys.get_mut(material) {
                *key = (0, 0, 0);
            }
            continue;
        };
        let revision = app.session.documents().get(index).map_or(0, |st| st.revision);
        let (doc, preview) = crate::canvas::display_doc(app, index);
        let key = (doc.id.0, revision, preview);
        if cache.opacity_keys.get(material) == Some(&key) {
            continue;
        }
        let scale = (texture_side / f64::from(doc.size.width.max(doc.size.height).max(1))).min(1.0);
        let (w, h) = ((f64::from(doc.size.width) * scale).round().max(1.0) as u32, (f64::from(doc.size.height) * scale).round().max(1.0) as u32);
        let buffer = photocraft_compose::render_reduced(&doc, w, h);
        if let Some(dst) = cache.opacity.get_mut(material) {
            *dst = Some(Image { width: w as usize, height: h as usize, rgba: buffer.px });
        }
        if let Some(dst) = cache.opacity_keys.get_mut(material) {
            *dst = key;
        }
    }
    if bindings.len() != cache.textures.len() {
        cache.textures = vec![None; bindings.len()];

        cache.keys = vec![(0, 0, 0, 0); bindings.len()];
        cache.snapshots = vec![None; bindings.len()];
        textures_changed = true;
    }
    for (material, id) in bindings.iter().enumerate() {
        let Some(index) = id.and_then(|id| app.session.documents().iter().position(|d| d.doc.id == id)) else {
            let opacity = cache
                .opacity
                .get(material)
                .and_then(Option::as_ref)
                .filter(|_| matches!(cache.display_channel, Channel::Material | Channel::Color | Channel::Normal));
            if let Some(opacity) = opacity {
                let k = cache.opacity_keys.get(material).copied().unwrap_or_default();
                let key = (0, k.1, k.2, k.0.wrapping_mul(0x9e3779b97f4a7c15));
                if cache.keys.get(material) != Some(&key) || cache.textures.get(material).is_none_or(Option::is_none) {
                    let image = Image {
                        width: opacity.width,
                        height: opacity.height,
                        rgba: opacity.rgba.iter().map(|p| [1.0, 1.0, 1.0, p[0].clamp(0.0, 1.0) * p[3].clamp(0.0, 1.0)]).collect(),
                    };
                    if let Some(dst) = cache.textures.get_mut(material) {
                        *dst = Some(image);
                    }
                    if let Some(dst) = cache.keys.get_mut(material) {
                        *dst = key;
                    }
                    textures_changed = true;
                }
            } else if let Some(image) = cache.textures.get_mut(material)
                && image.take().is_some()
            {
                textures_changed = true;
            }
            continue;
        };
        let revision = app.session.documents().get(index).map_or(0, |d| d.revision);
        // The SAME display snapshot as the canvas includes an in-progress brush stroke,
        // adjustment dialog, gradient and undo/redo, without waiting for a file save.
        let (doc, preview_key) = if cache.masked.contains(&material) {
            crate::canvas::filter_channel_preview(app, index, ctx).unwrap_or_else(|| crate::canvas::display_doc(app, index))
        } else {
            crate::canvas::display_doc(app, index)
        };
        let display = app.session.color.canvas_display(&doc).map_err(|e| e.to_string())?;
        let opacity_key = if matches!(cache.display_channel, Channel::Material | Channel::Color | Channel::Normal) {
            cache.opacity_keys.get(material).copied().unwrap_or_default()
        } else {
            (0, 0, 0)
        };
        let alpha_key = opacity_key.0.wrapping_mul(0x9e3779b97f4a7c15) ^ opacity_key.1.rotate_left(17) ^ opacity_key.2.rotate_left(39);
        let masked = cache.masked.contains(&material);
        let view_key = if masked { crate::uv_mask::key(app, index) } else { 0 };
        let key = (doc.id.0, revision, preview_key, display.key ^ alpha_key ^ view_key);
        if cache.keys.get(material) == Some(&key) && cache.textures.get(material).is_some_and(Option::is_some) {
            continue;
        }
        let scale = texture_side / f64::from(doc.size.width.max(doc.size.height).max(1));
        let w = (f64::from(doc.size.width) * scale.min(1.0)).round().max(1.0) as u32;
        let h = (f64::from(doc.size.height) * scale.min(1.0)).round().max(1.0) as u32;
        let old = cache.keys.get(material).copied().unwrap_or_default();
        let damage = if old.0 == key.0
            && old.3 == key.3
            && cache.textures.get(material).and_then(Option::as_ref).is_some_and(|im| im.width == w as usize && im.height == h as usize)
        {
            crate::canvas::damage_since(
                app,
                index,
                (old.1, old.2 ^ display.key),
                (revision, preview_key),
                display.key,
                app.session.documents().get(index).and_then(|st| st.last_damage),
            )
        } else {
            None
        };
        if old.3 != key.3 || cache.textures[material].is_none() {
            cache.snapshots[material] = None;
        }
        let regions = crate::uv_damage::regions(&mut cache.snapshots[material], &doc, damage);
        let mut hints = Vec::new();
        for rect in regions {
            let buffer = photocraft_compose::render_reduced_damage(&doc, w, h, rect);
            let image = display.to_rgba8(&buffer);
            let mut rgba: Vec<[f32; 4]> = if matches!(map_kind, Channel::Color | Channel::Emissive) {
                image.pixels.as_chunks::<4>().0.iter().map(|p| p.map(|v| f32::from(v) / 255.0)).collect()
            } else {
                buffer.px
            };
            if opacity_key.0 != 0
                && let Some(opacity) = cache.opacity.get(material).and_then(Option::as_ref)
            {
                for (i, pixel) in rgba.iter_mut().enumerate() {
                    let x = buffer.rect.x0 as usize + i % buffer.rect.width() as usize;
                    let y = buffer.rect.y0 as usize + i / buffer.rect.width() as usize;
                    let sample = preview::sample(opacity, [(x as f64 + 0.5) / f64::from(w), 1.0 - (y as f64 + 0.5) / f64::from(h)]);
                    pixel[3] *= sample[0].clamp(0.0, 1.0) * sample[3].clamp(0.0, 1.0);
                }
            }
            if masked {
                let factor = doc.size.width.max(doc.size.height).div_ceil(w.max(h)).max(1);
                crate::uv_mask::overlay(app, index, &doc, rect, &mut rgba, factor);
            }
            if let Some(t) = cache.textures.get_mut(material) {
                patch_image(t, [w, h], buffer.rect, rgba)?;
            }
            hints.push(region(buffer.rect));
        }
        if old.0 == key.0
            && let Some(gpu) = &mut cache.gpu
        {
            gpu.hint_region(material, false, (old.0, old.1, old.2, old.3 ^ ((u64::from(w)) << 32) ^ u64::from(h)), hints);
        }
        if let Some(k) = cache.keys.get_mut(material) {
            *k = key;
        }
        textures_changed = true;
    }
    let normal_bindings = app.session.uv.normal_bindings.clone();
    if cache.normals.len() != normal_bindings.len() {
        cache.normals = vec![None; normal_bindings.len()];
        cache.normal_keys = vec![(0, 0, 0); normal_bindings.len()];
        cache.normal_snapshots = vec![None; normal_bindings.len()];
        textures_changed = true;
    }
    for (material, id) in normal_bindings.iter().enumerate() {
        let Some(index) = id.and_then(|id| app.session.documents().iter().position(|d| d.doc.id == id)) else {
            if let Some(img) = cache.normals.get_mut(material)
                && img.take().is_some()
            {
                textures_changed = true;
            }
            continue;
        };
        let revision = app.session.documents().get(index).map_or(0, |d| d.revision);
        let (doc, preview_key) = crate::canvas::display_doc(app, index);
        let key = (doc.id.0, revision, preview_key);
        if cache.normal_keys.get(material) == Some(&key) && cache.normals.get(material).is_some_and(Option::is_some) {
            continue;
        }
        let scale = (texture_side / f64::from(doc.size.width.max(doc.size.height).max(1))).min(1.0);
        let w = (f64::from(doc.size.width) * scale).round().max(1.0) as u32;
        let h = (f64::from(doc.size.height) * scale).round().max(1.0) as u32;
        // Normal maps are data: sample native numeric RGB, bypass display ICC/proof/gamma.
        let old = cache.normal_keys.get(material).copied().unwrap_or_default();
        let damage =
            if old.0 == key.0 && cache.normals.get(material).and_then(Option::as_ref).is_some_and(|im| im.width == w as usize && im.height == h as usize) {
                crate::canvas::damage_since(
                    app,
                    index,
                    (old.1, old.2),
                    (revision, preview_key),
                    0,
                    app.session.documents().get(index).and_then(|st| st.last_damage),
                )
            } else {
                None
            };
        if cache.normals[material].as_ref().is_none_or(|im| im.width != w as usize || im.height != h as usize) {
            cache.normal_snapshots[material] = None;
        }
        let regions = crate::uv_damage::regions(&mut cache.normal_snapshots[material], &doc, damage);
        let mut hints = Vec::new();
        for rect in regions {
            let buffer = photocraft_compose::render_reduced_damage(&doc, w, h, rect);
            hints.push(region(buffer.rect));
            if let Some(image) = cache.normals.get_mut(material) {
                patch_image(image, [w, h], buffer.rect, buffer.px)?;
            }
        }
        if old.0 == key.0
            && let Some(gpu) = &mut cache.gpu
        {
            gpu.hint_region(material, true, (old.0, old.1, old.2, (u64::from(w) << 32) | u64::from(h)), hints);
        }
        if let Some(k) = cache.normal_keys.get_mut(material) {
            *k = key;
        }
        textures_changed = true;
    }
    if textures_changed {
        cache.rendered = None;
    }
    Ok(())
}

pub(crate) fn reveal(app: &mut PhotocraftApp) -> Result<(), String> {
    if app.ui.workspace != "3D" {
        app.ui.workspace = "3D".into();
        crate::menus::apply_workspace(app);
    }
    crate::uv_workspace::sync_workspace(app);
    app.ui.uv_workbench.open = true;
    app.ui.panels.dock = true;
    if app.ui.docking.is_some() {
        if let Some(result) = crate::panel_docking::command(app, "window.panel.activate", &json!({"panel":"uvWorkbench"})) {
            result?;
        }
    } else {
        app.ui.dock_tabs.properties = 2;
        crate::dock::reveal(app, crate::dock::Group::Properties);
        app.ui.dock.heights.entry(crate::dock::Group::Properties).or_insert(440.0);
    }
    Ok(())
}
pub(crate) fn pick_file(app: &mut PhotocraftApp, kind: &'static str, material: usize, normal: bool) -> Result<serde_json::Value, String> {
    reveal(app)?;
    pick_map_file(app, kind, material, if normal { Channel::Normal } else { Channel::Color })
}
fn pick_map_file(app: &mut PhotocraftApp, kind: &'static str, material: usize, channel: Channel) -> Result<serde_json::Value, String> {
    use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};
    let extensions = match kind {
        "model" => vec!["fbx", "obj"],
        "project" => vec!["pcuv"],
        _ => vec!["png", "tga", "jpg", "jpeg", "tif", "tiff", "dds", "exr", "bmp", "pcraft"],
    }
    .into_iter()
    .map(str::to_owned)
    .collect();
    let uv_set = app.ui.uv_workbench.uv_set;
    let initial_dir = if kind == "texture" {
        let slots = app.session.uv.map_bindings(channel);
        slots
            .get(material)
            .copied()
            .flatten()
            .and_then(|id| app.session.documents().iter().find(|d| d.doc.id == id))
            .and_then(|d| d.path.as_deref())
            .and_then(|path| std::path::Path::new(path).parent())
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.to_string_lossy().into_owned())
    } else {
        None
    };
    app.ask_file(FileDialogRequest::Open { multiple: false, initial_dir, extensions: Some(extensions) }, move |app, answer| {
        let FileDialogAnswer::Paths(paths) = answer else { return Err("This import needs a local file".into()) };
        let path = paths.first().ok_or("No file selected")?;
        let (id, params) = match kind {
            "model" => ("uv.model.import", json!({"path":path,"uvSet":uv_set})),
            "project" => ("uv.project.load", json!({"path":path})),
            _ => ("uv.texture.open", json!({"path":path,"material":material,"channel":channel.key()})),
        };
        finish_paint(app);
        let result = app.run(id, params);
        match &result {
            Ok(_) => {
                app.ui.uv_workbench.status.clear();
                if kind == "model" {
                    app.ui.uv_workbench.model_path = path.clone();
                    app.ui.uv_workbench.project_path.clear();
                    app.ui.uv_workbench.view_3d = true;
                }
                if kind == "project" {
                    app.ui.uv_workbench.project_path = path.clone();
                    app.ui.uv_workbench.view_3d = true;
                }
                app.ui.uv_workbench.guard = app.session.uv.guard;
                app.ui.uv_workbench.horizontal = app.session.uv.horizontal;
                app.ui.uv_workbench.vertical = app.session.uv.vertical;
                app.ui.uv_workbench.feather = app.session.uv.feather;
                app.ui.uv_workbench.open_borders = app.session.uv.open_borders;
                app.ui.uv_workbench.uv_set = app.session.uv.uv_set;
                // Reset scene resources, retaining the device attached at application startup.
                // Dropping the renderer here silently selected CPU rasterization after every picker.
                let mut gpu = app.uv_cache.gpu.take();
                if let Some(renderer) = &mut gpu {
                    renderer.clear();
                }
                app.uv_cache = Cache {
                    gpu,
                    lighting: std::mem::take(&mut app.uv_cache.lighting),
                    thumbnails: std::mem::take(&mut app.uv_cache.thumbnails),
                    ..Cache::default()
                };
                if kind == "project"
                    && let Ok(value) = &result
                    && let Err(error) = crate::uv_project::restore(app, &value["view"])
                {
                    app.ui.uv_workbench.status = error;
                }
            }
            Err(e) => app.ui.uv_workbench.status = e.clone(),
        }
        result
    })
}
pub(crate) fn save_project(app: &mut PhotocraftApp) -> Result<serde_json::Value, String> {
    if !app.session.uv.project_path.is_empty() {
        return crate::uv_project::save_to(app, app.session.uv.project_path.clone());
    }
    save_project_as(app)
}
pub(crate) fn save_project_as(app: &mut PhotocraftApp) -> Result<serde_json::Value, String> {
    let suggested = if app.ui.uv_workbench.project_path.is_empty() { "model.pcuv".into() } else { app.ui.uv_workbench.project_path.clone() };
    app.pick_save(&suggested, |app, path| {
        let path = if path.to_ascii_lowercase().ends_with(".pcuv") { path } else { format!("{path}.pcuv") };
        crate::uv_project::save_to(app, path)
    })
}
/// Settings belong to the shell-selection tool, not to a material.
pub(crate) fn shell_settings(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let mut state = std::mem::take(&mut app.ui.uv_workbench);
    ui.horizontal(|ui| {
        ui.label(tl!("Marquee:"));
        ui.selectable_value(&mut state.shell_enclosed, false, tl!("Intersecting"));
        ui.selectable_value(&mut state.shell_enclosed, true, tl!("Fully enclosed"));
    });
    let previous = (state.guard, state.horizontal, state.vertical, state.feather);
    for (label, value) in [
        (tl!("General margin, px"), &mut state.guard),
        (tl!("Horizontal margin, px"), &mut state.horizontal),
        (tl!("Vertical margin, px"), &mut state.vertical),
        (tl!("Inward feather, px"), &mut state.feather),
    ] {
        ui.horizontal(|ui| {
            ui.label(label);
            ui.add(egui::DragValue::new(value).range(0.0..=1024.0));
        });
    }
    if previous != (state.guard, state.horizontal, state.vertical, state.feather) && app.session.uv.scene.is_some() {
        let p = mask_params(&state);
        if command(app, &mut state, "uv.settings", p) {
            app.uv_cache.mask_settings_pending = true;
        }
    }
    app.ui.uv_workbench = state;
}
fn mask_params(state: &State) -> serde_json::Value {
    json!({"guard":state.guard,"horizontal":state.horizontal,"vertical":state.vertical,"feather":state.feather,"openBorders":state.open_borders})
}
fn flush_mask_settings(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // Dragging numbers must not rasterize a 4K selection on every mouse move.
    if app.uv_cache.mask_settings_pending && !ctx.input(|i| i.pointer.primary_down()) {
        app.uv_cache.mask_settings_pending = false;
        if !app.session.uv.selected.is_empty() {
            let p = mask_params(&app.ui.uv_workbench);
            if let Err(e) = app.run("uv.mask.apply", p) {
                app.ui.uv_workbench.status = e;
            }
        }
    }
}
/// Selecting a map also chooses its editable 2D document and native paint channel.
pub(crate) fn select_view_channel(app: &mut PhotocraftApp, channel: Channel) {
    app.ui.uv_workbench.channel = channel;
    if channel != Channel::Material {
        select_2d_channel(app, channel);
    }
}
pub(crate) fn select_2d_channel(app: &mut PhotocraftApp, channel: Channel) {
    finish_paint(app);
    app.ui.uv_workbench.channel_2d = channel;
    if channel == Channel::Material {
        return;
    }
    let slots = app.session.uv.map_bindings(channel);
    let valid = |material: usize| slots.get(material).copied().flatten().is_some_and(|id| app.session.documents().iter().any(|d| d.doc.id == id));
    let material = if valid(app.session.uv.material) {
        Some(app.session.uv.material)
    } else {
        (0..slots.len()).find(|m| !app.session.uv.hidden_materials.contains(m) && valid(*m))
    };
    if let Some(material) = material {
        match app.run("uv.material.select", json!({"material":material,"channel":channel.key()})) {
            Ok(_) => {
                use photocraft_engine::channel_cmds::ChannelTarget;
                app.ui.mask_target = app.session.active().is_some_and(|st| st.channel_view.target == ChannelTarget::LayerMask);
                app.ui.vector_mask_target = app.session.active().is_some_and(|st| st.channel_view.target == ChannelTarget::VectorMask);
                app.sync_mask_targets();

                if !app.session.uv.selected.is_empty() {
                    let p = mask_params(&app.ui.uv_workbench);
                    if let Err(e) = app.run("uv.mask.apply", p) {
                        app.ui.uv_workbench.status = e;
                    }
                }
            }
            Err(e) => app.ui.uv_workbench.status = e,
        }
    }
}

pub(crate) fn has_channel(app: &PhotocraftApp, channel: Channel) -> bool {
    let bindings = app.session.uv.map_bindings(channel);
    bindings.iter().flatten().any(|id| app.session.documents().iter().any(|d| d.doc.id == *id))
}

/// A small immutable preview is composed once on assignment. Brush strokes never invalidate it.
fn source_thumbnail(app: &mut PhotocraftApp, ctx: &egui::Context, id: photocraft_doc::DocId, normal: bool) -> Option<TextureHandle> {
    let key = (id.0, normal);
    if let Some(handle) = app.uv_cache.thumbnails.get(&key) {
        return Some(handle.clone());
    }
    let st = app.session.documents().iter().find(|d| d.doc.id == id)?;
    let size = st.doc.size;
    let scale = 64.0 / f64::from(size.width.max(size.height).max(1));
    let w = (f64::from(size.width) * scale.min(1.0)).round().max(1.0) as u32;
    let h = (f64::from(size.height) * scale.min(1.0)).round().max(1.0) as u32;
    let buffer = photocraft_compose::render_reduced(&st.doc, w, h);
    let bytes = if normal {
        buffer.px.iter().flat_map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)).collect()
    } else {
        match app.session.color.canvas_display(&st.doc) {
            Ok(display) => display.to_rgba8(&buffer).pixels,
            Err(_) => return None,
        }
    };
    let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &bytes);
    let handle = ctx.load_texture(format!("uv-source-{}-{normal}", id.0), image, TextureOptions::LINEAR);
    app.uv_cache.thumbnails.insert(key, handle.clone());
    Some(handle)
}

/// Called before the generic OS-drop dispatcher. A drop over a slot is consumed exactly once.
pub(crate) fn texture_drop(app: &mut PhotocraftApp, ctx: &egui::Context, at: Option<Pos2>, files: &mut Vec<egui::DroppedFileHandle>) {
    if ctx.viewport_id() != egui::ViewportId::ROOT {
        return;
    }
    if app.uv_cache.texture_target_frame.saturating_add(1) < ctx.cumulative_frame_nr() {
        return;
    }
    let Some(at) = at.or_else(|| ctx.input(|i| i.pointer.latest_pos())) else {
        return;
    };
    if ctx.layer_id_at(at).is_some_and(|layer| layer.order != egui::Order::Background) {
        return;
    }
    let Some((_, material, channel)) = app.uv_cache.texture_targets.iter().find(|(rect, _, _)| rect.contains(at)).copied() else {
        return;
    };
    if files.is_empty() {
        return;
    }
    // One card represents one map. Reject ambiguous multi-file drops without opening extra documents.
    let result = if files.len() != 1 {
        Err(tl!("Drop one file onto the texture slot").into())
    } else if let Some(path) = files.first().map(|f| f.path()).filter(|p| p.is_absolute()) {
        finish_paint(app);
        app.run("uv.texture.open", json!({"path":path,"material":material,"channel":channel.key()}))
    } else {
        Err(tl!("Assigning a texture requires a local file").into())
    };
    files.clear();
    match result {
        Ok(_) => {
            app.ui.uv_workbench.status.clear();
            app.uv_cache.refreshed = None;
        }
        Err(e) => app.ui.uv_workbench.status = e,
    }
}

fn material_cards(app: &mut PhotocraftApp, state: &mut State, ui: &mut egui::Ui, material: usize) -> Option<(&'static str, usize, Channel)> {
    let mut pick = None;
    app.uv_cache.thumbnails.retain(|(id, normal), _| {
        let _ = normal;
        app.session.uv.all_bindings().flatten().any(|doc| doc.0 == *id)
    });
    app.uv_cache.texture_targets.clear();
    app.uv_cache.texture_target_frame = ui.ctx().cumulative_frame_nr();
    let hover_at = if ui.input(|i| !i.raw.hovered_files.is_empty()) {
        app.services.cursor_pos.as_mut().and_then(|f| f(ui.ctx())).or_else(|| ui.input(|i| i.pointer.latest_pos()))
    } else {
        None
    };
    for kind in Channel::MAPS {
        let label = kind.label();
        let channel = kind.key();
        let normal = !matches!(kind, Channel::Color | Channel::Emissive);
        let bindings = app.session.uv.map_bindings(kind);
        let bound = bindings.get(material).copied().flatten().and_then(|id| app.session.documents().iter().find(|d| d.doc.id == id));
        let info = bound.map(|st| {
            (st.doc.id, std::path::Path::new(&st.doc.name).file_name().unwrap_or_default().to_string_lossy().into_owned(), st.doc.name.clone(), st.doc.size)
        });
        let thumbnail = info.as_ref().and_then(|(id, _, _, _)| source_thumbnail(app, ui.ctx(), *id, normal));
        let selected = app.session.uv.current_channel() == kind;
        ui.push_id(("uv-map-card", kind), |ui| {
            let t = Tokens::get(ui.ctx());
            let frame = egui::Frame::NONE
                .fill(t.chrome)
                .stroke(Stroke::new(1.0, if selected { t.accent } else { ui.visuals().widgets.noninteractive.bg_stroke.color }))
                .corner_radius(3)
                .inner_margin(3);
            let row = frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    let menu_width = (16.0 + 2.0 * ui.spacing().button_padding.x).max(26.0);
                    let width = (ui.available_width() - menu_width - ui.spacing().item_spacing.x).max(48.0);
                    let (rect, response) = ui.allocate_exact_size(vec2(width, if info.is_some() { 30.0 } else { 24.0 }), Sense::click());
                    if response.hovered() {
                        ui.painter().rect_filled(rect, 2, ui.visuals().widgets.hovered.weak_bg_fill);
                    }
                    let image_rect = Rect::from_min_size(rect.min + vec2(0.0, 1.0), vec2(28.0, 28.0));
                    if let Some(handle) = &thumbnail {
                        egui::Image::new(handle).paint_at(ui, image_rect);
                    }
                    let name = info.as_ref().map_or("", |(_, name, _, _)| name.as_str());
                    for (text, y, color) in
                        [(label, if info.is_some() { 0.0 } else { 4.0 }, ui.visuals().text_color()), (name, 15.0, ui.visuals().weak_text_color())]
                    {
                        let mut job = egui::text::LayoutJob::simple_singleline(text.into(), egui::FontId::proportional(11.0), color);
                        job.wrap.max_width = (width - 34.0).max(0.0);
                        job.wrap.max_rows = 1;
                        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
                        ui.painter().galley(rect.min + vec2(if info.is_some() { 34.0 } else { 4.0 }, y), galley, color);
                    }
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{label}: {name}")));
                    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(info.as_ref().map_or(
                        tl!("Choose a file or drop an image here").into(),
                        |(_, _, path, size)| {
                            format!("{path}\n{} × {} · {}\n{}", size.width, size.height, tl!("Source thumbnail"), tl!("Click to replace or drop a file"))
                        },
                    ));
                    if response.clicked() {
                        pick = Some(("texture", material, kind));
                    }
                    let button = egui::Button::image(crate::icons::image("ellipsis", 16.0, t.icon)).min_size(vec2(26.0, 24.0));
                    egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
                        if ui.add_enabled(info.is_some(), egui::Button::new(tl!("Edit in 2D"))).clicked() {
                            command(app, state, "uv.material.select", json!({"material":material,"channel":channel}));
                            state.view_3d = false;
                            state.split_view = false;
                            ui.close();
                        }
                        if ui.add_enabled(info.is_some(), egui::Button::new(tl!("Paint on this map"))).clicked() {
                            command(app, state, "uv.material.select", json!({"material":material,"channel":channel}));
                            state.select_shells = false;
                            ui.close();
                        }
                        if ui.add_enabled(app.session.active().is_some(), egui::Button::new(tl!("Use active document"))).clicked() {
                            finish_paint(app);
                            command(app, state, "uv.texture.bind", json!({"material":material,"channel":channel}));
                            ui.close();
                        }
                        if ui.add_enabled(info.is_some(), egui::Button::new(tl!("New layer"))).clicked() {
                            if command(app, state, "uv.material.select", json!({"material":material,"channel":channel})) {
                                command(app, state, "layer.new.layer", json!({"name":"3D paint"}));
                                command(app, state, "channel.target.composite", json!({}));
                            }
                            ui.close();
                        }
                        if ui.button(tl!("New 1K texture")).clicked() {
                            command(app, state, "uv.texture.create", json!({"material":material,"channel":channel}));
                            ui.close();
                        }
                    });
                });
            });
            app.uv_cache.texture_targets.push((row.response.rect, material, kind));
            if hover_at.is_some_and(|p| row.response.rect.contains(p)) {
                ui.painter().rect_stroke(row.response.rect, 3, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
            }
        });
    }
    pick
}

pub(crate) fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if app.ui.workspace != "3D" {
        if ui.button(tl!("Open the 3D workspace")).clicked() {
            let _ = reveal(app);
        }
        return;
    }
    let mut state = std::mem::take(&mut app.ui.uv_workbench);
    state.open = true;
    let t = Tokens::get(ui.ctx());
    let mut pick = None;
    app.uv_cache.texture_targets.clear();
    if !app.session.uv.source.is_empty() {
        ui.label(std::path::Path::new(&app.session.uv.source).file_name().unwrap_or_default().to_string_lossy()).on_hover_text(&app.session.uv.source);
    }
    if !app.uv_cache.source_message.is_empty() {
        ui.colored_label(t.warning, &app.uv_cache.source_message);
    }
    if let (Some(scene), Some(topology)) = (app.session.uv.scene.clone(), app.session.uv.topology.clone()) {
        ui.separator();
        ui.label(crate::i18n::fmt(
            tl!("Triangles: {triangles} · UV shells: {shells}"),
            &[("triangles", &scene.triangles.len().to_string()), ("shells", &topology.shells.len().to_string())],
        ));
        egui::CollapsingHeader::new(tl!("Materials / visibility")).default_open(true).show(ui, |ui| {
            for (id, material) in scene.materials.iter().enumerate() {
                ui.horizontal(|ui| {
                    let mut visible = !app.session.uv.hidden_materials.contains(&id);
                    if ui.checkbox(&mut visible, "").on_hover_text(tl!("Show mesh parts using this material")).changed() {
                        command(app, &mut state, "uv.material.visible", json!({"material":id,"visible":visible}));
                    }
                    if ui.selectable_label(app.session.uv.material == id, &material.name).clicked() {
                        let channel = state.channel_2d.key();
                        command(app, &mut state, "uv.material.select", json!({"material":id,"channel":channel}));
                        state.overlap.clear();
                    }
                });
            }
        });
        let material = app.session.uv.material;
        egui::CollapsingHeader::new(tl!("Material textures")).default_open(true).show(ui, |ui| {
            pick = material_cards(app, &mut state, ui, material);
        });
    } else {
        ui.label(tl!("Import a UV-mapped FBX or OBJ. Materials and available textures are loaded automatically."));
    }
    crate::uv_lighting::panel(app, &mut state, ui);
    if !state.status.is_empty() {
        ui.colored_label(t.danger, &state.status);
    }
    app.ui.uv_workbench = state;
    if let Some((kind, material, normal)) = pick
        && let Err(e) = pick_map_file(app, kind, material, normal)
    {
        app.ui.uv_workbench.status = e;
    }
}
/// Draw the mode switch before the no-document screen, so an untextured mesh is usable.
pub(crate) fn workspace(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> bool {
    crate::uv_workspace::sync_workspace(app);
    if app.ui.workspace != "3D" {
        return false;
    }
    check_source(app);
    flush_mask_settings(app, ui.ctx());
    crate::uv_workspace::show(app, ui)
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Navigation {
    Orbit,
    Pan,
    Dolly,
}
fn preview_size(size: egui::Vec2, pixels_per_point: f32, antialias: bool, texture_limit: usize) -> [usize; 2] {
    let density = if pixels_per_point.is_finite() { pixels_per_point.clamp(0.25, 8.0) } else { 1.0 };
    let multiplier = f64::from(density) * if antialias { 2.0 } else { 1.0 };
    let finite = |v: f32| if v.is_finite() { v.max(1.0) } else { 1.0 };
    let w = f64::from(finite(size.x)) * multiplier;
    let h = f64::from(finite(size.y)) * multiplier;
    let cap = texture_limit.clamp(1, 4096) as f64;
    let limit = cap / w.max(h).max(1.0);
    let scale = limit.min(1.0);
    [(w * scale).round().clamp(1.0, cap) as usize, (h * scale).round().clamp(1.0, cap) as usize]
}
fn navigation_limit(fast: bool, moving: bool, hardware: usize) -> usize {
    if fast && moving { hardware.min(1600) } else { hardware }
}
fn scene_extent(scene: &photocraft_uv::Scene) -> f64 {
    let (lo, hi) = scene.positions.iter().fold(([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]), |(lo, hi), p| {
        ([lo[0].min(p[0]), lo[1].min(p[1]), lo[2].min(p[2])], [hi[0].max(p[0]), hi[1].max(p[1]), hi[2].max(p[2])])
    });
    ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt().max(1e-12)
}
/// A gesture belongs to navigation from Alt+press until mouse release, even if Alt is released first.
struct NavigationResult {
    gesture: bool,
    orbit_started: bool,
}
fn wheel_delta(ui: &egui::Ui) -> f32 {
    let line = ui.ctx().options(|options| options.input_options.line_scroll_speed).clamp(1.0, 500.0);
    let page = ui.clip_rect().height().max(1.0);
    ui.input(|input| {
        input
            .raw
            .events
            .iter()
            .filter_map(|event| {
                let egui::Event::MouseWheel { unit, delta, phase, .. } = event else { return None };
                if *phase == egui::TouchPhase::Cancel {
                    return None;
                }
                let delta = delta.y
                    * match unit {
                        egui::MouseWheelUnit::Point => 1.0,
                        egui::MouseWheelUnit::Line => line,
                        egui::MouseWheelUnit::Page => page,
                    };
                delta.is_finite().then_some(delta.clamp(-2000.0, 2000.0))
            })
            .sum::<f32>()
            .clamp(-2000.0, 2000.0)
    })
}
fn navigate(ui: &egui::Ui, response: &egui::Response, state: &mut State, nav: &mut Option<Navigation>, extent: f64) -> NavigationResult {
    let previous = nav.is_some();
    let scroll = wheel_delta(ui);
    let (alt, pressed, left, middle, right, delta, scroll) = ui.input(|i| {
        (i.modifiers.alt, i.pointer.any_pressed(), i.pointer.primary_down(), i.pointer.middle_down(), i.pointer.secondary_down(), i.pointer.delta(), scroll)
    });
    let mut started = false;
    if alt && pressed && response.contains_pointer() {
        *nav = if right || (left && middle) {
            Some(Navigation::Dolly)
        } else if middle {
            Some(Navigation::Pan)
        } else if left {
            Some(Navigation::Orbit)
        } else {
            None
        };
        started = nav.is_some();
    }
    let held = match nav {
        Some(Navigation::Orbit) => left,
        Some(Navigation::Pan) => middle,
        Some(Navigation::Dolly) => right || (left && middle),
        None => false,
    };
    if !held {
        *nav = None;
    }
    if !started && let Some(mode) = nav {
        match mode {
            Navigation::Orbit => {
                state.yaw -= f64::from(delta.x) * 0.008;
                state.pitch = (state.pitch + f64::from(delta.y) * 0.008).clamp(-1.55, 1.55);
            }
            Navigation::Pan => {
                let unit = extent / (f64::from(response.rect.width().min(response.rect.height()).max(1.0)) * 0.8 * state.zoom);
                state.pan[0] += f64::from(delta.x) * unit;
                state.pan[1] -= f64::from(delta.y) * unit;
            }
            Navigation::Dolly => {
                state.zoom = (state.zoom * (f64::from(delta.x - delta.y) * 0.01).exp()).clamp(0.01, if state.perspective { 1_000_000.0 } else { 100.0 })
            }
        }
    }
    if response.hovered() {
        state.zoom = (state.zoom * (f64::from(scroll) * 0.002).exp()).clamp(0.01, if state.perspective { 1_000_000.0 } else { 100.0 });
    }
    NavigationResult { gesture: previous || nav.is_some() || started, orbit_started: started && *nav == Some(Navigation::Orbit) }
}
/// One BVH query on orbit press, never a GPU readback or a per-frame mesh scan.
fn surface_pivot(
    state: &mut State,
    geometry: &photocraft_uv::prepared::Geometry,
    scene: &photocraft_uv::Scene,
    maps: &preview::Materials<'_>,
    cursor: Option<[f64; 2]>,
    size: [u32; 2],
) {
    let at = match state.orbit_pivot {
        OrbitPivot::ViewCenter => [0.5, 0.5],
        OrbitPivot::Cursor => match cursor {
            Some(p) => p,
            None => return,
        },
        OrbitPivot::Fixed => return,
    };
    if let Some(hit) = geometry.hit(scene, state.camera(), maps, at, size) {
        let camera = geometry.retarget(state.camera(), hit.position);
        state.target = camera.target;
        state.pan = camera.pan;
        state.zoom = camera.zoom;
    }
}

fn frame_model(app: &mut PhotocraftApp, selected: bool) {
    let (Some(scene), Some(topology)) = (app.session.uv.scene.as_ref(), app.session.uv.topology.as_ref()) else { return };
    let state = &mut app.ui.uv_workbench;
    state.pan = [0.0; 2];
    state.target = None;
    state.zoom = 1.0;
    if !selected || app.session.uv.selected.is_empty() {
        return;
    }
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    let mut found = false;
    for (id, triangle) in scene.triangles.iter().enumerate() {
        if !topology.triangle_shell.get(id).is_some_and(|s| app.session.uv.selected.contains(s)) || app.session.uv.hidden_materials.contains(&triangle.material)
        {
            continue;
        }
        for corner in triangle.corners {
            if let Some(p) = scene.positions.get(corner.vertex) {
                for axis in 0..3 {
                    lo[axis] = lo[axis].min(p[axis]);
                    hi[axis] = hi[axis].max(p[axis]);
                }
                found = true;
            }
        }
    }
    if found {
        state.target = Some(std::array::from_fn(|i| (lo[i] + hi[i]) * 0.5));
        let extent = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt().max(1e-12);
        state.zoom = (scene_extent(scene) / extent).clamp(0.01, 100.0);
    }
}
/// Give the 3D viewport F/A before Photoshop's screen-mode/tool shortcuts.
pub(crate) fn keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    if app.ui.workspace != "3D" {
        return false;
    }
    if crate::uv_workspace::keys(app, ctx) {
        return true;
    }
    if (!app.ui.uv_workbench.view_3d && !app.ui.uv_workbench.split_view && !app.ui.uv_workbench.floating_3d)
        || app.session.uv.scene.is_none()
        || app.uv_cache.viewport_id.is_some_and(|id| id != ctx.viewport_id())
        || !ctx.input(|i| i.pointer.hover_pos().is_some_and(|p| app.uv_cache.viewport.is_some_and(|r| r.contains(p))))
    {
        return false;
    }
    let f = crate::uv_workspace::plain_key(ctx, egui::Key::F);
    let a = crate::uv_workspace::plain_key(ctx, egui::Key::A);
    if f || a {
        frame_model(app, f);
        return true;
    }
    false
}

pub(crate) fn main_view(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if ui.rect_contains_pointer(ui.available_rect_before_wrap()) {
        app.hover_doc = None;
        app.drop_canvas_rect = None;
    }
    let (Some(scene), Some(topology)) = (app.session.uv.scene.clone(), app.session.uv.topology.clone()) else { return };
    let mut state = std::mem::take(&mut app.ui.uv_workbench);
    let mut cache = std::mem::take(&mut app.uv_cache);
    let size = ui.available_size().max(vec2(1.0, 1.0));
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    cache.viewport = Some(rect);
    cache.viewport_id = Some(ui.ctx().viewport_id());
    if cache.geometry.as_ref().is_some_and(|(s, _)| !std::sync::Arc::ptr_eq(s, &scene)) {
        crate::uv_paint::finish(app, &mut cache.paint);
        cache.geometry = None;
        if let Some(gpu) = &mut cache.gpu {
            gpu.clear();
        }
        cache.refreshed = None;
    }
    if cache.geometry.as_ref().is_none_or(|(s, _)| !std::sync::Arc::ptr_eq(s, &scene)) {
        match photocraft_uv::prepared::Geometry::new(&scene, &topology) {
            Ok(g) => cache.geometry = Some((scene.clone(), std::sync::Arc::new(g))),
            Err(e) => {
                state.status = e.to_string();
                cache.gpu = None;
            }
        }
    }
    cache.display_channel = state.channel;
    if let Some(e) = crate::uv_lighting::poll(&mut cache.lighting, ui.ctx()) {
        state.status = e;
    }
    // Canvas presses release numeric/text focus, so the next Undo belongs to the
    // document rather than a previously edited brush-size field.
    if ui.input(|i| i.pointer.any_pressed())
        && response.contains_pointer()
        && ui.ctx().layer_id_at(ui.input(|i| i.pointer.interact_pos()).unwrap_or(rect.center())).is_none_or(|layer| layer == ui.layer_id())
    {
        ui.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
    }
    let extent = cache.geometry.as_ref().map_or_else(|| scene_extent(&scene), |(_, g)| g.diameter);
    let resizing = crate::uv_paint::resize_brush(app, ui, &response, !state.select_shells);
    let navigation =
        if resizing { NavigationResult { gesture: true, orbit_started: false } } else { navigate(ui, &response, &mut state, &mut cache.navigation, extent) };
    let wheel = if response.hovered() { wheel_delta(ui) } else { 0.0 };
    if wheel != 0.0 {
        cache.wheel_active = Some(Instant::now());
    } else if response.contains_pointer() && ui.input(|i| i.pointer.any_pressed()) {
        cache.wheel_active = None;
    }
    let moving = cache.navigation.is_some() || cache.wheel_active.is_some_and(|at| at.elapsed() < Duration::from_millis(120));
    // The quality settle interval owns no input: a click immediately after zoom
    // must still begin a stroke or select a shell.
    let gesture = navigation.gesture || wheel != 0.0;
    if !moving && !crate::adjust_preview::settling(app, 180.0) && cache.refreshed.is_none_or(|at| at.elapsed() >= Duration::from_millis(33)) {
        if let Err(e) = refresh(app, ui.ctx(), &mut cache) {
            state.status = e;
        }
        if let Err(e) = crate::uv_lighting::refresh(app, &mut cache.lighting.surface) {
            state.status = e;
        }
        cache.refreshed = Some(Instant::now());
    }
    let limit = navigation_limit(state.fast_navigation, moving, ui.input(|i| i.max_texture_side));
    let [w, h] = preview_size(size, ui.ctx().pixels_per_point(), cache.gpu.is_none() && state.antialias && !moving, limit);
    let maps = preview::Materials { colors: &cache.textures, normals: &cache.normals, hidden: app.session.uv.hidden_materials.clone() };
    if navigation.orbit_started
        && let Some((_, geometry)) = &cache.geometry
    {
        let cursor = response.interact_pointer_pos().map(|p| {
            let at = (p - rect.min) / rect.size();
            [f64::from(at.x), f64::from(at.y)]
        });
        surface_pivot(&mut state, geometry, &scene, &maps, cursor, [w as u32, h as u32]);
    }
    let camera = state.camera();
    let key = (app.session.uv.generation, camera, w, h, state.select_shells);
    let mut gpu_image = None;
    let no_highlight = std::collections::BTreeSet::new();
    let highlight = if state.select_shells { &app.session.uv.selected } else { &no_highlight };
    if let (Some(renderer), Some((_, geometry))) = (&mut cache.gpu, &cache.geometry) {
        let draw = crate::uv_gpu::Draw {
            scene: &scene,
            geometry: geometry.clone(),
            camera,
            size: [w as u32, h as u32],
            antialias: state.antialias && !moving,
            selected: highlight,
            hidden: &app.session.uv.hidden_materials,
            generation: app.session.uv.generation,
            colors: &cache.textures,
            normals: &cache.normals,
            keys: &cache.keys,
            normal_keys: &cache.normal_keys,
            masked: &cache.masked,
        };
        let environment = crate::uv_lighting::ensure(&mut cache.lighting);
        let result = renderer.set_lighting(state.lighting, &cache.lighting.surface, environment).and_then(|()| renderer.draw(draw));
        match result {
            Ok(id) => gpu_image = Some(id),
            Err(e) => {
                state.status = format!("3D GPU: {e}. CPU fallback.");
                cache.gpu = None;
            }
        }
    }
    if gpu_image.is_none() && cache.rendered != Some(key) {
        let selected: Vec<bool> = topology.triangle_shell.iter().map(|s| highlight.contains(s)).collect();
        let fallback_camera = if cache.masked.is_empty() { camera } else { Camera { channel: Channel::Color, shaded: false, ..camera } };
        match preview::render_materials(&scene, fallback_camera, &maps, &selected, w, h) {
            Ok(frame) => {
                let data: Vec<u8> = frame.rgba.iter().flatten().copied().collect();
                let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &data);
                if let Some(handle) = cache.frame_handle.as_mut() {
                    handle.set(image, TextureOptions::LINEAR);
                } else {
                    cache.frame_handle = Some(ui.ctx().load_texture("uv-main-3d", image, TextureOptions::LINEAR));
                }
                cache.frame = Some(frame);
                cache.rendered = Some(key);
            }
            Err(e) => state.status = e.to_string(),
        }
    }
    if let Some(id) = gpu_image {
        ui.painter().image(id, rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
    } else if let Some(handle) = &cache.frame_handle {
        ui.painter().image(handle.id(), rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
    }
    if state.select_shells && !gesture {
        if let Some((area, mods)) = crate::uv_box::input(ui, &response, &mut cache.shell_box_3d)
            && let Some((_, g)) = &cache.geometry
        {
            let material = app.session.uv.material;
            crate::uv_box::select(app, &mut state, area, mods, material, |id| {
                let tri = g.triangles.get(id)?;
                let points: Option<Vec<_>> = tri
                    .positions
                    .iter()
                    .map(|p| {
                        let q = g.project_normalized(camera, p.map(f64::from), [w as u32, h as u32])?;
                        Some(rect.min + vec2(q[0] as f32 * rect.width(), q[1] as f32 * rect.height()))
                    })
                    .collect();
                points?.try_into().ok()
            });
        }
    } else {
        cache.shell_box_3d = None;
    }
    if response.clicked()
        && state.select_shells
        && !gesture
        && !ui.input(|i| i.modifiers.alt)
        && let Some(p) = response.interact_pointer_pos()
    {
        let at = (p - rect.min) / rect.size();
        let triangle = if gpu_image.is_some() {
            cache.geometry.as_ref().and_then(|(_, g)| g.pick(&scene, camera, &maps, [f64::from(at.x), f64::from(at.y)], [w as u32, h as u32]))
        } else {
            cache.frame.as_ref().and_then(|frame| {
                let x = ((at.x * frame.width as f32) as usize).min(frame.width - 1);
                let y = ((at.y * frame.height as f32) as usize).min(frame.height - 1);
                frame.triangles.get(y * frame.width + x).copied().flatten()
            })
        };
        if let Some(triangle) = triangle
            && let Some(&shell) = topology.triangle_shell.get(triangle)
            && let Some(sh) = topology.shells.get(shell)
        {
            choose(app, &mut state, ui, sh.material, shell);
        }
    }
    if let Some((_, geometry)) = &cache.geometry {
        crate::uv_paint::input(
            app,
            ui,
            &response,
            &mut cache.paint,
            geometry,
            &scene,
            camera,
            &maps,
            [w as u32, h as u32],
            !state.select_shells && !gesture && cache.navigation.is_none() && !ui.input(|i| i.modifiers.alt),
            state.paint_wrap,
        );
    }
    if !state.status.is_empty() {
        ui.painter().text(
            rect.left_bottom() + vec2(12.0, -14.0),
            egui::Align2::LEFT_BOTTOM,
            &state.status,
            egui::FontId::proportional(13.0),
            Tokens::get(ui.ctx()).danger,
        );
    }
    if cache.navigation.is_some() {
        ui.ctx().request_repaint();
    }
    let picker = response.secondary_clicked()
        && !gesture
        && !state.select_shells
        && !ui.input(|i| i.modifiers.alt || i.modifiers.ctrl)
        && crate::paint_mouse::has_brush_picker(app.ui.tool);
    app.ui.uv_workbench = state;
    app.uv_cache = cache;
    if picker && let Some(p) = response.interact_pointer_pos() {
        crate::brush_picker::open(&mut app.ui, ui.ctx(), [p.x, p.y]);
        app.ui.uv_workbench.picker_3d = true;
    }
    ui.ctx().request_repaint_after(Duration::from_millis(33));
}

/// Resolve by the displayed document, rather than the currently chosen material. Other
/// image tabs never acquire unrelated UVs. Several materials may share the same image.
fn canvas_materials(app: &PhotocraftApp, index: usize) -> Vec<usize> {
    let Some(document) = app.session.documents().get(index) else { return Vec::new() };
    (0..app.session.uv.bindings.len())
        .filter(|m| Channel::MAPS.into_iter().any(|kind| app.session.uv.map_bindings(kind).get(*m).copied().flatten() == Some(document.doc.id)))
        .collect()
}

/// View-only overlay: it never changes the image pixels or the persistent seam mask.
pub(crate) fn canvas_overlay(app: &mut PhotocraftApp, ui: &mut egui::Ui, xf: &crate::canvas::ViewXform, index: usize, _primary: bool) -> Option<Rect> {
    if app.ui.workspace != "3D" {
        return None;
    }
    let topology = app.session.uv.topology.clone()?;
    let materials = canvas_materials(app, index);
    if materials.is_empty() {
        return None;
    }
    let document = app.session.documents().get(index)?;
    let size = [document.doc.size.width, document.doc.size.height];
    let t = Tokens::get(ui.ctx());
    if app.ui.uv_workbench.show_overlay {
        let painter = ui.painter_at(xf.rect);
        for edge in &topology.boundaries {
            if !topology.shells.get(edge.shell).is_some_and(|s| materials.contains(&s.material)) {
                continue;
            }
            let Some(segment) = clip_tile(edge.uv) else { continue };
            let points = segment.map(|uv| xf.to_screen((uv[0] * f64::from(size[0])) as f32, ((1.0 - uv[1]) * f64::from(size[1])) as f32));
            let selected = app.session.uv.selected.contains(&edge.shell);
            // A contrasting backing keeps the wire visible over light and dark maps.
            painter.line_segment(points, Stroke::new(if selected { 3.5 } else { 2.5 }, t.canvas));
            painter.line_segment(points, Stroke::new(if selected { 2.0 } else { 1.0 }, if selected { t.accent } else { t.text }));
        }
    }
    None
}

fn selection_mode(app: &PhotocraftApp, index: usize) -> bool {
    app.ui.uv_workbench.show_overlay
        && app.ui.uv_workbench.select_shells
        && app.session.uv.topology.is_some()
        && !canvas_materials(app, index).is_empty()
        && app.drag.is_none()
        && app.ui.transform.is_none()
}

/// Called before the ordinary tool's press handling, so a shell click cannot paint.
/// Space/middle-button navigation and modal tools are handled by the canvas first.
pub(crate) fn canvas_input(
    app: &mut PhotocraftApp,
    ui: &egui::Ui,
    xf: &crate::canvas::ViewXform,
    response: &egui::Response,
    index: usize,
    toolbar: Option<Rect>,
) -> bool {
    if app.ui.workspace != "3D" {
        return false;
    }
    let over_toolbar = ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| toolbar.is_some_and(|r| r.contains(p)));
    if over_toolbar {
        return true;
    }
    if !selection_mode(app, index) {
        return false;
    }
    if let Some((area, mods)) = crate::uv_box::input(ui, response, &mut app.uv_cache.shell_box_2d) {
        let scene = app.session.uv.scene.clone();
        let doc = app.session.documents().get(index).map(|st| st.doc.clone());
        if let (Some(scene), Some(doc)) = (scene, doc) {
            let materials = canvas_materials(app, index);
            let material = if materials.contains(&app.session.uv.material) { app.session.uv.material } else { materials.first().copied().unwrap_or(0) };
            let mut state = std::mem::take(&mut app.ui.uv_workbench);
            crate::uv_box::select(app, &mut state, area, mods, material, |id| {
                let t = scene.triangles.get(id)?;
                Some(t.corners.map(|c| xf.to_screen((c.uv[0] * f64::from(doc.size.width)) as f32, ((1.0 - c.uv[1]) * f64::from(doc.size.height)) as f32)))
            });
            app.ui.uv_workbench = state;
        }
    }
    if response.clicked()
        && let Some(p) = response.interact_pointer_pos()
    {
        pick_canvas(app, ui, index, xf.to_doc(p));
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    true
}

fn pick_canvas(app: &mut PhotocraftApp, ui: &egui::Ui, index: usize, point: [f64; 2]) {
    let (Some(scene), Some(topology), Some(document)) = (app.session.uv.scene.clone(), app.session.uv.topology.clone(), app.session.documents().get(index))
    else {
        return;
    };
    let document_id = document.doc.id;
    let uv = [point[0] / f64::from(document.doc.size.width.max(1)), 1.0 - point[1] / f64::from(document.doc.size.height.max(1))];
    if uv.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
        return;
    }
    let mut state = std::mem::take(&mut app.ui.uv_workbench);
    state.overlap = canvas_materials(app, index).into_iter().flat_map(|m| topology.pick_uv(&scene, uv, m)).collect();
    // Prefer the current material for stacked shells sharing a texture document.
    let shell =
        state.overlap.iter().find(|&&s| topology.shells.get(s).is_some_and(|s| s.material == app.session.uv.material)).or(state.overlap.first()).copied();
    if let Some(shell) = shell
        && let Some(s) = topology.shells.get(shell)
    {
        let current = app.session.uv.current_channel();
        let kind = std::iter::once(current)
            .chain(Channel::MAPS)
            .find(|kind| app.session.uv.map_bindings(*kind).get(s.material).copied().flatten() == Some(document_id))
            .unwrap_or(Channel::Color);
        command(app, &mut state, "uv.material.select", json!({"material":s.material,"channel":kind.key()}));
        choose(app, &mut state, ui, s.material, shell);
    }
    app.ui.uv_workbench = state;
}

/// Clip a wire segment in UV space before applying the canvas's rotation/flip.
fn clip_tile(segment: [[f64; 2]; 2]) -> Option<[[f64; 2]; 2]> {
    let [a, b] = segment;
    let delta = [b[0] - a[0], b[1] - a[1]];
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for (p, q) in [(-delta[0], a[0]), (delta[0], 1.0 - a[0]), (-delta[1], a[1]), (delta[1], 1.0 - a[1])] {
        if p.abs() < 1e-15 {
            if q < 0.0 {
                return None;
            }
        } else if p < 0.0 {
            lo = lo.max(q / p);
        } else {
            hi = hi.min(q / p);
        }
        if lo > hi {
            return None;
        }
    }
    Some([lo, hi].map(|t| [a[0] + delta[0] * t, a[1] + delta[1] * t]))
}

fn choose(app: &mut PhotocraftApp, state: &mut State, ui: &egui::Ui, material: usize, shell: usize) {
    let mode = ui.input(|i| {
        if i.modifiers.alt || (state.view_3d && i.modifiers.ctrl) {
            "subtract"
        } else if i.modifiers.shift {
            "add"
        } else {
            "replace"
        }
    });
    if command(app, state, "uv.shell.select", json!({"material":material,"shells":[shell],"mode":mode})) {
        let p = mask_params(state);
        command(app, state, "uv.mask.apply", p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input_key(h: &mut egui_kittest::Harness<'_, PhotocraftApp>, key: egui::Key, mods: egui::Modifiers) {
        h.event(egui::Event::ModifiersChanged(mods));
        h.event(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: mods });
        h.step();
        h.event(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: mods });
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        h.step();
    }
    #[test]
    fn opacity_affects_render_and_pick_with_and_without_color_but_raw_view_stays_visible() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        let ctx = egui::Context::default();
        let mut cache = Cache::default();
        let scene = app.session.uv.scene.clone().unwrap();
        let g = photocraft_uv::prepared::Geometry::new(&scene, app.session.uv.topology.as_ref().unwrap()).unwrap();
        let camera = Camera::default();
        let t = &g.triangles[0];
        let p = std::array::from_fn(|i| t.positions.iter().map(|p| f64::from(p[i])).sum::<f64>() / 3.0);
        let at = g.project_normalized(camera, p, [400, 400]).unwrap();
        refresh(&mut app, &ctx, &mut cache).unwrap();
        assert!(g.hit(&scene, camera, &preview::Materials { colors: &cache.textures, ..Default::default() }, at, [400, 400]).is_some());
        app.run("uv.texture.create", json!({"channel":"opacity","width":16,"height":16})).unwrap();
        app.run("edit.fill", json!({"color":"#000000"})).unwrap();
        for color in [true, false] {
            if !color {
                app.session.uv.bindings.fill(None);
            }
            cache.display_channel = Channel::Material;
            refresh(&mut app, &ctx, &mut cache).unwrap();
            let maps = preview::Materials { colors: &cache.textures, ..Default::default() };
            assert!(g.hit(&scene, camera, &maps, at, [400, 400]).is_none());
            cache.display_channel = Channel::Opacity;
            refresh(&mut app, &ctx, &mut cache).unwrap();
            let maps = preview::Materials { colors: &cache.textures, ..Default::default() };
            assert!(g.hit(&scene, Camera { channel: Channel::Opacity, ..camera }, &maps, at, [400, 400]).is_some());
            assert!(cache.textures.iter().flatten().flat_map(|im| &im.rgba).all(|p| p[0] == 0.0 && p[3] == 1.0));
        }
    }
    #[test]
    fn compact_slots_wrap_popup_wheel_fast_path_and_single_undo_key() {
        let _gpu = photocraft_testkit::gpu_lock();
        use egui_kittest::kittest::Queryable as _;
        assert!(crate::icons::exists("uv-perspective") && crate::icons::exists("uv-orthographic"));
        let mut app = fixture_app();
        app.ui.workspace = "3D".into();
        crate::menus::apply_workspace(&mut app);
        app.run("uv.texture.create", json!({"channel":"normal","width":32,"height":32})).unwrap();
        app.run("uv.material.select", json!({"channel":"baseColor"})).unwrap();
        app.ui.tool = crate::state::Tool::Brush;
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 1000.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            app.set_uv_wgpu(cc.wgpu_render_state.clone().unwrap()).unwrap();
            app
        });
        h.step();
        h.step();
        assert_eq!(h.state().uv_cache.texture_targets.len(), 9);
        assert!(h.state().uv_cache.texture_targets.iter().all(|(r, _, _)| r.height() <= 40.0));
        let point = h.state().uv_cache.viewport.unwrap().center();
        h.event(egui::Event::PointerMoved(point));
        h.step();
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, 60.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        assert!(h.state().uv_cache.wheel_active.is_some());
        assert_eq!(h.state().uv_cache.gpu.as_ref().unwrap().frame_info().unwrap().1, 1, "wheel uses navigation MSAA policy");
        // Wait egui's smooth-wheel tail out using frames; clear the wall-clock latch for this deterministic harness.
        for _ in 0..40 {
            h.step();
        }
        h.state_mut().uv_cache.wheel_active = None;
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        h.step();
        h.step();
        h.get_by_label("Paint Wrap").click();
        h.step();
        assert!(h.state().ui.uv_workbench.paint_wrap);
        if let Ok(path) = std::env::var("PHOTOCRAFT_UV_WRAP_SCREENSHOT") {
            h.step();
            h.render().unwrap().save(path).unwrap();
        }
        let revision = h.state().session.active().unwrap().revision;
        let dismiss = {
            let g = &h.state().uv_cache.geometry.as_ref().unwrap().1;
            let t = &g.triangles[0];
            let center = std::array::from_fn(|i| t.positions.iter().map(|p| f64::from(p[i])).sum::<f64>() / 3.0);
            let size = h.state().uv_cache.gpu.as_ref().unwrap().frame_info().unwrap().0;
            let at = g.project_normalized(h.state().ui.uv_workbench.camera(), center, size).unwrap();
            let rect = h.state().uv_cache.viewport.unwrap();
            rect.min + vec2(at[0] as f32 * rect.width(), at[1] as f32 * rect.height())
        };
        h.event(egui::Event::PointerMoved(dismiss));
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: dismiss, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        assert!(h.state().ui.brush_picker.is_none());
        assert_eq!(h.state().session.active().unwrap().revision, revision, "dismissing the picker must not paint the model");
        let index = h.state().session.active_index().unwrap();
        let before = h.state().session.documents()[index].doc.clone();
        h.state_mut().run("edit.fill", json!({"color":"#ff0000"})).unwrap();
        input_key(&mut h, egui::Key::Z, egui::Modifiers { ctrl: true, command: true, ..egui::Modifiers::NONE });
        assert_eq!(h.state().session.documents()[index].doc.layers[0].surface(), before.layers[0].surface(), "first Ctrl+Z undoes pixels");
        // Empty History plus a still-held first stroke must enable the *keyboard*
        // command before the live surface is committed by run(edit.undo).
        h.state_mut().ui.uv_workbench.paint_wrap = false;
        let (scene, geometry) = h.state().uv_cache.geometry.as_ref().unwrap();
        let t = &geometry.triangles[0];
        let center = std::array::from_fn(|i| t.positions.iter().map(|p| f64::from(p[i])).sum::<f64>() / 3.0);
        let camera = h.state().ui.uv_workbench.camera();
        let size = h.state().uv_cache.gpu.as_ref().unwrap().frame_info().unwrap().0;
        let at = geometry.project_normalized(camera, center, size).unwrap();
        let rect = h.state().uv_cache.viewport.unwrap();
        let pos = rect.min + vec2(at[0] as f32 * rect.width(), at[1] as f32 * rect.height());
        assert!(geometry.hit(scene, camera, &preview::Materials::default(), at, size).is_some());
        h.event(egui::Event::PointerMoved(pos));
        h.step();
        h.state_mut().uv_cache.wheel_active = Some(Instant::now());
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.step();
        assert!(pending_paint(h.state()));
        input_key(&mut h, egui::Key::Z, egui::Modifiers { ctrl: true, command: true, ..egui::Modifiers::NONE });
        assert!(!pending_paint(h.state()));
        assert_eq!(h.state().session.documents()[index].doc.layers[0].surface(), before.layers[0].surface());
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.step();
    }
    #[test]
    fn ctrl_right_drag_resizes_in_split_3d_and_2d_without_painting() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        app.ui.workspace = "3D".into();
        crate::menus::apply_workspace(&mut app);
        app.ui.uv_workbench.split_view = true;
        app.ui.tool = crate::state::Tool::Brush;
        app.run("tools.setBrush", json!({"brush":{"size":20,"hardness":0.333,"opacity":0.617}})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 900.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            app.set_uv_wgpu(cc.wgpu_render_state.clone().unwrap()).unwrap();
            app
        });
        h.step();
        h.step();
        let revision = h.state().session.active().unwrap().revision;
        for model in [true, false] {
            if !model {
                h.state_mut().ui.uv_workbench.view_3d = false;
                h.state_mut().ui.uv_workbench.split_view = false;
                h.step();
            }
            let at = if model { h.state().uv_cache.viewport.unwrap().center() } else { h.state().last_canvas_rect.center() };
            let camera = (h.state().ui.uv_workbench.yaw, h.state().ui.uv_workbench.pitch, h.state().ui.uv_workbench.zoom);
            let brush = h.state().session.tools.brush.clone();
            h.event(egui::Event::PointerMoved(at));
            h.step();
            h.event(egui::Event::ModifiersChanged(egui::Modifiers { ctrl: true, command: true, ..egui::Modifiers::NONE }));
            h.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Secondary,
                pressed: true,
                modifiers: egui::Modifiers { ctrl: true, command: true, ..egui::Modifiers::NONE },
            });
            h.step();
            h.event(egui::Event::PointerMoved(at + vec2(10.0, 20.0)));
            h.step();
            h.event(egui::Event::PointerMoved(at + vec2(30.0, 60.0)));
            h.step();
            assert!(h.state().session.tools.brush.size > brush.size, "model={model}");
            assert_eq!(h.state().session.tools.brush.hardness, brush.hardness);
            assert_eq!(h.state().session.tools.brush.opacity, brush.opacity);
            // Releasing Ctrl first must not turn the rest of the drag into painting/navigation.
            h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
            h.event(egui::Event::PointerMoved(at + vec2(40.0, 80.0)));
            h.step();
            h.event(egui::Event::PointerButton {
                pos: at + vec2(40.0, 80.0),
                button: egui::PointerButton::Secondary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
            assert!(h.state().brush_resize.is_none() && h.state().drag.is_none() && h.state().ui.brush_picker.is_none());
            assert_eq!(h.state().session.active().unwrap().revision, revision, "resize must not change texture pixels");
            assert_eq!((h.state().ui.uv_workbench.yaw, h.state().ui.uv_workbench.pitch, h.state().ui.uv_workbench.zoom), camera);
        }
    }
    #[derive(Debug)]
    struct MapDrop(std::path::PathBuf);
    impl egui::DroppedFile for MapDrop {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            Err("native import uses the path".into())
        }
    }
    #[test]
    fn material_cards_picker_drop_source_thumbnails_and_scoped_tabless_views() {
        let _gpu = photocraft_testkit::gpu_lock();
        use crate::file_dialog::FileDialogAnswer;
        use egui_kittest::kittest::Queryable as _;
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/checker.png");
        let (dialog, asked) = crate::file_dialog::fake(vec![Some(FileDialogAnswer::Paths(vec![source.to_string_lossy().into_owned()]))]);
        let mut app = fixture_app();
        app.session.uv.normal_bindings.fill(None); // Explicitly exercise the absent-channel state despite sibling auto-discovery.
        app.services.file_dialog = Some(dialog);
        app.ui.workspace = "3D".into();
        crate::menus::apply_workspace(&mut app);
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 900.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            app.set_uv_wgpu(cc.wgpu_render_state.clone().unwrap()).unwrap();
            app
        });
        h.step();
        h.step();
        assert!(h.query_by_label("UV and seam protection").is_none());
        assert!(h.query_by_label("3D view / normals").is_none());
        assert!(h.query_by_label("Import information").is_none());
        h.get_by_label("Display channel").click();
        h.step();
        assert!(h.query_by_label("Normal").is_none());
        h.get_by_label("Color").click();
        h.step();
        assert_eq!(h.state().ui.uv_workbench.channel, Channel::Color);
        let material = h.state().session.uv.material;
        let docs = h.state().session.documents().len();
        let rect = h.state().uv_cache.texture_targets.iter().find(|(_, _, n)| *n == Channel::Color).unwrap().0;
        h.event(egui::Event::PointerMoved(rect.center()));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: rect.center(), button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        h.step();
        h.step();
        assert_eq!(asked.borrow().len(), 1, "card click must open a file picker");
        assert_eq!(h.state().session.documents().len(), docs + 1);
        assert!(h.state().uv_gpu_active());
        let rect = h.state().uv_cache.texture_targets.iter().find(|(_, _, n)| *n == Channel::Normal).unwrap().0;
        h.event(egui::Event::PointerMoved(rect.center()));
        h.step();
        h.input_mut().dropped_files.push(std::sync::Arc::new(MapDrop(source)));
        h.step();
        h.step();
        assert_eq!(h.state().session.documents().len(), docs + 2, "drop opens exactly one document");
        assert!(h.state().session.uv.normal_bindings[material].is_some());
        assert!(h.state().session.uv.normal_channel);
        assert!(h.state().drop_places.is_empty());
        assert!(h.state().uv_gpu_active());
        let id = h.state().session.uv.normal_bindings[material].unwrap();
        let thumb = h.state().uv_cache.thumbnails[&(id.0, true)].id();
        h.state_mut().run("layer.setProps", json!({"locks":{"all":false,"pixels":false}})).unwrap();
        h.state_mut().run("edit.fill", json!({"color":"#00ff00"})).unwrap();
        h.step();
        assert_eq!(thumb, h.state().uv_cache.thumbnails[&(id.0, true)].id(), "painting must not regenerate source thumbnails");
        h.get_by_label("Display channel").click();
        h.step();
        h.get_by_label("Normal").click();
        h.step();
        assert_eq!(h.state().ui.uv_workbench.channel, Channel::Normal);
        assert_eq!(h.state().session.active().unwrap().doc.id, id, "2D must follow the selected map");
        h.get_by_label("UV shell selection").click();
        h.step();
        assert!(h.query_by_label("General margin, px").is_some());
        assert!(h.query_by_label("Horizontal margin, px").is_some());
        assert!(h.query_by_label("Vertical margin, px").is_some());
        for removed in ["Apply selection", "Protected layer", "Also protect open borders"] {
            assert!(h.query_by_label(removed).is_none());
        }
        if let Ok(path) = std::env::var("PHOTOCRAFT_UV_MARGINS_SCREENSHOT") {
            h.render().unwrap().save(path).unwrap();
        }
        // Close the tool flyout before checking the canvas layout.
        h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
        h.step();
        let point = h.state().uv_cache.viewport.unwrap().center();
        h.event(egui::Event::PointerMoved(point));
        h.step();
        h.state_mut().run("uv.shell.select", json!({"shells":[0]})).unwrap();
        h.state_mut().ui.uv_workbench.guard = 0.0;
        h.state_mut().ui.uv_workbench.horizontal = 4.0;
        h.state_mut().ui.uv_workbench.vertical = 2.0;
        h.state_mut().uv_cache.mask_settings_pending = true;
        h.step();
        assert_eq!((h.state().session.uv.horizontal, h.state().session.uv.vertical), (4.0, 2.0));
        assert!(h.state().session.active().unwrap().doc.selection.is_some(), "changed margins apply automatically");
        h.state_mut().run("tools.setColors", json!({"foreground":[1.0,0.0,0.0,1.0]})).unwrap();
        let projection = h.state().ui.uv_workbench.perspective;
        input_key(&mut h, egui::Key::F5, egui::Modifiers::SHIFT);
        assert_eq!(h.state().ui.uv_workbench.perspective, projection, "Shift+F5 must not switch the camera");
        assert!(h.state().ui.dialogs.iter().any(|d| crate::fill_ui::owns(&d.fields)), "shell mode opens native Fill");
        h.get_by_label("OK").click();
        h.step();
        let surface = h.state().session.active().unwrap().doc.layers[0].surface().unwrap();
        assert!(surface.rgba(30, 64)[0] > 0.9 && surface.rgba(30, 64)[1] < 0.1);
        assert!(surface.rgba(90, 64)[1] > 0.9, "fill respects the selected shell");
        input_key(&mut h, egui::Key::D, egui::Modifiers { ctrl: true, command: true, ..egui::Modifiers::NONE });
        assert!(h.state().session.uv.selected.is_empty());
        assert!(h.state().session.active().unwrap().doc.selection.is_none());
        h.state_mut().ui.uv_workbench.select_shells = false;
        input_key(&mut h, egui::Key::F5, egui::Modifiers::SHIFT);
        assert!(h.state().ui.dialogs.iter().any(|d| crate::fill_ui::owns(&d.fields)), "paint mode opens native Fill too");
        h.get_by_label("Cancel").click();
        h.step();
        h.state_mut().ui.uv_workbench.view_3d = false;
        h.state_mut().ui.uv_workbench.split_view = false;
        h.step();
        assert!(h.state().tab_strip.is_none(), "UV texture view has no document header");
        if let Ok(path) = std::env::var("PHOTOCRAFT_UV_CARDS_SCREENSHOT") {
            h.render().unwrap().save(path).unwrap();
        }
        h.state_mut().session.uv.scene = None;
        h.state_mut().ui.workspace = "Essentials".into();
        h.step();
        assert!(h.state().tab_strip.is_some(), "ordinary PhotoCraft documents keep their tabs");
    }
    #[test]
    fn fast_navigation_is_optional_and_does_not_limit_idle_or_source_maps() {
        let _gpu = photocraft_testkit::gpu_lock();
        let state: State = serde_json::from_value(json!({})).unwrap();
        assert!(state.fast_navigation);
        assert_eq!(navigation_limit(true, true, 8192), 1600);
        assert_eq!(navigation_limit(true, false, 8192), 8192);
        assert_eq!(navigation_limit(false, true, 8192), 8192);
        assert_eq!(navigation_limit(true, true, 1024), 1024);
    }
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn gpu_refresh_keeps_original_resolution_above_two_k_and_normal_values() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        let material = app.session.uv.material;
        app.run("uv.texture.create", json!({"channel":"normal","width":4096,"height":32,"depth":16})).unwrap();
        let rs = egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default());
        app.set_uv_wgpu(rs).unwrap();
        let mut cache = std::mem::take(&mut app.uv_cache);
        refresh(&mut app, &egui::Context::default(), &mut cache).unwrap();
        let image = cache.normals[material].as_ref().unwrap();
        assert_eq!([image.width, image.height], [4096, 32]);
        assert!((image.rgba[0][0] - 0.5).abs() < 0.001 && image.rgba[0][2] > 0.999);
        assert_eq!(app.session.active().unwrap().doc.size.width, 4096);
    }
    #[test]
    fn pbr_linear_sources_persisted_settings_always_normals_and_panel() {
        let _gpu = photocraft_testkit::gpu_lock();
        use egui_kittest::kittest::Queryable as _;
        let mut app = fixture_app();
        for channel in [Channel::Color, Channel::Metallic, Channel::Roughness, Channel::Emissive] {
            app.run("uv.texture.create", json!({"channel":channel.key(),"width":16,"height":16,"depth":16})).unwrap();
            app.run("edit.fill", json!({"color":"#808080"})).unwrap();
        }
        let mut maps = crate::uv_lighting::SurfaceMaps::default();
        crate::uv_lighting::refresh(&mut app, &mut maps).unwrap();
        let mat = app.session.uv.material;
        assert!((maps.images[0][mat].as_ref().unwrap().rgba[0][0] - 0.21586).abs() < 0.003);
        assert!((maps.images[4][mat].as_ref().unwrap().rgba[0][0] - 0.21586).abs() < 0.003);
        assert!((maps.images[1][mat].as_ref().unwrap().rgba[0][0] - 0.50196).abs() < 0.003);
        let pointer = maps.images[0][mat].as_ref().unwrap().rgba.as_ptr();
        crate::uv_lighting::refresh(&mut app, &mut maps).unwrap();
        assert_eq!(maps.images[0][mat].as_ref().unwrap().rgba.as_ptr(), pointer);
        crate::uv_workspace::restore_preferences(&mut app, Some(&json!({"authored_normals":false,"lighting":{"pbr":false,"rotation":45.0}})));
        assert!(app.ui.uv_workbench.camera().authored_normals);
        assert!(!app.ui.uv_workbench.lighting.pbr);
        let saved = crate::uv_workspace::preferences(&app);
        assert_eq!(saved["lighting"]["rotation"], 45.0);
        assert!(saved.get("authored_normals").is_none());
        app.ui.uv_workbench.lighting.pbr = true;
        let mut h = egui_kittest::Harness::builder().with_size(vec2(450.0, 1000.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.step();
        h.step();
        assert!(h.state().uv_cache.texture_targets.iter().any(|(_, _, channel)| *channel == Channel::Emissive));
        assert!(h.query_by_label("Model normals").is_none());
        assert!(h.query_by_label("HDR / EXR…").is_some());
        if let Ok(path) = std::env::var("PHOTOCRAFT_UV_PBR_SCREENSHOT") {
            h.render().unwrap().save(path).unwrap();
        }
    }
    #[test]
    #[ignore = "explicit full-resolution 4K native brush benchmark"]
    fn benchmark_native_paint_4k_2d_split_and_3d() {
        let _gpu = photocraft_testkit::gpu_lock();
        for mode in 0..3 {
            let mut app = if let Ok(source) = std::env::var("PHOTOCRAFT_UV_PAINT_MODEL") {
                let mut session = photocraft_engine::Session::new();
                session.execute("uv.model.import", json!({"path":source})).unwrap();
                PhotocraftApp::new(session, Default::default())
            } else {
                fixture_app()
            };
            app.run("uv.texture.create", json!({"width":4096,"height":4096,"depth":16})).unwrap();
            app.run("tools.setColors", json!({"foreground":"#ff0000"})).unwrap();
            app.run("tools.setBrush", json!({"brush":{"size":32.0,"hardness":0.5}})).unwrap();
            app.ui.uv_workbench.view_3d = mode != 0;
            app.ui.uv_workbench.split_view = mode == 1;
            app.ui.tool = crate::state::Tool::Brush;
            let shared = std::sync::Arc::new(std::sync::Mutex::new(None));
            let capture = shared.clone();
            let mut h = egui_kittest::Harness::builder().with_size(vec2(1600.0, 1000.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                let rs = cc.wgpu_render_state.clone().unwrap();
                *capture.lock().unwrap() = Some(rs.clone());
                app.set_wgpu(rs);
                app
            });
            h.step();
            h.step();
            let rs = shared.lock().unwrap().clone().unwrap();
            let point = if mode == 0 {
                pos2(600.0, 450.0)
            } else {
                let app = h.state();
                let (_, g) = app.uv_cache.geometry.as_ref().unwrap();
                let camera = app.ui.uv_workbench.camera();
                let size = app.uv_cache.gpu.as_ref().unwrap().frame_info().unwrap().0;
                let maps = preview::Materials { colors: &app.uv_cache.textures, ..Default::default() };
                let at = g
                    .triangles
                    .iter()
                    .enumerate()
                    .find_map(|(id, t)| {
                        let p = std::array::from_fn(|axis| t.positions.iter().map(|p| f64::from(p[axis])).sum::<f64>() / 3.0);
                        let at = g.project_normalized(camera, p, size)?;
                        (t.material == app.session.uv.material && g.pick(app.session.uv.scene.as_ref().unwrap(), camera, &maps, at, size) == Some(id))
                            .then_some(at)
                    })
                    .unwrap();
                let rect = app.uv_cache.viewport.unwrap();
                rect.min + vec2(at[0] as f32 * rect.width(), at[1] as f32 * rect.height())
            };
            h.event(egui::Event::PointerMoved(point));
            h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
            h.step();
            assert!(h.state().drag.is_some());
            let mut times = Vec::new();
            for frame in 0..24 {
                h.state_mut().uv_cache.refreshed = None;
                h.event(egui::Event::PointerMoved(point + vec2(frame as f32 * 0.5, frame as f32 * 0.15)));
                let start = Instant::now();
                h.step();
                rs.device.poll(eframe::wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
                if frame >= 4 {
                    times.push(start.elapsed().as_secs_f64() * 1000.0);
                }
            }
            times.sort_by(f64::total_cmp);
            println!("4K U16 paint mode={mode} CPU+completed GPU median={:.2}ms p95={:.2}ms", times[times.len() / 2], times[times.len() * 95 / 100]);
            h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
            h.step();
            assert!(h.state().drag.is_none());
        }
    }

    #[test]
    fn independent_2d_channel_follows_global_changes_and_round_trips_preferences() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        app.run("uv.texture.create", json!({"channel":"normal","width":64,"height":64})).unwrap();
        let normal = app.session.active().unwrap().doc.id;
        select_view_channel(&mut app, Channel::Normal);
        assert_eq!(app.ui.uv_workbench.channel_2d, Channel::Normal);
        select_2d_channel(&mut app, Channel::Color);
        assert_eq!(app.ui.uv_workbench.channel, Channel::Normal);
        assert_ne!(app.session.active().unwrap().doc.id, normal);
        select_view_channel(&mut app, Channel::Material);
        assert_eq!(app.ui.uv_workbench.channel_2d, Channel::Color);
        select_view_channel(&mut app, Channel::Normal);
        assert_eq!(app.session.active().unwrap().doc.id, normal);
        let prefs = crate::uv_workspace::preferences(&app);
        app.ui.uv_workbench.channel_2d = Channel::Color;
        crate::uv_workspace::restore_preferences(&mut app, Some(&prefs));
        assert_eq!(app.ui.uv_workbench.channel_2d, Channel::Normal);
        let mut legacy = prefs;
        legacy.as_object_mut().unwrap().remove("channel_2d");
        app.ui.uv_workbench.channel_2d = Channel::Color;
        crate::uv_workspace::restore_preferences(&mut app, Some(&legacy));
        assert_eq!(app.ui.uv_workbench.channel_2d, Channel::Normal);
    }
    #[test]
    fn uv_mask_view_replaces_live_map_and_restores_material_without_touching_texture() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        let ctx = egui::Context::default();
        app.run("uv.shell.select", json!({"shells":[0]})).unwrap();
        app.run("uv.layer.create", json!({"guard":8})).unwrap();
        app.run("view.layerMask", json!({"mode":"gray"})).unwrap();
        let doc = app.session.active().unwrap().doc.clone();
        let material = app.session.uv.material;
        let mask = doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.clone();
        let mut cache = Cache { display_channel: Channel::Material, ..Default::default() };
        refresh(&mut app, &ctx, &mut cache).unwrap();
        assert!(cache.masked.contains(&material));
        let im = cache.textures[material].as_ref().unwrap();
        for (x, y) in [(30, 64), (52, 64)] {
            let p = im.rgba[y * im.width + x];
            let g = mask.pixel(x as i32, y as i32)[0];
            assert!((p[0] - g).abs() < 0.01 && p[0] == p[1] && p[1] == p[2] && p[3] == 1.0, "mask shown on mesh at {x},{y}");
        }
        assert!(std::sync::Arc::ptr_eq(&doc, &app.session.active().unwrap().doc));
        app.run("channel.target.layerMask", json!({})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        app.run("tools.setColors", json!({"foreground":"#000000"})).unwrap();
        app.run("tools.setBrush", json!({"brush":{"size":8.0,"hardness":1.0}})).unwrap();
        app.ui.tool = crate::state::Tool::Brush;
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Down { x: 30.0, y: 64.0, pressure: 1.0 }, egui::Modifiers::NONE);
        refresh(&mut app, &ctx, &mut cache).unwrap();
        assert!(cache.textures[material].as_ref().unwrap().rgba[64 * 128 + 30][0] < 0.01, "3D mask display includes a live native brush stroke");
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Up { x: 30.0, y: 64.0 }, egui::Modifiers::NONE);
        app.run("edit.undo", json!({})).unwrap();
        refresh(&mut app, &ctx, &mut cache).unwrap();
        assert!(cache.textures[material].as_ref().unwrap().rgba[64 * 128 + 30][0] > 0.99, "Undo refreshes mask display");
        let old = cache.keys[material];
        app.run("view.layerMask", json!({"mode":"off"})).unwrap();
        refresh(&mut app, &ctx, &mut cache).unwrap();
        assert!(cache.masked.is_empty());
        assert_ne!(cache.keys[material], old);
    }
    fn fixture_app() -> PhotocraftApp {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.fbx");
        let mut session = photocraft_engine::Session::new();
        session.execute("uv.model.import", json!({"path":source.to_string_lossy()})).unwrap();
        {
            let mut app = PhotocraftApp::new(session, crate::Services::default());
            app.ui.workspace = "3D".into();
            app
        }
    }
    #[test]
    fn pbr_live_brush_updates_only_damage_and_reuses_source_buffers() {
        let _gpu = photocraft_testkit::gpu_lock();
        for channel in [Channel::Color, Channel::Metallic, Channel::Emissive] {
            let mut app = fixture_app();
            app.run("uv.texture.create", json!({"channel":channel.key(),"width":1024,"height":1024,"depth":16})).unwrap();
            app.run("tools.setColors", json!({"foreground":"#ff0000"})).unwrap();
            app.run("tools.setBrush", json!({"brush":{"size":16.0,"hardness":1.0}})).unwrap();
            app.ui.tool = crate::state::Tool::Brush;
            let mut maps = crate::uv_lighting::SurfaceMaps::default();
            let material = app.session.uv.material;
            let kind = crate::uv_lighting::MAPS.iter().position(|kind| *kind == channel).unwrap();
            crate::uv_lighting::refresh(&mut app, &mut maps).unwrap();
            let pointer = maps.images[kind][material].as_ref().unwrap().rgba.as_ptr();
            crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Down { x: 200.0, y: 200.0, pressure: 1.0 }, egui::Modifiers::NONE);
            crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Move { x: 215.0, y: 205.0, pressure: 1.0 }, egui::Modifiers::NONE);
            crate::uv_lighting::refresh(&mut app, &mut maps).unwrap();
            let (_, regions) = maps.regions[kind][material].clone().expect("live stroke must propagate its damage");
            let region = regions.first().copied().unwrap_or([0; 4]);
            assert!(region[2] * region[3] < 65536, "small dabs cannot recompose a 1K map: {region:?}");
            assert_eq!(maps.images[kind][material].as_ref().unwrap().rgba.as_ptr(), pointer);
            assert!(maps.images[kind][material].as_ref().unwrap().rgba[200 * 1024 + 200][1] < 0.02);
            crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Up { x: 215.0, y: 205.0 }, egui::Modifiers::NONE);
            crate::uv_lighting::refresh(&mut app, &mut maps).unwrap();
            assert_eq!(maps.images[kind][material].as_ref().unwrap().rgba.as_ptr(), pointer, "commit preserves the live texture cache");
            app.run("edit.undo", json!({})).unwrap();
            crate::uv_lighting::refresh(&mut app, &mut maps).unwrap();
            assert!(maps.images[kind][material].as_ref().unwrap().rgba[200 * 1024 + 200][1] > 0.9);
        }
    }
    #[test]
    fn painting_in_3d_keeps_zoomed_split_2d_view_fixed_on_both_sides() {
        let _gpu = photocraft_testkit::gpu_lock();
        for swapped in [false, true] {
            let mut app = fixture_app();
            app.run("uv.texture.create", json!({"width":512,"height":512})).unwrap();
            app.run("tools.setColors", json!({"foreground":"#ff0000"})).unwrap();
            app.ui.uv_workbench.split_view = true;
            app.ui.uv_workbench.swap_views = swapped;
            app.ui.uv_workbench.antialias = false;
            app.ui.tool = crate::state::Tool::Brush;
            let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 900.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                app.set_uv_wgpu(cc.wgpu_render_state.clone().unwrap()).unwrap();
                app
            });
            h.step();
            h.step();
            for view in &mut h.state_mut().ui.views {
                view.zoom = 8.0;
                view.center = [220.0, 240.0];
                view.fit_pending = false;
            }
            h.step();
            let app = h.state();
            let (_, g) = app.uv_cache.geometry.as_ref().unwrap();
            let camera = app.ui.uv_workbench.camera();
            let size = app.uv_cache.gpu.as_ref().unwrap().frame_info().unwrap().0;
            let maps = preview::Materials { colors: &app.uv_cache.textures, ..Default::default() };
            let at = g
                .triangles
                .iter()
                .enumerate()
                .find_map(|(id, t)| {
                    let p = std::array::from_fn(|axis| t.positions.iter().map(|p| f64::from(p[axis])).sum::<f64>() / 3.0);
                    let at = g.project_normalized(camera, p, size)?;
                    (t.material == app.session.uv.material && g.pick(app.session.uv.scene.as_ref().unwrap(), camera, &maps, at, size) == Some(id)).then_some(at)
                })
                .expect("visible model face");
            let rect = app.uv_cache.viewport.unwrap();
            let point = rect.min + vec2(at[0] as f32 * rect.width(), at[1] as f32 * rect.height());
            let centers: Vec<_> = app.ui.views.iter().map(|view| view.center).collect();
            h.event(egui::Event::PointerMoved(point));
            h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
            h.step();
            assert!(h.state().drag.is_some());
            for _ in 0..8 {
                h.step();
            }
            assert_eq!(centers, h.state().ui.views.iter().map(|view| view.center).collect::<Vec<_>>());
            h.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
            h.step();
            assert!(h.state().drag.is_none());
        }
    }
    #[test]
    fn main_3d_pointer_uses_native_brush_and_refreshes_the_bound_gpu_map() {
        let _gpu = photocraft_testkit::gpu_lock();
        use egui_kittest::kittest::Queryable as _;
        let source = std::env::var_os("PHOTOCRAFT_UV_PAINT_MODEL")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj"));
        let side = if std::env::var_os("PHOTOCRAFT_UV_PAINT_MODEL").is_some() { 4096 } else { 128 };
        let mut session = photocraft_engine::Session::new();
        session.execute("uv.model.import", json!({"path":source})).unwrap();
        session.execute("uv.texture.create", json!({"width":side,"height":side})).unwrap();
        session.execute("tools.setColors", json!({"foreground":[1.0,0.0,0.0,1.0]})).unwrap();
        session.execute("tools.setBrush", json!({"brush":{"size":8,"hardness":1.0}})).unwrap();
        let mut harness = egui_kittest::Harness::builder().with_size(vec2(1200.0, 850.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(session, Default::default());
            app.ui.workspace = "3D".into();
            app.set_uv_wgpu(cc.wgpu_render_state.clone().unwrap()).unwrap();
            app.ui.uv_workbench.view_3d = true;
            app.ui.uv_workbench.split_view = true;
            app.ui.uv_workbench.antialias = false;
            app.ui.uv_workbench.yaw = 0.0;
            app.ui.uv_workbench.pitch = 0.0;
            app.ui.tool = crate::state::Tool::Brush;
            app
        });
        harness.step();
        let app = harness.state();
        let rect = app.uv_cache.viewport.unwrap();
        let g = &app.uv_cache.geometry.as_ref().unwrap().1;
        let size = app.uv_cache.gpu.as_ref().unwrap().frame_info().unwrap().0;
        let camera = Camera { yaw: 0.0, pitch: 0.0, ..Default::default() };
        let c = g.camera_values(camera, size);
        let scene = app.session.uv.scene.as_ref().unwrap();
        let maps = preview::Materials { colors: &app.uv_cache.textures, normals: &app.uv_cache.normals, ..Default::default() };
        let triangle = g
            .triangles
            .iter()
            .enumerate()
            .find_map(|(id, t)| {
                if t.material != app.session.uv.material {
                    return None;
                }
                let p: [f64; 2] = std::array::from_fn(|i| t.positions.iter().map(|p| f64::from(p[i])).sum::<f64>() / 3.0);
                let at = [((p[0] + f64::from(c[6])) * f64::from(c[4]) + 1.0) * 0.5, (1.0 - (p[1] + f64::from(c[7])) * f64::from(c[5])) * 0.5];
                if at.iter().all(|v| (0.0..=1.0).contains(v)) && g.pick(scene, camera, &maps, at, size) == Some(id) { Some(t) } else { None }
            })
            .expect("visible triangle of the active material");
        let xy: [f64; 2] = std::array::from_fn(|i| triangle.positions.iter().map(|p| f64::from(p[i])).sum::<f64>() / 3.0);
        let point = rect.min
            + vec2(
                ((xy[0] + f64::from(c[6])) * f64::from(c[4]) + 1.0) as f32 * 0.5 * rect.width(),
                (1.0 - (xy[1] + f64::from(c[7])) * f64::from(c[5])) as f32 * 0.5 * rect.height(),
            );
        let uv: [f64; 2] = std::array::from_fn(|i| triangle.uv.iter().map(|p| f64::from(p[i])).sum::<f64>() / 3.0);
        let pixel = [(uv[0] * f64::from(side)) as i32, ((1.0 - uv[1]) * f64::from(side)) as i32];
        let material = app.session.uv.material;
        let before = app.session.active().unwrap().doc.clone();
        harness.event(egui::Event::PointerMoved(point));
        harness.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        harness.step();
        assert!(harness.state().live_stroke.is_some(), "3D press must reach native live brush");
        let index = harness.state().session.active_index().unwrap();
        let (shown, key) = crate::canvas::display_doc(harness.state_mut(), index);
        assert_ne!(key, 0);
        assert!(shown.layers[0].surface().unwrap().rgba(pixel[0], pixel[1])[1] < 0.1);
        harness.state_mut().uv_cache.refreshed = None;
        let refresh_start = Instant::now();
        harness.step();
        eprintln!("3D paint {side}x{side}: partial compose/upload + UI {:.3}ms", refresh_start.elapsed().as_secs_f64() * 1000.0);
        let image = harness.state().uv_cache.textures[material].as_ref().unwrap();
        assert!(image.rgba[pixel[1] as usize * side as usize + pixel[0] as usize][1] < 0.1);
        harness.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        harness.step();
        assert!(harness.state().drag.is_none() && harness.state().uv_paint_key.is_none());
        assert!(harness.state().uv_gpu_active());
        let doc = harness.state().session.active().unwrap().doc.id;
        let docs = harness.state().session.documents().len();
        let old_scene = harness.state().session.uv.scene.clone().unwrap();
        reveal(harness.state_mut()).unwrap();
        harness.step();
        harness.get_by_label("File").click();
        harness.step();
        harness.get_by_label("3D Import ⏵").click();
        harness.step();
        harness.get_by_label("Reimport").click();
        harness.step();
        harness.step();
        assert!(harness.state().uv_gpu_active());
        assert!(!std::sync::Arc::ptr_eq(&old_scene, harness.state().session.uv.scene.as_ref().unwrap()));
        assert_eq!(harness.state().session.documents().len(), docs);
        assert_eq!(harness.state().session.active().unwrap().doc.id, doc);
        if let Ok(path) = std::env::var("PHOTOCRAFT_UV_PAINT_SCREENSHOT") {
            harness.state_mut().uv_cache.refreshed = None;
            harness.step();
            harness.render().unwrap().save(path).unwrap();
        }
        harness.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(harness.state().session.active().unwrap().doc.layers[0].surface().unwrap(), before.layers[0].surface().unwrap());
    }

    #[test]
    fn new_protected_layer_targets_pixels_after_editing_an_existing_mask() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        app.session.execute("uv.shell.select", json!({"shells":[0]})).unwrap();
        let mut state = State::default();
        assert!(command(&mut app, &mut state, "uv.layer.create", json!({"guard":8})));
        app.run("channel.target.layerMask", json!({})).unwrap();
        assert!(app.ui.mask_target);
        assert!(command(&mut app, &mut state, "uv.layer.create", json!({"guard":8})));
        assert!(!app.ui.mask_target);
        assert!(!app.ui.vector_mask_target);
        assert!(!app.layer_mask_targeted());
        assert_eq!(app.session.active().unwrap().channel_view.target, photocraft_engine::channel_cmds::ChannelTarget::Composite);
        app.run("edit.fill", json!({"color":"#ff0000"})).unwrap();
        let doc = &app.session.active().unwrap().doc;
        let image = photocraft_compose::render(doc, doc.bounds());
        assert!(image.px[64 * 128 + 30][1] < 0.01, "fill changes the new layer pixels");
        assert!(doc.layers.last().unwrap().mask.as_ref().unwrap().surface.pixel(52, 64)[0] < 0.01, "the seam stays blocked");
    }

    #[test]
    fn main_canvas_shell_click_does_not_start_brush_and_paint_mode_resumes_it() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        app.ui.uv_workbench.select_shells = true;
        app.session.execute("layer.setProps", json!({"locks":{"all":false,"pixels":false}})).unwrap();
        app.sync_views();
        let index = app.session.active_index().unwrap();
        let mut view = app.ui.views[index].clone();
        view.zoom = 2.5;
        view.center = [50.0, 70.0];
        view.rotation = 31.0;
        view.fit_pending = false;
        view.fill_pending = false;
        app.ui.view.flip_horizontal = true;
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(600.0, 600.0));
        let xf = crate::canvas::ViewXform { rect, zoom: 2.5, center: view.center, rotation: view.rotation, flip: true, aspect: 1.0 };
        let at = xf.to_screen(32.0, 64.0);
        let ctx = egui::Context::default();
        let mut frame = |app: &mut PhotocraftApp, events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(rect), events, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                view = crate::canvas::canvas_view(app, ui, index, rect, view.clone(), true);
            });
            out.textures_delta.clear();
        };
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame(&mut app, vec![egui::Event::PointerMoved(at)]);
        frame(&mut app, vec![button(true)]);
        assert!(app.drag.is_none());
        assert!(!app.press_stroke, "UV press must not paint");
        frame(&mut app, vec![button(false)]);
        assert_eq!(app.session.uv.selected.iter().copied().collect::<Vec<_>>(), vec![0]);
        assert!(app.session.active().unwrap().doc.selection.is_some());
        app.ui.uv_workbench.select_shells = false;
        frame(&mut app, vec![button(true)]);
        assert!(app.drag.is_some() || app.press_stroke, "Paint mode reaches the ordinary brush");
        frame(&mut app, vec![button(false)]);
        assert!(app.drag.is_none());
    }

    #[test]
    fn hidden_uvs_and_unrelated_documents_leave_tools_available() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        let index = app.session.active_index().unwrap();
        app.ui.uv_workbench.select_shells = true;
        assert!(selection_mode(&app, index));
        app.ui.uv_workbench.show_overlay = false;
        assert!(!selection_mode(&app, index));
        app.ui.uv_workbench.show_overlay = true;
        app.session.execute("file.new", json!({"width":128,"height":128})).unwrap();
        let other = app.session.active_index().unwrap();
        assert!(canvas_materials(&app, other).is_empty());
        assert!(!selection_mode(&app, other));
        assert!(selection_mode(&app, index), "closing the separate 3D window does not disable the overlay");
    }

    #[test]
    fn uv_picking_uses_canvas_transform_and_shift_alt_modifiers() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        let color = app.session.uv.bindings[app.session.uv.material];
        *app.session.uv.normal_bindings.last_mut().unwrap() = color;

        let index = app.session.active_index().unwrap();
        let ctx = egui::Context::default();
        for (point, modifiers, expected) in
            [([32.0, 64.0], egui::Modifiers::NONE, vec![0]), ([96.0, 64.0], egui::Modifiers::SHIFT, vec![0, 1]), ([32.0, 64.0], egui::Modifiers::ALT, vec![1])]
        {
            let xf = crate::canvas::ViewXform {
                rect: Rect::from_min_size(pos2(35.0, 80.0), vec2(800.0, 600.0)),
                zoom: 3.7,
                center: [70.0, 50.0],
                rotation: 43.0,
                flip: true,
                aspect: 1.8,
            };
            let p = xf.to_screen(point[0], point[1]);
            let mut out = ctx.run_ui(egui::RawInput { events: vec![egui::Event::ModifiersChanged(modifiers)], ..Default::default() }, |ui| {
                pick_canvas(&mut app, ui, index, xf.to_doc(p))
            });
            out.textures_delta.clear();
            assert_eq!(app.session.uv.selected.iter().copied().collect::<Vec<_>>(), expected);
            assert!(!app.session.uv.normal_channel, "a different material using this image as a normal map must not change the clicked material channel");
        }
        assert_eq!(clip_tile([[-1.0, 0.5], [2.0, 0.5]]), Some([[0.0, 0.5], [1.0, 0.5]]));
        assert!(clip_tile([[-1.0, 0.2], [-1.0, 0.8]]).is_none());
    }

    #[test]
    fn editor_fill_and_undo_update_preview_without_saving_files() {
        let _gpu = photocraft_testkit::gpu_lock();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.fbx");
        let mut session = photocraft_engine::Session::new();
        session.execute("uv.model.import", json!({"path":source.to_string_lossy()})).unwrap();
        let material = session.uv.material;
        let mut app = {
            let mut app = PhotocraftApp::new(session, crate::Services::default());
            app.ui.workspace = "3D".into();
            app
        };
        let ctx = egui::Context::default();
        let mut cache = Cache::default();
        refresh(&mut app, &ctx, &mut cache).unwrap();
        let original = cache.textures[material].as_ref().unwrap().rgba.clone();
        app.session.execute("layer.setProps", json!({"locks":{"all":false,"pixels":false}})).unwrap();
        app.session.execute("edit.fill", json!({"color":"#ff0000"})).unwrap();
        refresh(&mut app, &ctx, &mut cache).unwrap();
        let changed = &cache.textures[material].as_ref().unwrap().rgba;
        assert_ne!(&original, changed);
        assert!(changed.iter().all(|p| p[0] > 0.99 && p[1] < 0.01 && p[2] < 0.01));
        app.session.execute("edit.undo", json!({})).unwrap();
        refresh(&mut app, &ctx, &mut cache).unwrap();
        assert_eq!(original, cache.textures[material].as_ref().unwrap().rgba);
    }

    #[test]
    fn native_picker_import_cancel_and_integrated_view_without_texture_document() {
        let _gpu = photocraft_testkit::gpu_lock();
        use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.fbx").to_string_lossy().into_owned();
        let (dialog, asked) = crate::file_dialog::fake(vec![None, Some(FileDialogAnswer::Paths(vec![source.clone()]))]);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_dialog: Some(dialog), ..Default::default() });
        let ctx = egui::Context::default();
        pick_file(&mut app, "model", 0, false).unwrap();
        app.poll_file_dialog(&ctx, None);
        assert!(app.session.uv.scene.is_none());
        pick_file(&mut app, "model", 0, false).unwrap();
        app.poll_file_dialog(&ctx, None);
        assert_eq!(app.session.uv.source, source);
        assert!(app.ui.uv_workbench.view_3d);
        assert!(matches!(&asked.borrow()[1],FileDialogRequest::Open{multiple:false,extensions:Some(e),..} if e==&vec!["fbx".to_owned(),"obj".to_owned()]));
        reveal(&mut app).unwrap();
        assert!(app.ui.panels.properties);
        assert_eq!(app.ui.dock_tabs.properties, 2);
        // An untextured import has no document tab, but its 3D viewport must still draw.
        app.session.uv.bindings.fill(None);
        app.session.uv.normal_bindings.fill(None);
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 650.0))), ..Default::default() }, |ui| {
            assert!(workspace(&mut app, ui));
        });
        assert!(out.viewport_output.len() <= 1, "no separate native workbench window");
        out.textures_delta.clear();
        assert!(app.uv_cache.frame.as_ref().is_some_and(|f| f.width > f.height));
        assert!(
            app.uv_cache.frame.as_ref().is_some_and(|f| f.triangles.iter().any(Option::is_some)),
            "the initial camera must show the front of the imported FBX"
        );
    }
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn native_picker_keeps_gpu_after_model_texture_and_project_import() {
        let _gpu = photocraft_testkit::gpu_lock();
        use crate::file_dialog::FileDialogAnswer;
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures");
        let project = std::env::temp_dir().join(format!(
            "photocraft-uv-picker-gpu-{}-{}.pcuv",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fixture_app().run("uv.project.save", json!({"path":project})).unwrap();
        let answer = |path: std::path::PathBuf| Some(FileDialogAnswer::Paths(vec![path.to_string_lossy().into_owned()]));
        let (dialog, _) = crate::file_dialog::fake(vec![
            None,
            answer(fixtures.join("absent-model.fbx")),
            answer(fixtures.join("two-shells.fbx")),
            answer(fixtures.join("checker.png")),
            answer(project.clone()),
            answer(fixtures.join("normals-reflected.fbx")),
        ]);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_dialog: Some(dialog), ..Default::default() });
        let rs = egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default());
        app.set_uv_wgpu(rs).unwrap();
        let ctx = egui::Context::default();
        for (index, kind) in ["model", "model", "model", "texture", "project", "model"].into_iter().enumerate() {
            pick_file(&mut app, kind, 0, false).unwrap();
            app.poll_file_dialog(&ctx, None);
            assert!(app.uv_gpu_active(), "picker {kind}, step {index}: GPU renderer was lost");
            if index >= 2 {
                let mut out =
                    ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 650.0))), ..Default::default() }, |ui| {
                        assert!(workspace(&mut app, ui));
                    });
                out.textures_delta.clear();
                assert!(app.uv_cache.gpu.as_ref().unwrap().frame_info().is_some());
                assert!(app.uv_cache.frame.is_none(), "native picker must not select the CPU rasterizer");
            }
        }
        std::fs::remove_file(project).unwrap();
    }
    #[test]
    fn normal_preview_samples_data_without_display_transform_and_tracks_edits() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = fixture_app();
        let material = app.session.uv.material;
        app.run("uv.texture.create", json!({"channel":"normal","width":32,"height":32,"depth":32})).unwrap();
        let ctx = egui::Context::default();
        let mut cache = Cache::default();
        refresh(&mut app, &ctx, &mut cache).unwrap();
        let pixel = cache.normals[material].as_ref().unwrap().rgba[0];
        assert!((pixel[0] - 0.5).abs() < 1e-6 && pixel[2] == 1.0);
        app.run("edit.fill", json!({"color":"#ff0000"})).unwrap();
        refresh(&mut app, &ctx, &mut cache).unwrap();
        assert!(cache.normals[material].as_ref().unwrap().rgba.iter().all(|p| p[0] > 0.99 && p[1] < 0.01));
        app.run("edit.undo", json!({})).unwrap();
        refresh(&mut app, &ctx, &mut cache).unwrap();
        assert_eq!(pixel, cache.normals[material].as_ref().unwrap().rgba[0]);
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;
    #[test]
    fn surface_orbit_updates_once_per_gesture_without_a_view_jump() {
        let _gpu = photocraft_testkit::gpu_lock();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj");
        let mut session = photocraft_engine::Session::new();
        session.execute("uv.model.import", json!({"path":source})).unwrap();
        let mut app = PhotocraftApp::new(session, Default::default());
        app.ui.uv_workbench.yaw = 0.0;
        app.ui.uv_workbench.pitch = 0.0;
        app.ui.uv_workbench.pan = [-0.4, 0.1];
        let ctx = egui::Context::default();
        let pump = |app: &mut PhotocraftApp, events| {
            let mut out = ctx
                .run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(320.0, 240.0))), events, ..Default::default() }, |ui| {
                    main_view(app, ui)
                });
            out.textures_delta.clear();
        };
        pump(&mut app, vec![]);
        let at = app.uv_cache.viewport.unwrap().center();
        let g = app.uv_cache.geometry.as_ref().unwrap().1.clone();
        let before = app.ui.uv_workbench.camera();
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::ALT };
        pump(&mut app, vec![egui::Event::PointerMoved(at), egui::Event::ModifiersChanged(egui::Modifiers::ALT), button(true)]);
        let pivot = app.ui.uv_workbench.target.expect("camera center intersects the sloped shell");
        let after = app.ui.uv_workbench.camera();
        let a = g.camera_values(before, [320, 240]);
        let b = g.camera_values(after, [320, 240]);
        for i in [4, 5, 6, 7] {
            assert!((a[i] - b[i]).abs() < 1e-6);
        }
        pump(&mut app, vec![egui::Event::PointerMoved(at + vec2(35.0, 15.0))]);
        assert_eq!(app.ui.uv_workbench.target, Some(pivot));
        assert_ne!(app.ui.uv_workbench.yaw, before.yaw);
        pump(&mut app, vec![button(false)]);
        // Pan to another area; the next Alt+LMB must find that surface afresh.
        app.ui.uv_workbench.yaw = 0.0;
        app.ui.uv_workbench.pitch = 0.0;
        app.ui.uv_workbench.pan[0] += 0.3;
        pump(&mut app, vec![button(true)]);
        assert_ne!(app.ui.uv_workbench.target, Some(pivot));
        assert!(app.drag.is_none(), "navigation must not paint");
        let previous = app.ui.uv_workbench.clone();
        let maps = preview::Materials::default();
        let scene = app.session.uv.scene.as_ref().unwrap();
        app.ui.uv_workbench.orbit_pivot = OrbitPivot::Cursor;
        surface_pivot(&mut app.ui.uv_workbench, &g, scene, &maps, Some([0.0, 0.0]), [320, 240]);
        assert_eq!(app.ui.uv_workbench.target, previous.target, "background retains the old pivot");
        app.ui.uv_workbench.orbit_pivot = OrbitPivot::Fixed;
        surface_pivot(&mut app.ui.uv_workbench, &g, scene, &maps, Some([0.5, 0.5]), [320, 240]);
        assert_eq!(app.ui.uv_workbench.target, previous.target);
        let mut cursor_state = State { orbit_pivot: OrbitPivot::Cursor, yaw: 0.0, pitch: 0.0, ..Default::default() };
        let cursor = [0.4, 0.5];
        let hit = g.hit(scene, cursor_state.camera(), &maps, cursor, [320, 240]).unwrap();
        surface_pivot(&mut cursor_state, &g, scene, &maps, Some(cursor), [320, 240]);
        assert_eq!(cursor_state.target, Some(hit.position));
        let maps = preview::Materials { hidden: (0..scene.materials.len()).collect(), ..Default::default() };
        let target = cursor_state.target;
        surface_pivot(&mut cursor_state, &g, scene, &maps, Some([0.6, 0.5]), [320, 240]);
        assert_eq!(cursor_state.target, target, "hidden geometry cannot change the pivot");
        let legacy: State = serde_json::from_value(json!({"zoom":2.0})).unwrap();
        assert_eq!(legacy.orbit_pivot, OrbitPivot::ViewCenter);
        assert_eq!(serde_json::from_value::<State>(serde_json::to_value(&app.ui.uv_workbench).unwrap()).unwrap(), app.ui.uv_workbench);
    }
    #[test]
    fn resolution_tracks_physical_pixels_and_supersampling_with_bounded_memory() {
        let _gpu = photocraft_testkit::gpu_lock();
        assert_eq!(preview_size(vec2(1800.0, 1000.0), 1.0, false, 4096), [1800, 1000]);
        assert_eq!(preview_size(vec2(1800.0, 1000.0), 1.0, true, 4096), [3600, 2000]);
        assert_eq!(preview_size(vec2(900.0, 500.0), 2.0, false, 4096), [1800, 1000]);
        assert_eq!(preview_size(vec2(4096.0, 2048.0), 1.0, true, 2048), [2048, 1024]);
        let [w, h] = preview_size(vec2(7680.0, 4320.0), 2.0, true, 4096);
        assert_eq!(w, 4096);
        assert!(w * h <= preview::MAX_PREVIEW_PIXELS);
        assert!((w as f32 / h as f32 - 16.0 / 9.0).abs() < 0.002);
        assert_eq!(preview_size(vec2(f32::NAN, f32::INFINITY), f32::NAN, false, 4096), [1, 1]);
    }
    #[test]
    fn maya_mouse_gestures_latch_alt_until_release_and_plain_buttons_do_not_navigate() {
        let _gpu = photocraft_testkit::gpu_lock();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj");
        let scene = photocraft_uv::import::load("test.obj", &std::fs::read(source).unwrap(), 0, |_| Err(photocraft_uv::Error("unused MTL".into()))).unwrap();
        for button in [egui::PointerButton::Primary, egui::PointerButton::Middle, egui::PointerButton::Secondary] {
            let ctx = egui::Context::default();
            let mut state = State::default();
            let mut nav = None;
            let mut pump = |state: &mut State, mut events: Vec<egui::Event>, modifiers: egui::Modifiers| {
                events.insert(0, egui::Event::ModifiersChanged(modifiers));
                let mut used = false;
                let mut out =
                    ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 500.0))), events, ..Default::default() }, |ui| {
                        let (_, response) = ui.allocate_exact_size(vec2(600.0, 500.0), Sense::click_and_drag());
                        used = navigate(ui, &response, state, &mut nav, scene_extent(&scene)).gesture;
                    });
                out.textures_delta.clear();
                used
            };
            let at = pos2(200.0, 200.0);
            let end = at + vec2(50.0, 20.0);
            let event = |pressed, pos, modifiers| egui::Event::PointerButton { pos, button, pressed, modifiers };
            pump(&mut state, vec![egui::Event::PointerMoved(at)], egui::Modifiers::NONE);
            assert!(!pump(&mut state, vec![event(true, at, egui::Modifiers::NONE)], egui::Modifiers::NONE));
            let before = state.clone();
            assert!(!pump(&mut state, vec![egui::Event::PointerMoved(end)], egui::Modifiers::NONE));
            assert_eq!(state, before);
            pump(&mut state, vec![event(false, end, egui::Modifiers::NONE), egui::Event::PointerMoved(at)], egui::Modifiers::NONE);
            assert!(pump(&mut state, vec![event(true, at, egui::Modifiers::ALT)], egui::Modifiers::ALT));
            assert!(pump(&mut state, vec![egui::Event::PointerMoved(end)], egui::Modifiers::ALT));
            match button {
                egui::PointerButton::Primary => {
                    assert!(state.yaw < before.yaw);
                    assert!(state.pitch > before.pitch);
                    assert_eq!(state.zoom, before.zoom);
                }
                egui::PointerButton::Middle => {
                    assert!(state.pan[0] > 0.0 && state.pan[1] < 0.0);
                    assert_eq!(state.yaw, before.yaw);
                }
                _ => {
                    assert!(state.zoom > before.zoom);
                    assert_eq!(state.yaw, before.yaw);
                }
            }
            // Alt released first: continue the same gesture and suppress the release click.
            assert!(pump(&mut state, vec![egui::Event::PointerMoved(end + vec2(5.0, 0.0))], egui::Modifiers::NONE));
            assert!(pump(&mut state, vec![event(false, end + vec2(5.0, 0.0), egui::Modifiers::NONE)], egui::Modifiers::NONE));
            assert!(!pump(&mut state, vec![], egui::Modifiers::NONE));
        }
    }
    #[test]
    fn frame_selected_changes_the_orbit_pivot_and_frame_all_restores_it() {
        let _gpu = photocraft_testkit::gpu_lock();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.fbx");
        let mut session = photocraft_engine::Session::new();
        session.execute("uv.model.import", json!({"path":source})).unwrap();
        session.execute("uv.shell.select", json!({"shells":[0]})).unwrap();
        let mut app = {
            let mut app = PhotocraftApp::new(session, crate::Services::default());
            app.ui.workspace = "3D".into();
            app
        };
        app.ui.uv_workbench.view_3d = true;
        app.uv_cache.viewport = Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 500.0)));
        let ctx = egui::Context::default();
        for key in [egui::Key::F, egui::Key::A] {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    events: vec![
                        egui::Event::PointerMoved(pos2(200.0, 200.0)),
                        egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
                    ],
                    ..Default::default()
                },
                |_| {
                    crate::shortcuts::handle(&mut app, &ctx);
                },
            );
            out.textures_delta.clear();
            if key == egui::Key::F {
                assert!(app.ui.uv_workbench.target.is_some());
                assert!(app.ui.uv_workbench.zoom > 1.0);
            } else {
                assert!(app.ui.uv_workbench.target.is_none());
                assert_eq!(app.ui.uv_workbench.zoom, 1.0);
            }
        }
    }
}
