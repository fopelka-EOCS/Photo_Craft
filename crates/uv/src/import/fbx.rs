//! Binary FBX static meshes, preserving per-corner UV and control-point identities.
use crate::{Corner, Error, MAX_VERTICES, Material, Result, Scene};
use fbxcel::{
    low::v7400::AttributeValue as A,
    tree::{any::AnyTree, v7400::NodeHandle},
};
use glam::{DMat4, DQuat, DVec3, EulerRot};
use std::collections::{BTreeMap, BTreeSet};

fn child<'a>(n: NodeHandle<'a>, name: &str) -> Option<NodeHandle<'a>> {
    n.first_child_by_name(name)
}
fn string(n: NodeHandle<'_>, index: usize) -> Option<&str> {
    n.attributes().get(index)?.get_string()
}
fn id(n: NodeHandle<'_>) -> Option<i64> {
    n.attributes().first()?.get_i64()
}
fn text<'a>(n: NodeHandle<'a>, name: &str) -> Option<&'a str> {
    string(child(n, name)?, 0)
}
fn floats(n: NodeHandle<'_>, name: &str) -> Result<Vec<f64>> {
    let values = child(n, name).and_then(|n| n.attributes().first());
    match values {
        Some(A::ArrF64(a)) => Ok(a.clone()),
        Some(A::ArrF32(a)) => Ok(a.iter().map(|v| f64::from(*v)).collect()),
        _ => Err(Error(format!("FBX {name} array is missing"))),
    }
}
fn ints(n: NodeHandle<'_>, name: &str) -> Result<Vec<i32>> {
    child(n, name)
        .and_then(|n| n.attributes().first())
        .and_then(A::get_arr_i32)
        .map(|a| a.to_vec())
        .ok_or_else(|| Error(format!("FBX {name} array is missing")))
}
fn property<'a>(n: NodeHandle<'a>, name: &str) -> Option<NodeHandle<'a>> {
    child(n, "Properties70")?.children_by_name("P").find(|p| string(*p, 0) == Some(name))
}
fn vector(n: NodeHandle<'_>, name: &str, default: DVec3) -> DVec3 {
    property(n, name)
        .and_then(|p| {
            let a = p.attributes();
            Some(DVec3::new(a.get(4)?.get_f64()?, a.get(5)?.get_f64()?, a.get(6)?.get_f64()?))
        })
        .unwrap_or(default)
}
fn rotate(v: DVec3, order: i32) -> DMat4 {
    let v = v * std::f64::consts::PI / 180.0;
    let q = match order {
        1 => DQuat::from_euler(EulerRot::YZX, v.y, v.z, v.x),
        2 => DQuat::from_euler(EulerRot::XZY, v.x, v.z, v.y),
        3 => DQuat::from_euler(EulerRot::ZXY, v.z, v.x, v.y),
        4 => DQuat::from_euler(EulerRot::YXZ, v.y, v.x, v.z),
        5 => DQuat::from_euler(EulerRot::XYZ, v.x, v.y, v.z),
        _ => DQuat::from_euler(EulerRot::ZYX, v.z, v.y, v.x),
    };
    DMat4::from_quat(q)
}
fn integer(n: NodeHandle<'_>, name: &str) -> i32 {
    property(n, name).and_then(|p| p.attributes().get(4)).and_then(A::get_i32).unwrap_or(0)
}
fn transform(n: NodeHandle<'_>, geometric: bool) -> DMat4 {
    let v = |name, default| vector(n, name, default);
    let t = |x| DMat4::from_translation(x);
    if geometric {
        return t(v("GeometricTranslation", DVec3::ZERO))
            * rotate(v("GeometricRotation", DVec3::ZERO), 0)
            * DMat4::from_scale(v("GeometricScaling", DVec3::ONE));
    }
    let rp = v("RotationPivot", DVec3::ZERO);
    let sp = v("ScalingPivot", DVec3::ZERO);
    t(v("Lcl Translation", DVec3::ZERO))
        * t(v("RotationOffset", DVec3::ZERO))
        * t(rp)
        * rotate(v("PreRotation", DVec3::ZERO), 0)
        * rotate(v("Lcl Rotation", DVec3::ZERO), integer(n, "RotationOrder"))
        * rotate(v("PostRotation", DVec3::ZERO), 0).inverse()
        * t(-rp)
        * t(v("ScalingOffset", DVec3::ZERO))
        * t(sp)
        * DMat4::from_scale(v("Lcl Scaling", DVec3::ONE))
        * t(-sp)
}

pub(super) fn load(bytes: &[u8], uv_set: usize) -> Result<Scene> {
    if !bytes.starts_with(b"Kaydara FBX Binary") {
        return Err(Error("This build imports binary FBX 7.4+; export FBX as Binary from Maya (ASCII FBX is not supported)".into()));
    }
    let tree = AnyTree::from_seekable_reader(std::io::Cursor::new(bytes)).map_err(|e| Error(format!("FBX: {e}")))?;
    let AnyTree::V7400(_, tree, _) = tree else { return Err(Error("Unsupported FBX version".into())) };
    let root = tree.root();
    let objects = child(root, "Objects").ok_or_else(|| Error("FBX Objects missing".into()))?;
    let connections: Vec<(i64, i64, String)> = child(root, "Connections")
        .map(|c| {
            c.children_by_name("C")
                .filter_map(|n| {
                    let a = n.attributes();
                    Some((a.get(1)?.get_i64()?, a.get(2)?.get_i64()?, a.get(3).and_then(A::get_string).unwrap_or("").to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    let by_id: BTreeMap<i64, NodeHandle<'_>> = objects.children().filter_map(|n| Some((id(n)?, n))).collect();
    let mut scene = Scene::default();
    let mut materials = BTreeMap::new();
    for m in objects.children_by_name("Material") {
        let Some(mid) = id(m) else { continue };
        let color = vector(m, "DiffuseColor", DVec3::ONE);
        let texture = connections
            .iter()
            .filter(|(_, to, prop)| *to == mid && (prop == "DiffuseColor" || prop == "Diffuse" || prop == "BaseColor"))
            .find_map(|(from, _, _)| by_id.get(from).copied().filter(|n| n.name() == "Texture"));
        let filename = texture.and_then(|t| text(t, "RelativeFilename").filter(|s| !s.is_empty()).or_else(|| text(t, "FileName"))).map(str::to_owned);
        let embedded = texture.and_then(id).and_then(|tid| {
            connections.iter().find_map(|(from, to, _)| {
                if *to != tid {
                    return None;
                }
                let video = by_id.get(from).copied().filter(|n| n.name() == "Video")?;
                child(video, "Content")?.attributes().first()?.get_binary().map(|b| b.to_vec())
            })
        });
        let normal_texture = connections
            .iter()
            .filter(|(_, to, prop)| *to == mid && (prop == "NormalMap" || prop == "normalCamera"))
            .find_map(|(from, _, _)| by_id.get(from).copied().filter(|n| n.name() == "Texture"));
        let normal_map = normal_texture.and_then(|t| text(t, "RelativeFilename").filter(|s| !s.is_empty()).or_else(|| text(t, "FileName"))).map(str::to_owned);
        let embedded_normal = normal_texture.and_then(id).and_then(|tid| {
            connections.iter().find_map(|(from, to, _)| {
                if *to != tid {
                    return None;
                }
                let video = by_id.get(from).copied().filter(|n| n.name() == "Video")?;
                child(video, "Content")?.attributes().first()?.get_binary().map(|b| b.to_vec())
            })
        });
        let mut maps = BTreeMap::new();
        for (kind, properties) in [
            (crate::preview::Channel::Id, &["MaterialID", "ID"][..]),
            (crate::preview::Channel::Ao, &["AmbientOcclusion", "AmbientOcclusionMap"][..]),
            (crate::preview::Channel::Curvature, &["Curvature"][..]),
            (crate::preview::Channel::Opacity, &["Opacity", "OpacityMap", "TransparentColor"][..]),
            (crate::preview::Channel::Metallic, &["Metalness", "Metallic"][..]),
            (crate::preview::Channel::Roughness, &["Roughness", "DiffuseRoughness"][..]),
            (crate::preview::Channel::Emissive, &["EmissiveColor", "Emissive", "Emission"][..]),
        ] {
            if let Some(texture) = connections
                .iter()
                .filter(|(_, to, prop)| *to == mid && properties.contains(&prop.as_str()))
                .find_map(|(from, _, _)| by_id.get(from).copied().filter(|n| n.name() == "Texture"))
                && let Some(name) = text(texture, "RelativeFilename").filter(|s| !s.is_empty()).or_else(|| text(texture, "FileName"))
            {
                maps.insert(kind, name.to_owned());
            }
        }
        materials.insert(mid, scene.materials.len());
        scene.materials.push(Material {
            name: string(m, 1).unwrap_or("Material").split('\0').next().unwrap_or("Material").trim_start_matches("Material::").to_owned(),
            color: [color.x as f32, color.y as f32, color.z as f32, 1.0],
            base_color: filename,
            embedded,
            normal_map,
            embedded_normal,
            maps,
        });
    }
    let default_material = scene.materials.len();
    scene.materials.push(Material::default());
    let mut warned_skin = false;
    for geometry in objects.children_by_name("Geometry").filter(|n| string(*n, 2) == Some("Mesh")) {
        let gid = id(geometry).ok_or_else(|| Error("FBX geometry id missing".into()))?;
        let vertices = floats(geometry, "Vertices")?;
        let polygons = ints(geometry, "PolygonVertexIndex")?;
        if !vertices.len().is_multiple_of(3) || vertices.len() / 3 > MAX_VERTICES {
            return Err(Error("Invalid FBX vertex array".into()));
        }
        let uv = geometry
            .children_by_name("LayerElementUV")
            .nth(uv_set)
            .ok_or_else(|| Error(format!("FBX mesh has no UV set {uv_set}; unwrap it before import")))?;
        let uv_values = floats(uv, "UV")?;
        let uv_indices = if text(uv, "ReferenceInformationType") == Some("IndexToDirect") { Some(ints(uv, "UVIndex")?) } else { None };
        let mapping = text(uv, "MappingInformationType").unwrap_or("ByPolygonVertex");
        let normals = child(geometry, "LayerElementNormal");
        let normal_values = normals.map(|n| floats(n, "Normals")).transpose()?.unwrap_or_default();
        let normal_mapping = normals.and_then(|n| text(n, "MappingInformationType")).unwrap_or("ByPolygonVertex");
        let normal_indices = normals
            .filter(|n| matches!(text(*n, "ReferenceInformationType"), Some("IndexToDirect" | "Index")))
            .map(|n| ints(n, if child(n, "NormalsIndex").is_some() { "NormalsIndex" } else { "NormalIndex" }))
            .transpose()?;
        let material_layer = child(geometry, "LayerElementMaterial");
        let material_indices = material_layer.and_then(|n| ints(n, "Materials").ok()).unwrap_or_default();
        let all_same = material_layer.and_then(|n| text(n, "MappingInformationType")) == Some("AllSame");
        let models: Vec<NodeHandle<'_>> =
            connections.iter().filter(|(from, _, _)| *from == gid).filter_map(|(_, to, _)| by_id.get(to).copied().filter(|n| n.name() == "Model")).collect();
        if models.is_empty() {
            scene.warnings.push("Unconnected FBX geometry skipped".into());
        }
        for model in models {
            let mid = id(model).ok_or_else(|| Error("Missing model id".into()))?;
            let mut matrix = transform(model, false);
            let mut current = mid;
            let mut seen = BTreeSet::new();
            seen.insert(mid);
            while let Some(parent) =
                connections.iter().filter(|(from, _, _)| *from == current).find_map(|(_, to, _)| by_id.get(to).copied().filter(|n| n.name() == "Model"))
            {
                current = id(parent).ok_or_else(|| Error("Missing parent id".into()))?;
                if seen.len() >= 128 || !seen.insert(current) {
                    return Err(Error("Cyclic or overly deep FBX hierarchy".into()));
                }
                matrix = transform(parent, false) * matrix;
            }
            matrix *= transform(model, true);
            let reflected = matrix.determinant() < 0.0;
            let normal_matrix = matrix.inverse().transpose();
            if !normal_matrix.is_finite() {
                return Err(Error("FBX has a singular mesh transform; freeze transforms before importing".into()));
            }
            if integer(model, "InheritType") != 0 {
                scene.warnings.push("FBX nondefault transform inheritance is approximated; freeze mesh transforms for exact placement".into());
            }
            let attached: Vec<usize> = connections.iter().filter(|(_, to, _)| *to == mid).filter_map(|(from, _, _)| materials.get(from).copied()).collect();
            let offset = scene.positions.len();
            if offset.saturating_add(vertices.len() / 3) > MAX_VERTICES {
                return Err(Error("Too many FBX vertices".into()));
            }
            scene.positions.extend(vertices.as_chunks::<3>().0.iter().map(|&[x, y, z]| matrix.transform_point3(DVec3::new(x, y, z)).to_array()));
            let mut face = Vec::new();
            let mut polygon = 0;
            for (corner, &raw) in polygons.iter().enumerate() {
                let vertex = usize::try_from(if raw < 0 { !raw } else { raw }).map_err(|_| Error("Invalid FBX vertex index".into()))?;
                if vertex >= vertices.len() / 3 {
                    return Err(Error("FBX vertex index is out of range".into()));
                }
                let mapped = match mapping {
                    "ByPolygonVertex" => corner,
                    "ByVertice" | "ByVertex" => vertex,
                    "ByPolygon" => polygon,
                    "AllSame" => 0,
                    _ => return Err(Error(format!("Unsupported FBX UV mapping: {mapping}"))),
                };
                let direct = if let Some(indices) = &uv_indices {
                    indices.get(mapped).copied().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| Error("FBX UV index out of range".into()))?
                } else {
                    mapped
                };
                let at = direct.checked_mul(2).ok_or_else(|| Error("FBX UV index overflow".into()))?;
                let (Some(&u), Some(&v)) = (uv_values.get(at), uv_values.get(at + 1)) else { return Err(Error("FBX UV coordinates out of range".into())) };
                let normal = if normals.is_some() {
                    let mapped = match normal_mapping {
                        "ByPolygonVertex" => corner,
                        "ByVertice" | "ByVertex" => vertex,
                        "ByPolygon" => polygon,
                        "AllSame" => 0,
                        _ => return Err(Error("Unsupported FBX normal mapping".into())),
                    };
                    let direct = if let Some(indices) = &normal_indices {
                        indices.get(mapped).and_then(|v| usize::try_from(*v).ok()).ok_or_else(|| Error("FBX normal index out of range".into()))?
                    } else {
                        mapped
                    };
                    let at = direct.checked_mul(3).ok_or_else(|| Error("FBX normal index overflow".into()))?;
                    let xyz = normal_values.get(at..at.saturating_add(3)).ok_or_else(|| Error("FBX normals out of range".into()))?;
                    let &[x, y, z] = xyz else { return Err(Error("Invalid FBX normal".into())) };
                    let n = normal_matrix.transform_vector3(DVec3::new(x, y, z));
                    if !n.is_finite() || n.length_squared() < 1e-24 {
                        return Err(Error("Invalid FBX normal".into()));
                    }
                    Some(n.normalize().to_array())
                } else {
                    None
                };
                face.push(Corner { vertex: vertex + offset, uv: [u, v], normal });
                if face.len() > 65_536 {
                    return Err(Error("FBX polygon is too large".into()));
                }
                if raw < 0 {
                    let slot = material_indices.get(if all_same { 0 } else { polygon }).copied().unwrap_or(0);
                    let material = usize::try_from(slot).ok().and_then(|i| attached.get(i)).copied().unwrap_or(default_material);
                    if reflected {
                        face.reverse();
                    }
                    if super::triangulate(&scene.positions, &face, material, &mut scene.triangles)? && scene.warnings.len() < 100 {
                        scene.warnings.push("FBX: redundant or degenerate polygon corners omitted during triangulation".into());
                    }
                    face.clear();
                    polygon += 1;
                }
            }
            if !face.is_empty() {
                return Err(Error("Unterminated FBX polygon".into()));
            }
        }
        if objects.children_by_name("Deformer").next().is_some() {
            warned_skin = true;
        }
    }
    if warned_skin {
        scene.warnings.push("Skinning and animation are not evaluated; the original static mesh is shown".into());
    }
    scene.warnings.push("GPU Material supports Color, Normal, Metallic, Roughness, AO and Emissive; height/bump is not evaluated".into());
    scene.warnings.sort();
    scene.warnings.dedup();
    Ok(scene)
}
