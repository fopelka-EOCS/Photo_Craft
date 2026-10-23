//! Reimport geometry without reopening, replacing or modifying texture documents.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceStamp {
    size: u64,
    modified_ns: u64,
}
impl SourceStamp {
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn read(path: &str) -> Result<Option<Self>> {
        let metadata = std::fs::metadata(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
        let time = metadata.modified().map_err(|e| EngineError::Other(e.to_string()))?;
        let ns = time.duration_since(std::time::UNIX_EPOCH).map_err(|e| EngineError::Other(e.to_string()))?.as_nanos();
        Ok(Some(Self { size: metadata.len(), modified_ns: u64::try_from(ns).map_err(|_| EngineError::Other("Invalid source timestamp".into()))? }))
    }
    #[cfg(target_arch = "wasm32")]
    pub(super) fn read(_: &str) -> Result<Option<Self>> {
        Err(EngineError::Other("Source-file checking is available in native builds only".into()))
    }
}
fn enabled(s: &Session) -> std::result::Result<(), String> {
    if s.uv.scene.is_some() && !s.uv.source.is_empty() { Ok(()) } else { Err("Import a model with a source file first".into()) }
}
fn check(s: &mut Session, _: &Value) -> Result<Value> {
    match SourceStamp::read(&s.uv.source) {
        Ok(now) => {
            let changed = s.uv.source_stamp.is_some() && now != s.uv.source_stamp;
            if s.uv.source_stamp.is_none() {
                s.uv.source_stamp = now;
            }
            Ok(json!({"changed":changed,"tracked":s.uv.source_stamp.is_some(),"available":true}))
        }
        Err(error) => Ok(json!({"changed":false,"available":false,"error":error.to_string()})),
    }
}
fn reload(s: &mut Session, p: &Value) -> Result<Value> {
    let old = s.uv.scene.as_ref().ok_or_else(|| EngineError::Other("Import a model first".into()))?;
    let uv_set = match p.get("uvSet") {
        None => s.uv.uv_set,
        Some(value) => {
            value.as_u64().and_then(|v| usize::try_from(v).ok()).filter(|v| *v <= 31).ok_or_else(|| EngineError::Other("uvSet must be in 0..31".into()))?
        }
    };
    let (mut scene, stamp) = crate::uv_cmds::load_scene(&s.uv.source, uv_set)?;
    let topology = photocraft_uv::topology::Topology::build(&scene).map_err(|e| EngineError::Other(e.to_string()))?;
    let uv_changed = s.uv.uv_set != uv_set
        || old.triangles.len() != scene.triangles.len()
        || old.triangles.iter().zip(&scene.triangles).any(|(a, b)| a.corners.iter().zip(b.corners).any(|(a, b)| a.vertex != b.vertex || a.uv != b.uv));
    let mut old_names = BTreeMap::<&str, Vec<usize>>::new();
    let mut new_names = BTreeMap::<&str, usize>::new();
    for (id, material) in old.materials.iter().enumerate() {
        old_names.entry(&material.name).or_default().push(id);
    }
    for material in &scene.materials {
        *new_names.entry(&material.name).or_default() += 1;
    }
    let mapping: Vec<_> = scene
        .materials
        .iter()
        .map(|m| old_names.get(m.name.as_str()).filter(|ids| ids.len() == 1 && new_names.get(m.name.as_str()) == Some(&1)).and_then(|ids| ids.first()).copied())
        .collect();
    let bindings = mapping.iter().map(|id| id.and_then(|i| s.uv.bindings.get(i).copied().flatten())).collect();
    let normal_bindings = mapping.iter().map(|id| id.and_then(|i| s.uv.normal_bindings.get(i).copied().flatten())).collect();
    let map_bindings =
        s.uv.map_bindings.iter().map(|(kind, slots)| (*kind, mapping.iter().map(|id| id.and_then(|i| slots.get(i).copied().flatten())).collect())).collect();
    let hidden_materials = mapping.iter().enumerate().filter_map(|(new, old)| old.filter(|old| s.uv.hidden_materials.contains(old)).map(|_| new)).collect();
    let material = mapping.iter().position(|old| *old == Some(s.uv.material)).unwrap_or(0);
    for (id, old) in mapping.iter().enumerate() {
        if old.is_none() {
            scene.warnings.push(format!(
                "{}: new or ambiguous material; assign its texture documents manually",
                scene.materials.get(id).map_or("?", |m| m.name.as_str())
            ));
        }
    }
    if uv_changed {
        scene.warnings.push("UV/topology changed. Texture documents and existing masks were retained; review seam-protection masks.".into());
    }
    // Everything above is fallible. Publish the new geometry only after it has fully validated.
    s.uv.wrap_mesh = None;
    s.uv.wrapped_stroke = None;
    s.uv.scene = Some(Arc::new(scene));
    s.uv.topology = Some(Arc::new(topology));
    s.uv.bindings = bindings;
    s.uv.normal_bindings = normal_bindings;
    s.uv.map_bindings = map_bindings;
    s.uv.hidden_materials = hidden_materials;
    s.uv.material = material;
    s.uv.uv_set = uv_set;
    s.uv.source_stamp = stamp;
    s.uv.selected.clear();
    s.uv.generation = s.uv.generation.wrapping_add(1);
    Ok(json!({"reimported":true,"uvChanged":uv_changed,"triangles":s.uv.scene.as_ref().map_or(0,|s|s.triangles.len())}))
}
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "uv.model.reimport",
            label: "Reimport model geometry",
            params: r#"{"uvSet":int?}"#,
            enabled,
            run: reload,
            menu: &[],
            shortcut: None,
            journal: true,
        },
        CommandSpec {
            id: "uv.model.checkSource",
            label: "Check source model for changes",
            params: "{}",
            enabled,
            run: check,
            menu: &[],
            shortcut: None,
            journal: false,
        },
    ]
}
