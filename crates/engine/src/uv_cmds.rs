//! Optional 3D / UV bridge. Mesh data stays separate from the 2D document format.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{DocId, Document, Layer, LayerMask};
use photocraft_geom::Size;
use photocraft_raster::Surface;
use photocraft_uv::preview::Channel;
use photocraft_uv::{
    Scene,
    topology::{MaskOptions, Topology},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path, sync::Arc};

#[derive(Default)]
pub struct UvState {
    /// Restored external destinations require explicit rebinding before texture writeback.
    pub(crate) blocked_exports: std::collections::HashSet<DocId>,
    /// Opaque, bounded view metadata; camera/UI types stay above the engine layer.
    pub project_view: Value,
    pub project_path: String,
    pub assets: Vec<crate::uv_asset_cmds::Asset>,
    pub environment: Option<crate::uv_asset_cmds::Resource>,
    pub(crate) exported_revisions: std::collections::HashMap<DocId, u64>,
    pub(crate) saved_revisions: std::collections::HashMap<DocId, u64>,
    pub(crate) saved_generation: u64,
    pub scene: Option<Arc<Scene>>,
    pub topology: Option<Arc<Topology>>,
    pub(crate) wrap_mesh: Option<Arc<photocraft_uv::wrap::Mesh>>,
    pub(crate) wrapped_stroke: Option<crate::uv_wrap_cmds::WrappedStroke>,
    pub source: String,
    pub uv_set: usize,
    pub source_stamp: Option<crate::uv_reload_cmds::SourceStamp>,
    pub bindings: Vec<Option<DocId>>,
    pub normal_bindings: Vec<Option<DocId>>,
    pub hidden_materials: BTreeSet<usize>,
    pub normal_channel: bool,
    pub extra_channel: Option<Channel>,
    pub map_bindings: std::collections::BTreeMap<Channel, Vec<Option<DocId>>>,
    pub selected: BTreeSet<usize>,
    selection_masks: std::collections::BTreeMap<u64, Surface>,
    pub material: usize,
    pub guard: f64,
    pub horizontal: f64,
    pub vertical: f64,
    pub feather: f64,
    pub open_borders: bool,
    pub generation: u64,
}

impl UvState {
    pub fn needs_export(&self, doc: &crate::DocState) -> bool {
        self.exported_revisions.get(&doc.doc.id).map_or_else(|| doc.is_dirty(), |revision| doc.revision.saturating_sub(doc.view_revision) != *revision)
    }
    pub fn has_unsaved_project(&self, docs: &[crate::DocState]) -> bool {
        self.scene.is_some()
            && (self.project_path.is_empty()
                || self.generation != self.saved_generation
                || self.saved_revisions.len() != docs.len()
                || docs.iter().any(|st| self.saved_revisions.get(&st.doc.id) != Some(&st.revision.saturating_sub(st.view_revision))))
    }

    pub fn wrap_pixels_per_unit(&self) -> Option<f64> {
        self.wrapped_stroke.as_ref().and_then(|stroke| stroke.pixels_per_unit)
    }
    pub fn current_channel(&self) -> Channel {
        self.extra_channel.unwrap_or(if self.normal_channel { Channel::Normal } else { Channel::Color })
    }
    pub fn map_bindings(&self, kind: Channel) -> &[Option<DocId>] {
        match kind {
            Channel::Normal => &self.normal_bindings,
            Channel::Color | Channel::Material => &self.bindings,
            _ => self.map_bindings.get(&kind).map_or(&[], Vec::as_slice),
        }
    }
    pub fn all_bindings(&self) -> impl Iterator<Item = &Option<DocId>> {
        self.bindings.iter().chain(&self.normal_bindings).chain(self.map_bindings.values().flatten())
    }
    fn set_channel(&mut self, kind: Channel) {
        self.normal_channel = kind == Channel::Normal;
        self.extra_channel = (!matches!(kind, Channel::Material | Channel::Color | Channel::Normal)).then_some(kind);
    }
}

fn err(message: impl Into<String>) -> EngineError {
    EngineError::Other(message.into())
}
fn mesh(s: &Session) -> std::result::Result<(), String> {
    if s.uv.scene.is_some() { Ok(()) } else { Err("Import a UV-mapped model first".into()) }
}
fn uint(p: &Value, key: &str, default: usize) -> Result<usize> {
    match p.get(key) {
        None => Ok(default),
        Some(n) => n.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| err(format!("{key} must be a nonnegative integer"))),
    }
}
fn number(p: &Value, key: &str, default: f64) -> Result<f64> {
    match p.get(key) {
        None => Ok(default),
        Some(n) => n.as_f64().filter(|n| n.is_finite()).ok_or_else(|| err(format!("{key} must be finite"))),
    }
}
fn boolean(p: &Value, key: &str, default: bool) -> Result<bool> {
    match p.get(key) {
        None => Ok(default),
        Some(n) => n.as_bool().ok_or_else(|| err(format!("{key} must be boolean"))),
    }
}
fn path(p: &Value) -> Result<&str> {
    p.get("path").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| err("path is required"))
}
fn material(s: &Session, p: &Value) -> Result<usize> {
    let i = uint(p, "material", s.uv.material)?;
    if s.uv.scene.as_ref().is_some_and(|s| i < s.materials.len()) { Ok(i) } else { Err(err("material is out of range")) }
}
#[cfg(not(target_arch = "wasm32"))]
fn read(path: &str) -> Result<Vec<u8>> {
    read_limited(path, photocraft_uv::MAX_FILE_BYTES)
}
const MAX_PROJECT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_limited(path: &str, limit: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    if !std::fs::metadata(path).map_err(|e| err(e.to_string()))?.is_file() {
        return Err(err("UV input must be a regular file"));
    }
    let file = std::fs::File::open(path).map_err(|e| err(format!("{path}: {e}")))?;
    let size = file.metadata().map_err(|e| err(e.to_string()))?.len();
    if size > limit {
        return Err(err(format!("UV input exceeds {} MiB", limit / (1024 * 1024))));
    }
    let mut reader = file.take(limit + 1);
    let bytes = photocraft_format::read::read_all(&mut reader, size).map_err(|e| err(e.to_string()))?;
    if bytes.len() as u64 > limit {
        return Err(err(format!("UV input exceeds {} MiB", limit / (1024 * 1024))));
    }
    Ok(bytes)
}
#[cfg(target_arch = "wasm32")]
fn read(_path: &str) -> Result<Vec<u8>> {
    Err(err("UV file import is available in native builds only"))
}
#[cfg(target_arch = "wasm32")]
pub(crate) fn read_limited(path: &str, _limit: u64) -> Result<Vec<u8>> {
    read(path)
}
/// A bare model filename belongs to the current directory, not the empty path.
fn model_directory(source: &str) -> &Path {
    Path::new(source).parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new("."))
}

/// Resolve model-provided references only to regular files within the model directory.
fn resolved(source: &str, name: &str) -> String {
    let base = model_directory(source);
    let Ok(base) = base.canonicalize() else { return String::new() };
    let name = name.replace('\\', "/");
    let candidate = base.join(&name);
    let nearby = base.join(Path::new(&name).file_name().unwrap_or_default());
    for path in [candidate, nearby] {
        if let Ok(path) = path.canonicalize()
            && path.starts_with(&base)
            && path.is_file()
        {
            return path.to_string_lossy().into_owned();
        }
    }
    String::new()
}

pub(super) fn load_scene(source: &str, uv_set: usize) -> Result<(Scene, Option<crate::uv_reload_cmds::SourceStamp>)> {
    let stamp = crate::uv_reload_cmds::SourceStamp::read(source)?;
    let bytes = read(source)?;
    let mut scene = photocraft_uv::import::load(source, &bytes, uv_set, |name| read(&resolved(source, name)).map_err(|e| photocraft_uv::Error(e.to_string())))
        .map_err(|e| err(e.to_string()))?;
    // Only unambiguous names: material-prefixed siblings, or bare channel names
    // when exactly one material is used. Never guess between multiple candidates.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let used: BTreeSet<_> = scene.triangles.iter().map(|t| t.material).collect();
        let base = model_directory(source);
        let files: Vec<_> =
            std::fs::read_dir(base).into_iter().flatten().filter_map(std::result::Result::ok).take(4096).map(|e| e.path()).filter(|p| p.is_file()).collect();
        for (i, m) in scene.materials.iter_mut().enumerate().filter(|(i, _)| used.contains(i)) {
            let _ = i;
            for (kind, aliases) in [
                (Channel::Color, &["color", "basecolor", "albedo", "diffuse"][..]),
                (Channel::Normal, &["normal", "normalmap", "n"][..]),
                (Channel::Id, &["id", "matid"][..]),
                (Channel::Ao, &["ao", "ambientocclusion"][..]),
                (Channel::Curvature, &["curvature", "cv"][..]),
                (Channel::Opacity, &["opacity"][..]),
                (Channel::Metallic, &["metallic", "metalness"][..]),
                (Channel::Roughness, &["roughness"][..]),
                (Channel::Emissive, &["emissive", "emission", "emissivecolor"][..]),
            ] {
                let existing = match kind {
                    Channel::Color => m.base_color.as_ref(),
                    Channel::Normal => m.normal_map.as_ref(),
                    _ => m.maps.get(&kind),
                };
                if existing.is_some() {
                    continue;
                }
                let prefix = m.name.to_lowercase();
                let candidates: Vec<_> = files
                    .iter()
                    .filter(|p| {
                        let ext = p.extension().and_then(|v| v.to_str()).unwrap_or("").to_ascii_lowercase();
                        if !["tif", "tiff", "tga", "png", "dds", "exr", "jpg", "jpeg", "bmp"].contains(&ext.as_str()) {
                            return false;
                        }
                        let stem = p.file_stem().and_then(|v| v.to_str()).unwrap_or("").to_lowercase();
                        aliases.iter().any(|alias| (used.len() == 1 && stem == *alias) || stem == format!("{prefix}_{alias}"))
                    })
                    .collect();
                if let [path] = candidates.as_slice() {
                    let name = path.to_string_lossy().into_owned();
                    match kind {
                        Channel::Color => m.base_color = Some(name),
                        Channel::Normal => m.normal_map = Some(name),
                        _ => {
                            m.maps.insert(kind, name);
                        }
                    }
                }
            }
        }
    }
    for m in &mut scene.materials {
        for name in m.maps.values_mut() {
            *name = resolved(source, name);
        }
        if let Some(name) = &m.normal_map {
            m.normal_map = Some(resolved(source, name));
        }
        if let Some(name) = &m.base_color {
            m.base_color = Some(resolved(source, name));
        }
    }
    if crate::uv_reload_cmds::SourceStamp::read(source)? != stamp {
        return Err(err("The model changed while being read; wait for the export to finish and retry"));
    }
    Ok((scene, stamp))
}
fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let previous: Vec<_> = s.documents().iter().filter(|_| s.uv_workspace.active && s.uv.scene.is_some()).map(|st| st.doc.id).collect();
    let source = path(p)?.to_owned();
    let uv_set = uint(p, "uvSet", 0)?;
    let (mut scene, source_stamp) = load_scene(&source, uv_set)?;
    let topology = Topology::build(&scene).map_err(|e| err(e.to_string()))?;
    let mut documents = Vec::new();
    // Decode everything before mutating the session; missing textures stay unbound, never
    // aborting an otherwise valid mesh import.
    for (i, m) in scene.materials.iter().enumerate() {
        for (normal, filename, embedded) in [(false, &m.base_color, &m.embedded), (true, &m.normal_map, &m.embedded_normal)] {
            let Some(bytes) = embedded.clone().or_else(|| filename.as_ref().and_then(|p| read(p).ok())) else {
                if let Some(path) = filename {
                    scene.warnings.push(format!("{}: missing texture {path}; assign a replacement", m.name));
                }
                continue;
            };
            match photocraft_io::import(filename.as_deref().unwrap_or("embedded.png"), &bytes) {
                Ok(imported) if !imported.preview_only => {
                    if normal && imported.document.mode != ColorMode::Rgb {
                        scene.warnings.push(format!("{}: normal map must be RGB; assign a replacement", m.name));
                        continue;
                    }
                    scene.warnings.extend(imported.warnings);
                    documents.push((i, normal, imported.document, filename.clone().filter(|_| embedded.is_none() && !imported.source_read_only)));
                }
                Ok(_) => scene.warnings.push(format!("{}: texture is preview only; relink it", m.name)),
                Err(e) => scene.warnings.push(format!("{}: cannot open texture: {e}", m.name)),
            }
        }
    }
    let mut bindings = vec![None; scene.materials.len()];
    let mut normal_bindings = vec![None; scene.materials.len()];
    let mut opened = std::collections::BTreeMap::<String, DocId>::new();
    for (i, normal, doc, path) in documents {
        let target = if normal { &mut normal_bindings } else { &mut bindings };
        if let Some(id) = path.as_ref().and_then(|p| opened.get(p)).copied() {
            if let Some(b) = target.get_mut(i) {
                *b = Some(id);
            }
            continue;
        }
        let source_path = path.clone();
        let index = s.add_document(doc, path);
        if let Some(id) = s.documents().get(index).map(|d| d.doc.id)
            && let Some(b) = target.get_mut(i)
        {
            *b = Some(id);
            if let Some(path) = source_path {
                opened.insert(path, id);
            }
        }
    }
    let mut map_bindings = std::collections::BTreeMap::new();
    for kind in Channel::MAPS.into_iter().filter(|kind| !matches!(kind, Channel::Color | Channel::Normal)) {
        let mut target = vec![None; scene.materials.len()];
        for (i, m) in scene.materials.iter().enumerate() {
            let Some(path) = m.maps.get(&kind) else { continue };
            if let Some(id) = opened.get(path) {
                if let Some(slot) = target.get_mut(i) {
                    *slot = Some(*id);
                }
                continue;
            }
            match read(path).and_then(|bytes| photocraft_io::import(path, &bytes).map_err(|e| err(e.to_string()))) {
                Ok(imported) if !imported.preview_only => {
                    let index = s.add_document(imported.document, (!imported.source_read_only).then(|| path.clone()));
                    if let Some(id) = s.documents().get(index).map(|st| st.doc.id) {
                        if let Some(slot) = target.get_mut(i) {
                            *slot = Some(id);
                        }
                        opened.insert(path.clone(), id);
                    }
                }
                _ => scene.warnings.push(format!("{}: cannot load {} map {path}", m.name, kind.label())),
            }
        }
        map_bindings.insert(kind, target);
    }
    let active_material = bindings.iter().position(Option::is_some).unwrap_or(0);
    let generation = s.uv.generation.wrapping_add(1);
    s.uv = UvState {
        scene: Some(Arc::new(scene)),
        topology: Some(Arc::new(topology)),
        source,
        uv_set,
        source_stamp,
        bindings,
        normal_bindings,
        map_bindings,
        material: active_material,
        guard: 8.0,
        generation,
        ..Default::default()
    };
    for id in previous {
        if let Some(index) = s.documents().iter().position(|st| st.doc.id == id) {
            s.close(index);
        }
    }
    let _ = activate(s, active_material);
    inspect(s, &Value::Null)
}

fn channel(p: &Value, default: Channel) -> Result<Channel> {
    match p.get("channel") {
        None => Ok(default),
        Some(value) => Channel::MAPS.into_iter().find(|kind| value.as_str() == Some(kind.key())).ok_or_else(|| err("Unsupported texture channel")),
    }
}
fn bindings(s: &Session) -> &[Option<DocId>] {
    s.uv.map_bindings(s.uv.current_channel())
}
fn bind(s: &mut Session, material: usize, id: DocId, kind: Channel) {
    s.uv.blocked_exports.remove(&id);
    let count = s.uv.bindings.len();
    let target = match kind {
        Channel::Normal => &mut s.uv.normal_bindings,
        Channel::Material | Channel::Color => &mut s.uv.bindings,
        _ => s.uv.map_bindings.entry(kind).or_insert_with(|| vec![None; count]),
    };
    if let Some(b) = target.get_mut(material) {
        *b = Some(id);
    }
    s.uv.set_channel(kind);
}
fn material_select(s: &mut Session, p: &Value) -> Result<Value> {
    let material = material(s, p)?;
    let normal = channel(p, s.uv.current_channel())?;
    if s.uv.material != material {
        s.uv.selected.clear();
    }
    s.uv.material = material;
    s.uv.set_channel(normal);
    let _ = activate(s, material);
    s.uv.generation = s.uv.generation.wrapping_add(1);
    Ok(json!({"material":material,"channel":normal.key()}))
}
fn settings(s: &mut Session, p: &Value) -> Result<Value> {
    let guard = number(p, "guard", s.uv.guard)?;
    let feather = number(p, "feather", s.uv.feather)?;
    let horizontal = number(p, "horizontal", s.uv.horizontal)?;
    let vertical = number(p, "vertical", s.uv.vertical)?;
    let borders = boolean(p, "openBorders", s.uv.open_borders)?;
    if [guard, horizontal, vertical, feather].iter().any(|v| !(0.0..=1024.0).contains(v)) {
        return Err(err("guard and feather must be in 0..1024"));
    }
    if (s.uv.guard, s.uv.horizontal, s.uv.vertical, s.uv.feather, s.uv.open_borders) != (guard, horizontal, vertical, feather, borders) {
        s.uv.generation = s.uv.generation.wrapping_add(1);
    }
    s.uv.guard = guard;
    s.uv.horizontal = horizontal;
    s.uv.vertical = vertical;
    s.uv.feather = feather;
    s.uv.open_borders = borders;
    Ok(json!({"guard":guard,"horizontal":horizontal,"vertical":vertical,"feather":feather,"openBorders":borders}))
}
fn material_visible(s: &mut Session, p: &Value) -> Result<Value> {
    let material = material(s, p)?;
    let visible = boolean(p, "visible", true)?;
    if visible {
        s.uv.hidden_materials.remove(&material);
    } else {
        s.uv.hidden_materials.insert(material);
    }
    s.uv.generation = s.uv.generation.wrapping_add(1);
    Ok(json!({"visible":visible}))
}
fn activate(s: &mut Session, material: usize) -> Result<()> {
    let doc = bindings(s).get(material).copied().flatten().ok_or_else(|| err("No texture document is assigned; open or create a texture"))?;
    let index = s.documents().iter().position(|d| d.doc.id == doc).ok_or_else(|| err("The assigned texture document was closed; relink it"))?;
    s.set_active(index);
    Ok(())
}

fn texture_open(s: &mut Session, p: &Value) -> Result<Value> {
    let material = material(s, p)?;
    let normal = channel(p, s.uv.current_channel())?;
    let texture_path = path(p)?.to_owned();
    let bytes = read(&texture_path)?;
    let imported = photocraft_io::import(&texture_path, &bytes).map_err(|e| err(e.to_string()))?;
    if imported.preview_only {
        return Err(err("A preview-only document cannot be used as a texture"));
    }
    if normal == Channel::Normal && imported.document.mode != ColorMode::Rgb {
        return Err(err("Normal maps must use RGB mode"));
    }
    let save_path = if imported.source_read_only { None } else { Some(texture_path) };
    let index = s.add_document(imported.document, save_path);
    let id = s.documents().get(index).map(|d| d.doc.id).ok_or(EngineError::NoDocument)?;
    bind(s, material, id, normal);
    s.uv.material = material;
    s.uv.selected.clear();
    s.uv.generation = s.uv.generation.wrapping_add(1);
    Ok(json!({"document":id.0,"warnings":imported.warnings}))
}
fn texture_create(s: &mut Session, p: &Value) -> Result<Value> {
    let material = material(s, p)?;
    let normal = channel(p, s.uv.current_channel())?;
    let width = uint(p, "width", 1024)?;
    let height = uint(p, "height", 1024)?;
    if width == 0 || height == 0 || width.checked_mul(height).is_none_or(|n| n > photocraft_uv::MAX_PIXELS) {
        return Err(err("Texture dimensions must fit in 16M pixels"));
    }
    let depth = match uint(p, "depth", 8)? {
        8 => SampleType::U8,
        16 => SampleType::U16,
        32 => SampleType::F32,
        _ => return Err(err("depth must be 8, 16 or 32")),
    };
    let mut doc = Document::new("UV texture", Size::new(width as u32, height as u32), ColorMode::Rgb, depth);
    let mut background = Layer::raster("Background", doc.pixel_format());
    if let Some(surface) = background.surface_mut() {
        surface.fill_rect(doc.bounds(), if normal == Channel::Normal { &[0.5, 0.5, 1.0, 1.0] } else { &[1.0, 1.0, 1.0, 1.0] });
    }
    doc.layers.push(background);
    let index = s.add_document(doc, None);
    let id = s.documents().get(index).map(|d| d.doc.id).ok_or(EngineError::NoDocument)?;
    bind(s, material, id, normal);
    s.uv.material = material;
    s.uv.selected.clear();
    s.uv.generation = s.uv.generation.wrapping_add(1);
    Ok(json!({"document":id.0}))
}
fn texture_bind(s: &mut Session, p: &Value) -> Result<Value> {
    let material = material(s, p)?;
    let normal = channel(p, s.uv.current_channel())?;
    if normal == Channel::Normal && s.active().is_some_and(|d| d.doc.mode != ColorMode::Rgb) {
        return Err(err("Normal maps must use RGB mode"));
    }
    let id = s.active().ok_or(EngineError::NoDocument)?.doc.id;
    bind(s, material, id, normal);
    s.uv.material = material;
    s.uv.selected.clear();
    s.uv.generation = s.uv.generation.wrapping_add(1);
    Ok(json!({"document":id.0}))
}

fn select(s: &mut Session, p: &Value) -> Result<Value> {
    let material = material(s, p)?;
    let topology = s.uv.topology.as_ref().ok_or_else(|| err("No topology"))?;
    let items = p.get("shells").and_then(Value::as_array).ok_or_else(|| err("shells must be an array of shell IDs"))?;
    if items.len() > topology.shells.len() {
        return Err(err("Too many shell IDs"));
    }
    let mut selected = if s.uv.material == material { s.uv.selected.clone() } else { BTreeSet::new() };
    let incoming: BTreeSet<usize> =
        items.iter().map(|v| v.as_u64().and_then(|v| usize::try_from(v).ok()).ok_or_else(|| err("Invalid shell ID"))).collect::<Result<_>>()?;
    if incoming.iter().any(|i| topology.shells.get(*i).is_none_or(|shell| shell.material != material)) {
        return Err(err("A shell does not belong to this material"));
    }
    match p.get("mode").and_then(Value::as_str).unwrap_or("replace") {
        "replace" => selected = incoming,
        "add" => selected.extend(incoming),
        "subtract" => selected.retain(|i| !incoming.contains(i)),
        "intersect" => selected.retain(|i| incoming.contains(i)),
        _ => return Err(err("mode must be replace, add, subtract or intersect")),
    }
    let _ = activate(s, material);
    s.uv.material = material;
    s.uv.selected = selected;
    s.uv.generation = s.uv.generation.wrapping_add(1);
    Ok(json!({"shells":s.uv.selected}))
}

fn selection_surface(s: &Session, p: &Value) -> Result<(DocId, Surface)> {
    let material = material(s, p)?;
    let id = bindings(s).get(material).copied().flatten().ok_or_else(|| err("No texture document assigned"))?;
    let doc = s.documents().iter().find(|d| d.doc.id == id).ok_or_else(|| err("The texture document is closed"))?;
    let guard = number(p, "guard", s.uv.guard)?;
    let feather = number(p, "feather", s.uv.feather)?;
    let horizontal = number(p, "horizontal", s.uv.horizontal)?;
    let vertical = number(p, "vertical", s.uv.vertical)?;
    let mask =
        s.uv.topology
            .as_ref()
            .ok_or_else(|| err("No topology"))?
            .mask(
                s.uv.scene.as_ref().ok_or_else(|| err("No mesh"))?,
                &MaskOptions {
                    width: doc.doc.size.width,
                    height: doc.doc.size.height,
                    material,
                    tile: [0, 0],
                    selected: s.uv.selected.clone(),
                    guard,
                    horizontal,
                    vertical,
                    feather,
                    open_borders: boolean(p, "openBorders", s.uv.open_borders)?,
                },
            )
            .map_err(|e| err(e.to_string()))?;
    // No selection means unrestricted editing in PhotoCraft. An empty UV selection MUST be
    // an explicit zero surface, otherwise selecting a tiny/protected shell would edit everything.
    let mut surface = Surface::new(PixelFormat::GRAY8);
    surface.write_region(doc.doc.bounds(), &mask);
    Ok((id, surface))
}
fn apply_mask(s: &mut Session, p: &Value) -> Result<Value> {
    let (id, surface) = selection_surface(s, p)?;
    let index = s.documents().iter().position(|d| d.doc.id == id).ok_or(EngineError::NoDocument)?;
    s.set_active(index);
    let owned = surface.clone();
    s.edit("Select UV shells", |doc, _| {
        doc.selection = Some(surface);
        Ok(())
    })?;
    let live: BTreeSet<_> = s.documents().iter().map(|d| d.doc.id.0).collect();
    s.uv.selection_masks.retain(|id, _| live.contains(id));
    s.uv.selection_masks.insert(id.0, owned);
    s.uv.guard = number(p, "guard", s.uv.guard)?;
    s.uv.horizontal = number(p, "horizontal", s.uv.horizontal)?;
    s.uv.vertical = number(p, "vertical", s.uv.vertical)?;
    s.uv.feather = number(p, "feather", s.uv.feather)?;
    s.uv.open_borders = boolean(p, "openBorders", s.uv.open_borders)?;
    Ok(json!({"selected":true,"guard":s.uv.guard}))
}
pub(crate) fn has_selection(s: &Session) -> bool {
    !(s.uv_workspace.initialized && !s.uv_workspace.active) && (!s.uv.selected.is_empty() || !s.uv.selection_masks.is_empty())
}
/// Clear only selections still owned by the UV tool; preserve unrelated manually replaced masks.
pub(crate) fn deselect(s: &mut Session) -> Result<()> {
    if !has_selection(s) {
        return Ok(());
    }
    let active = s.active_index();
    let masks = std::mem::take(&mut s.uv.selection_masks);
    let result = (|| {
        for (id, mask) in masks {
            if let Some(index) = s.documents().iter().position(|d| d.doc.id.0 == id && d.doc.selection.as_ref() == Some(&mask)) {
                s.set_active(index);
                s.edit("Deselect UV shells", |doc, _| {
                    doc.selection = None;
                    Ok(())
                })?;
            }
        }
        Ok(())
    })();
    if let Some(index) = active {
        s.set_active(index);
    }
    s.uv.selected.clear();
    s.uv.generation = s.uv.generation.wrapping_add(1);
    result
}
fn paint_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let (id, surface) = selection_surface(s, p)?;
    let index = s.documents().iter().position(|d| d.doc.id == id).ok_or(EngineError::NoDocument)?;
    s.set_active(index);
    let layer = s.edit("UV protected paint layer", |doc, _| {
        let mut layer = Layer::raster("UV protected edits", doc.pixel_format());
        layer.mask = Some(LayerMask { surface: surface.try_convert(PixelFormat::GRAY8.with_sample(doc.depth))?, ..LayerMask::reveal_all() });
        let id = layer.id;
        doc.layers.push(layer);
        doc.selection = None;
        Ok(id)
    })?;
    s.select_layer(layer)?;
    Ok(json!({"layer":layer.0}))
}

fn inspect(s: &mut Session, _: &Value) -> Result<Value> {
    let scene = s.uv.scene.as_ref().ok_or_else(|| err("No mesh"))?;
    let topology = s.uv.topology.as_ref().ok_or_else(|| err("No topology"))?;
    Ok(
        json!({"source":s.uv.source,"triangles":scene.triangles.len(),"materials":scene.materials,"bindings":s.uv.bindings.iter().map(|d|d.map(|d|d.0)).collect::<Vec<_>>(),
        "shells":topology.shells.iter().enumerate().map(|(id,sh)|json!({"id":id,"material":sh.material,"neighbors":sh.neighbors,"triangles":sh.triangles.len()})).collect::<Vec<_>>(),
        "normalBindings":s.uv.normal_bindings.iter().map(|d|d.map(|d|d.0)).collect::<Vec<_>>(),"hiddenMaterials":s.uv.hidden_materials,"normalChannel":s.uv.normal_channel,"channel":s.uv.current_channel().key(),"mapBindings":s.uv.map_bindings.iter().map(|(k,ids)|(k.key(),ids.iter().map(|id|id.map(|v|v.0)).collect::<Vec<_>>())).collect::<std::collections::BTreeMap<_,_>>(),"guard":s.uv.guard,"horizontal":s.uv.horizontal,"vertical":s.uv.vertical,"warnings":scene.warnings,"nonManifoldEdges":topology.non_manifold_edges,"selected":s.uv.selected,"material":s.uv.material}),
    )
}

#[derive(Serialize, Deserialize)]
struct Project {
    version: u32,
    scene: Scene,
    source: String,
    #[serde(default)]
    uv_set: usize,
    #[serde(default)]
    source_stamp: Option<crate::uv_reload_cmds::SourceStamp>,
    selected: BTreeSet<usize>,
    #[serde(default)]
    selection_documents: Option<Vec<usize>>,
    material: usize,
    guard: f64,
    #[serde(default)]
    horizontal: f64,
    #[serde(default)]
    vertical: f64,
    feather: f64,
    open_borders: bool,
    documents: Vec<Option<String>>,
    #[serde(default)]
    normal_documents: Vec<Option<String>>,
    #[serde(default)]
    hidden_materials: BTreeSet<usize>,
    #[serde(default)]
    normal_channel: bool,
    #[serde(default)]
    extra_channel: Option<Channel>,
    #[serde(default)]
    map_documents: std::collections::BTreeMap<Channel, Vec<Option<String>>>,
    #[serde(default)]
    view: Value,
    #[serde(default)]
    document_paths: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    dirty_documents: Option<BTreeSet<String>>,
    #[serde(default)]
    assets: Vec<crate::uv_asset_cmds::Packed>,
    #[serde(default)]
    environment: Option<crate::uv_asset_cmds::Packed>,
    #[serde(default)]
    loose_documents: Vec<String>,
}
fn validate_view(view: &Value) -> Result<()> {
    if !view.is_null() && !view.is_object() {
        return Err(err("Project view must be an object"));
    }
    let mut stack = vec![(view, 0usize)];
    let (mut nodes, mut bytes) = (0usize, 0usize);
    while let Some((value, depth)) = stack.pop() {
        nodes += 1;
        if depth > 16 || nodes > 8192 || bytes > 65536 {
            return Err(err("Project view metadata exceeds its budget"));
        }
        match value {
            Value::String(s) => bytes = bytes.saturating_add(s.len()),
            Value::Array(a) => {
                if a.len() > 8192 {
                    return Err(err("Project view array is too large"));
                }
                stack.extend(a.iter().map(|v| (v, depth + 1)));
            }
            Value::Object(o) => {
                if o.len() > 8192 {
                    return Err(err("Project view object is too large"));
                }
                for (k, v) in o {
                    bytes = bytes.saturating_add(k.len());
                    stack.push((v, depth + 1));
                }
            }
            _ => {}
        }
    }
    if bytes > 65536 {
        return Err(err("Project view metadata exceeds 64 KiB"));
    }
    Ok(())
}
fn project_save(s: &mut Session, p: &Value) -> Result<Value> {
    let path = path(p)?;
    if !path.to_ascii_lowercase().ends_with(".pcuv") {
        return Err(err("UV projects must use the .pcuv extension"));
    }
    let view = p.get("view").unwrap_or(&s.uv.project_view);
    validate_view(view)?;
    let view = view.clone();
    let count = s.uv.scene.as_ref().ok_or_else(|| err("No mesh"))?.materials.len();
    if s.uv.bindings.len() != count || s.uv.normal_bindings.len() != count {
        return Err(err("Invalid material bindings"));
    }
    if s.uv.map_bindings.values().any(|ids| ids.get(count..).is_some_and(|extra| extra.iter().any(Option::is_some))) {
        return Err(err("Invalid auxiliary map length"));
    }
    if count > 4096 || s.uv.map_bindings.keys().any(|kind| matches!(kind, Channel::Material | Channel::Color | Channel::Normal)) {
        return Err(err("Invalid material map table"));
    }
    let slots: Vec<_> =
        s.uv.bindings
            .iter()
            .chain(&s.uv.normal_bindings)
            .copied()
            .chain(s.uv.map_bindings.values().flat_map(|ids| (0..count).map(move |i| ids.get(i).copied().flatten())))
            .collect();
    let mut zip = photocraft_format::zip::ZipWriter::new();
    let mut documents = Vec::new();
    let mut saved = std::collections::BTreeMap::<u64, String>::new();
    let mut document_paths = std::collections::BTreeMap::new();
    let mut dirty_documents = BTreeSet::new();
    for (i, id) in slots.iter().enumerate() {
        if let Some(doc) = id.and_then(|id| s.documents().iter().find(|d| d.doc.id == id)) {
            if let Some(name) = saved.get(&doc.doc.id.0) {
                documents.push(Some(name.clone()));
                continue;
            }
            let name = format!("documents/{i}.pcraft");
            let bytes = photocraft_format::save_to_bytes(&doc.doc, &Default::default()).map_err(|e| err(e.to_string()))?;
            zip.add(&name, &bytes).map_err(|e| err(e.to_string()))?;
            if let Some(path) = &doc.path {
                document_paths.insert(name.clone(), path.clone());
            }
            if s.uv.needs_export(doc) {
                dirty_documents.insert(name.clone());
            }
            saved.insert(doc.doc.id.0, name.clone());
            documents.push(Some(name));
        } else {
            documents.push(None);
        }
    }
    let mut loose_documents = Vec::new();
    if s.uv_workspace.active {
        for doc in s.documents() {
            if saved.contains_key(&doc.doc.id.0) {
                continue;
            }
            let name = format!("documents/loose-{}.pcraft", loose_documents.len());
            let bytes = photocraft_format::save_to_bytes(&doc.doc, &Default::default()).map_err(|e| err(e.to_string()))?;
            zip.add(&name, &bytes).map_err(|e| err(e.to_string()))?;
            if let Some(path) = &doc.path {
                document_paths.insert(name.clone(), path.clone());
            }
            if s.uv.needs_export(doc) {
                dirty_documents.insert(name.clone());
            }
            saved.insert(doc.doc.id.0, name.clone());
            loose_documents.push(name);
        }
    }
    let mut assets = Vec::new();
    for asset in &s.uv.assets {
        let entry = format!("assets/{}.bin", asset.id);
        zip.add(&entry, &asset.source.bytes).map_err(|e| err(e.to_string()))?;
        assets.push(crate::uv_asset_cmds::Packed { id: asset.id, name: asset.source.name.clone(), entry });
    }
    let environment_resource = if let Some(source) = view.get("environment").and_then(Value::as_str).filter(|v| !v.is_empty()) {
        if s.uv.project_view.get("environment").and_then(Value::as_str) == Some(source) && s.uv.environment.is_some() {
            s.uv.environment.clone()
        } else {
            let bytes = read_limited(source, crate::uv_asset_cmds::MAX_ASSET_BYTES)?;
            let name = Path::new(source).file_name().map_or_else(|| "sky.exr".into(), |n| n.to_string_lossy().into_owned());
            Some(crate::uv_asset_cmds::Resource { name, bytes: Arc::new(bytes) })
        }
    } else {
        None
    };
    let environment = if let Some(resource) = &environment_resource {
        let entry = "environment/source.bin".to_owned();
        zip.add(&entry, &resource.bytes).map_err(|e| err(e.to_string()))?;
        Some(crate::uv_asset_cmds::Packed { id: 0, name: resource.name.clone(), entry })
    } else {
        None
    };
    let mut extra = documents.split_off(s.uv.bindings.len() * 2);
    let normal_documents = documents.split_off(s.uv.bindings.len());
    let mut map_documents = std::collections::BTreeMap::new();
    for kind in s.uv.map_bindings.keys() {
        let tail = extra.split_off(count);
        map_documents.insert(*kind, extra);
        extra = tail;
    }
    let project = Project {
        version: 1,
        scene: s.uv.scene.as_deref().cloned().ok_or_else(|| err("No mesh"))?,
        source: s.uv.source.clone(),
        uv_set: s.uv.uv_set,
        source_stamp: s.uv.source_stamp.clone(),
        selected: s.uv.selected.clone(),
        selection_documents: Some(
            slots
                .iter()
                .enumerate()
                .filter_map(|(i, id)| {
                    id.and_then(|id| s.documents().iter().find(|d| d.doc.id == id))
                        .filter(|doc| s.uv.selection_masks.get(&doc.doc.id.0).is_some_and(|mask| doc.doc.selection.as_ref() == Some(mask)))
                        .map(|_| i)
                })
                .collect(),
        ),
        material: s.uv.material,
        guard: s.uv.guard,
        horizontal: s.uv.horizontal,
        vertical: s.uv.vertical,
        feather: s.uv.feather,
        open_borders: s.uv.open_borders,
        documents,
        normal_documents,
        hidden_materials: s.uv.hidden_materials.clone(),
        normal_channel: s.uv.normal_channel,
        extra_channel: s.uv.extra_channel,
        map_documents,
        view: view.clone(),
        document_paths,
        dirty_documents: Some(dirty_documents),
        assets,
        environment,
        loose_documents,
    };
    let manifest = serde_json::to_vec(&project).map_err(|e| err(e.to_string()))?;
    if manifest.len() > 128 * 1024 * 1024 {
        return Err(err("UV mesh manifest exceeds 128 MiB"));
    }
    zip.add("project.json", &manifest).map_err(|e| err(e.to_string()))?;
    let bytes = zip.finish().map_err(|e| err(e.to_string()))?;
    if bytes.len() as u64 > MAX_PROJECT_BYTES {
        return Err(err("UV project exceeds 2 GiB; save texture documents separately"));
    }
    crate::file_cmds::write_file(path, &bytes)?;
    // Checkpoints change only after the atomic container write succeeded. External
    // exports have their own revision: saving a project never writes source textures.
    s.uv.saved_revisions.clear();
    for st in &mut s.docs {
        if saved.contains_key(&st.doc.id.0) {
            s.uv.exported_revisions.entry(st.doc.id).or_insert(st.saved_revision.saturating_sub(st.view_revision));
            s.uv.saved_revisions.insert(st.doc.id, st.revision.saturating_sub(st.view_revision));
            st.saved_revision = st.revision;
        }
    }
    s.uv.saved_generation = s.uv.generation;
    s.uv.project_path = path.to_owned();
    s.uv.environment = environment_resource;
    s.uv.project_view = view;
    Ok(json!({"saved":path}))
}
fn project_load(s: &mut Session, p: &Value) -> Result<Value> {
    let previous: Vec<_> = s.documents().iter().filter(|_| s.uv_workspace.active && s.uv.scene.is_some()).map(|st| st.doc.id).collect();
    let bytes = read_limited(path(p)?, MAX_PROJECT_BYTES)?;
    let zip = photocraft_format::zip::ZipReader::new(&bytes).map_err(|e| err(e.to_string()))?;
    let json = zip.read_by_name("project.json", 128 * 1024 * 1024).map_err(|e| err(e.to_string()))?;
    let mut project: Project = serde_json::from_slice(&json).map_err(|e| err(e.to_string()))?;
    if project.version != 1 {
        return Err(err("Unsupported UV project version"));
    }
    validate_view(&project.view)?;
    if project.normal_documents.is_empty() {
        project.normal_documents = vec![None; project.scene.materials.len()];
    }
    project.scene.validate().map_err(|e| err(e.to_string()))?;
    let topology = Topology::build(&project.scene).map_err(|e| err(e.to_string()))?;
    if project.uv_set > 31
        || project.documents.len() != project.scene.materials.len()
        || project.normal_documents.len() != project.scene.materials.len()
        || project.hidden_materials.iter().any(|i| *i >= project.scene.materials.len())
        || project.material >= project.scene.materials.len()
        || project.selected.iter().any(|i| topology.shells.get(*i).is_none_or(|sh| sh.material != project.material))
        || [project.horizontal, project.vertical].iter().any(|v| !v.is_finite() || !(0.0..=1024.0).contains(v))
        || project.selection_documents.as_ref().is_some_and(|slots| {
            slots.len() > project.documents.len() + project.normal_documents.len() + project.map_documents.values().map(Vec::len).sum::<usize>()
                || slots
                    .iter()
                    .any(|i| *i >= project.documents.len() + project.normal_documents.len() + project.map_documents.values().map(Vec::len).sum::<usize>())
        })
        || !project.guard.is_finite()
        || !(0.0..=1024.0).contains(&project.guard)
        || !project.feather.is_finite()
        || !(0.0..=1024.0).contains(&project.feather)
    {
        return Err(err("Invalid UV project state"));
    }
    if project.map_documents.len() > 7
        || project
            .map_documents
            .iter()
            .any(|(kind, maps)| matches!(kind, Channel::Material | Channel::Color | Channel::Normal) || maps.len() != project.documents.len())
        || project.extra_channel.is_some_and(|kind| matches!(kind, Channel::Material | Channel::Color | Channel::Normal))
    {
        return Err(err("Invalid auxiliary map bindings"));
    }
    if project.assets.len() > 256 || project.loose_documents.len() > 4096 {
        return Err(err("Project resource count exceeds budget"));
    }
    let mut assets = Vec::new();
    let mut library_bytes = 0usize;
    let mut asset_ids = BTreeSet::new();
    for asset in &project.assets {
        if !asset_ids.insert(asset.id) {
            return Err(err("Duplicate library image id"));
        }
        let bytes = zip.read_by_name(&asset.entry, crate::uv_asset_cmds::MAX_ASSET_BYTES as usize).map_err(|e| err(e.to_string()))?;
        library_bytes = library_bytes.saturating_add(bytes.len());
        if library_bytes > crate::uv_asset_cmds::MAX_LIBRARY_BYTES {
            return Err(err("Library exceeds 512 MiB"));
        }
        assets.push(crate::uv_asset_cmds::decode(asset.id, asset.name.clone(), bytes)?);
    }
    let environment = project
        .environment
        .as_ref()
        .map(|env| -> Result<_> {
            if env.name.len() > 4096 || env.name.contains('\0') {
                return Err(err("Invalid environment name"));
            }
            let bytes = zip.read_by_name(&env.entry, crate::uv_asset_cmds::MAX_ASSET_BYTES as usize).map_err(|e| err(e.to_string()))?;
            Ok(crate::uv_asset_cmds::Resource { name: env.name.clone(), bytes: Arc::new(bytes) })
        })
        .transpose()?;
    let mut documents = Vec::new();
    let mut decoded = BTreeSet::new();
    let mut entries: Vec<_> = project.documents.iter().chain(&project.normal_documents).chain(project.map_documents.values().flatten()).cloned().collect();
    let bound_count = entries.len();
    entries.extend(project.loose_documents.iter().cloned().map(Some));
    let mut decoded_bytes = library_bytes;
    for entry in &entries {
        documents.push(if let Some(name) = entry {
            if decoded.insert(name.clone()) {
                let bytes = zip.read_by_name(name, MAX_PROJECT_BYTES as usize).map_err(|e| err(e.to_string()))?;
                decoded_bytes = decoded_bytes.saturating_add(bytes.len());
                if decoded_bytes as u64 > MAX_PROJECT_BYTES {
                    return Err(err("Expanded project exceeds 2 GiB"));
                }
                Some(photocraft_format::load_from_bytes(&bytes).map_err(|e| err(e.to_string()))?)
            } else {
                // Repeated material bindings restore the same document below. Decode
                // it once so a small project cannot multiply a large image in memory.
                None
            }
        } else {
            None
        });
    }
    for (doc, entry) in documents.iter().zip(&entries) {
        if doc.as_ref().is_some_and(|d| d.mode != ColorMode::Rgb) && project.normal_documents.iter().any(|name| name.as_ref() == entry.as_ref()) {
            return Err(err("Normal maps must use RGB mode"));
        }
    }
    let mut blocked_exports = std::collections::HashSet::new();
    let mut restored = std::collections::BTreeMap::new();
    let mut exported_revisions = std::collections::HashMap::new();
    let mut saved_revisions = std::collections::HashMap::new();
    let mut bindings: Vec<Option<DocId>> = documents
        .into_iter()
        .zip(entries)
        .map(|(doc, name)| {
            if let Some(id) = name.as_ref().and_then(|name| restored.get(name)).copied() {
                return Some(id);
            }
            doc.and_then(|doc| {
                let source = name.as_ref().and_then(|name| project.document_paths.get(name)).filter(|p| p.len() <= 32768 && !p.contains('\0')).cloned();
                // Legacy projects did not track external writeback. A linked embedded
                // texture needs one writeback; saving .pcuv alone never wrote its map file.
                let writeback = project.dirty_documents.as_ref().map_or(source.is_some(), |dirty| name.as_ref().is_some_and(|name| dirty.contains(name)));
                let external = source.as_ref().is_some_and(|source| {
                    !Path::new(source)
                        .canonicalize()
                        .ok()
                        .zip(Path::new(path(p).unwrap_or("")).parent().and_then(|p| p.canonicalize().ok()))
                        .is_some_and(|(file, base)| file.starts_with(base) && file.is_file())
                });
                let index = s.add_document(doc, source);
                if external && let Some(st) = s.docs.get(index) {
                    blocked_exports.insert(st.doc.id);
                }
                if let Some(st) = s.docs.get(index) {
                    exported_revisions.insert(st.doc.id, if writeback { 0 } else { st.revision });
                    saved_revisions.insert(st.doc.id, st.revision);
                }
                let id = s.documents().get(index).map(|d| d.doc.id);
                if let (Some(name), Some(id)) = (name, id) {
                    restored.insert(name.clone(), id);
                }
                id
            })
        })
        .collect();
    bindings.truncate(bound_count);
    let mut extra = bindings.split_off(project.documents.len() * 2);
    let normal_bindings = bindings.split_off(project.documents.len());
    let mut map_bindings = std::collections::BTreeMap::new();
    for kind in project.map_documents.keys() {
        let tail = extra.split_off(project.documents.len());
        map_bindings.insert(*kind, extra);
        extra = tail;
    }
    s.uv = UvState {
        blocked_exports,
        project_view: project.view,
        project_path: path(p)?.to_owned(),
        assets,
        environment,
        exported_revisions,
        saved_revisions,
        saved_generation: s.uv.generation.wrapping_add(1),
        wrap_mesh: None,
        wrapped_stroke: None,
        scene: Some(Arc::new(project.scene)),
        topology: Some(Arc::new(topology)),
        source: project.source,
        uv_set: project.uv_set,
        source_stamp: project.source_stamp,
        normal_bindings,
        hidden_materials: project.hidden_materials,
        normal_channel: project.normal_channel,
        extra_channel: project.extra_channel,
        map_bindings,
        selected: project.selected,
        selection_masks: Default::default(),
        bindings,
        material: project.material,
        guard: project.guard,
        horizontal: project.horizontal,
        vertical: project.vertical,
        feather: project.feather,
        open_borders: project.open_borders,
        generation: s.uv.generation.wrapping_add(1),
    };
    let slots = project
        .selection_documents
        .unwrap_or_else(|| if s.uv.selected.is_empty() { Vec::new() } else { vec![s.uv.material + if s.uv.normal_channel { s.uv.bindings.len() } else { 0 }] });
    let ids: Vec<_> = s.uv.all_bindings().copied().collect();
    for slot in slots {
        if let Some(id) = ids.get(slot).copied().flatten()
            && let Some(mask) = s.documents().iter().find(|d| d.doc.id == id).and_then(|d| d.doc.selection.clone())
        {
            s.uv.selection_masks.insert(id.0, mask);
        }
    }
    for id in previous {
        if let Some(index) = s.documents().iter().position(|st| st.doc.id == id) {
            s.close(index);
        }
    }
    let _ = activate(s, s.uv.material);
    let mut result = inspect(s, &Value::Null)?;
    result["view"] = s.uv.project_view.clone();
    Ok(result)
}

pub fn specs() -> Vec<CommandSpec> {
    let spec = |id, label, params, enabled, run| CommandSpec { id, label, params, enabled, run, menu: &[], shortcut: None, journal: id != "uv.inspect" };
    vec![
        spec("uv.model.import", "Import UV model", r#"{"path":string,"uvSet":int=0}"#, crate::commands::always, import),
        spec("uv.settings", "UV seam protection settings", r#"{"guard":px,"horizontal":px,"vertical":px,"feather":px,"openBorders":bool}"#, mesh, settings),
        spec(
            "uv.material.select",
            "Select material channel",
            r#"{"material":int,"channel":"baseColor|normal|id|ao|curvature|opacity|metallic|roughness|emissive"}"#,
            mesh,
            material_select,
        ),
        spec("uv.material.visible", "Show or hide material", r#"{"material":int,"visible":bool}"#, mesh, material_visible),
        spec("uv.inspect", "Inspect UV model", "{}", mesh, inspect),
        spec(
            "uv.texture.open",
            "Open material texture",
            r#"{"path":string,"material":int,"channel":"baseColor|normal|id|ao|curvature|opacity|metallic|roughness|emissive"}"#,
            mesh,
            texture_open,
        ),
        spec(
            "uv.texture.create",
            "Create material texture",
            r#"{"material":int,"channel":"baseColor|normal|id|ao|curvature|opacity|metallic|roughness|emissive","width":int=1024,"height":int=1024,"depth":8|16|32=8}"#,
            mesh,
            texture_create,
        ),
        spec(
            "uv.texture.bind",
            "Use active document as material texture",
            r#"{"material":int,"channel":"baseColor|normal|id|ao|curvature|opacity|metallic|roughness|emissive"}"#,
            mesh,
            texture_bind,
        ),
        spec("uv.shell.select", "Select UV shells", r#"{"shells":[int],"material":int,"mode":"replace|add|subtract|intersect"}"#, mesh, select),
        spec("uv.mask.apply", "Apply protected UV selection", r#"{"material":int,"guard":px=8,"feather":px=0,"openBorders":bool=false}"#, mesh, apply_mask),
        spec(
            "uv.layer.create",
            "Create protected UV paint layer",
            r#"{"material":int,"guard":px=8,"feather":px=0,"openBorders":bool=false}"#,
            mesh,
            paint_layer,
        ),
        spec("uv.project.save", "Save UV project", r#"{"path":string,"view":object?}"#, mesh, project_save),
        spec("uv.project.load", "Open UV project", r#"{"path":string}"#, crate::commands::always, project_load),
    ]
}

#[cfg(test)]
mod asset_path_tests {
    #[test]
    fn model_references_cannot_escape_the_asset_directory() {
        let root = tempfile::tempdir().unwrap();
        let assets = root.path().join("assets");
        std::fs::create_dir(&assets).unwrap();
        let model = assets.join("mesh.obj");
        std::fs::write(&model, "").unwrap();
        std::fs::write(assets.join("map.png"), "map").unwrap();
        std::fs::write(root.path().join("outside.png"), "outside").unwrap();
        let source = model.to_str().unwrap();
        assert!(!super::resolved(source, "map.png").is_empty());
        assert!(super::resolved(source, "../outside.png").is_empty());
        assert!(super::resolved(source, root.path().join("outside.png").to_str().unwrap()).is_empty());
        assert!(super::resolved(source, ".").is_empty());
    }
}
