//! Native-brush tangent wrapping. Geometry provides the chart; the shared paint
//! renderer retains flow, stroke opacity, native formats, blend modes and masks.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{DocId, LayerId};
use photocraft_geom::Rect;
use photocraft_paint::{BrushContext, BrushSettings, Stroke, StrokePoint, StrokeRenderer};
use photocraft_raster::Surface;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

pub(crate) struct WrappedStroke {
    doc: DocId,
    layer: Option<LayerId>,
    revision: u64,
    key: String,
    pre: Surface,
    brush: BrushSettings,
    renderer: StrokeRenderer,
    previous: Option<[f64; 3]>,
    pub(crate) pixels_per_unit: Option<f64>,
    index: u64,
    target: Value,
}
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::Other(message.into())
}
fn enabled(s: &Session) -> std::result::Result<(), String> {
    if s.uv.scene.is_none() {
        Err("Import a model first".into())
    } else if s.active().is_none() {
        Err("Assign an editable texture first".into())
    } else {
        Ok(())
    }
}
fn bilinear(data: &[f32], rect: Rect, p: [f64; 2]) -> f32 {
    let (x, y) = (p[0] - f64::from(rect.x0) - 0.5, p[1] - f64::from(rect.y0) - 0.5);
    let (ix, iy) = (x.floor() as i64, y.floor() as i64);
    let (fx, fy) = ((x - ix as f64) as f32, (y - iy as f64) as f32);
    let value = |x: i64, y: i64| {
        if x < 0 || y < 0 || x >= i64::from(rect.width()) || y >= i64::from(rect.height()) {
            0.0
        } else {
            data.get(y as usize * rect.width() as usize + x as usize).copied().unwrap_or(0.0)
        }
    };
    ((value(ix, iy) * (1.0 - fx) + value(ix + 1, iy) * fx) * (1.0 - fy) + (value(ix, iy + 1) * (1.0 - fx) + value(ix + 1, iy + 1) * fx) * fy).clamp(0.0, 1.0)
}
fn stamp(s: &mut Session, p: &Value) -> Result<Value> {
    if p.get("end").and_then(Value::as_bool) == Some(true) {
        s.uv.wrapped_stroke = None;
        return Ok(Value::Null);
    }
    let triangle = p.get("triangle").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).ok_or_else(|| bad("triangle is required"))?;
    let position: [f64; 3] = serde_json::from_value(p.get("position").cloned().ok_or_else(|| bad("position is required"))?)
        .map_err(|_| bad("position must contain three finite numbers"))?;
    if position.iter().any(|v| !v.is_finite() || v.abs() > 1e15) {
        return Err(bad("Invalid surface position"));
    }
    let pressure = p
        .get("pressure")
        .map_or(Some(1.0), Value::as_f64)
        .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
        .ok_or_else(|| bad("pressure must be in 0..1"))? as f32;
    let key = p
        .get("coalesce")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= 128)
        .ok_or_else(|| bad("A bounded gesture coalesce key is required"))?
        .to_owned();
    let scene = s.uv.scene.clone().ok_or_else(|| bad("No mesh"))?;
    let material = scene.triangles.get(triangle).ok_or_else(|| bad("Triangle out of range"))?.material;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if s.uv.map_bindings(s.uv.current_channel()).get(material).copied().flatten() != Some(st.doc.id) || s.uv.hidden_materials.contains(&material) {
        return Err(bad("Surface does not belong to the active editable texture"));
    }
    let size = [st.doc.size.width, st.doc.size.height];
    let (doc_id, active_layer, revision) = (st.doc.id, st.active_layer, st.revision);
    let mut stroke = if let Some(stroke) = s.uv.wrapped_stroke.take().filter(|old| {
        old.doc == doc_id
            && old.layer == if crate::channel_cmds::is_channel_target(p) { None } else { active_layer }
            && old.revision == revision
            && old.key == key
            && old.target.get("target") == p.get("target")
    }) {
        stroke
    } else {
        let cmd = if p.get("pencil").and_then(Value::as_bool) == Some(true) { "paint.pencil" } else { "paint.stroke" };
        let brush = if cmd == "paint.pencil" {
            crate::brush_cmds::pencil_brush(s, p)?
        } else {
            crate::brush_cmds::with_blend_mode(crate::brush_cmds::resolve_brush(s, p, cmd)?, p)
        };
        let (layer, mut brush, _) = crate::brush_cmds::stroke_target(s, p, brush)?;
        let st = s.active().ok_or(EngineError::NoDocument)?;
        let mut probe = (*st.doc).clone();
        let (target, lock) = crate::channel_cmds::target_surface(&mut probe, layer, p)?;
        let pre = target.clone();
        crate::brush_cmds::erase_locked(&mut brush, lock, s.tools.background);
        WrappedStroke {
            doc: st.doc.id,
            layer,
            revision: st.revision,
            key,
            renderer: StrokeRenderer::new(&brush, Some(pre.format()), 1.0),
            pre,
            brush,
            previous: None,
            pixels_per_unit: None,
            index: 0,
            target: p.clone(),
        }
    };
    // Reject sub-spacing samples before chart traversal/raster allocation.
    if let (Some(prev), Some(scale)) = (stroke.previous, stroke.pixels_per_unit) {
        let travelled = prev.iter().zip(position).map(|(a, b)| (a - b).powi(2)).sum::<f64>().sqrt();
        if travelled * scale < f64::from(stroke.brush.size * stroke.brush.spacing).max(0.5) {
            s.uv.wrapped_stroke = Some(stroke);
            return Ok(Value::Null);
        }
    }
    if s.uv.wrap_mesh.is_none() {
        s.uv.wrap_mesh = Some(Arc::new(photocraft_uv::wrap::Mesh::new(&scene).map_err(|e| bad(e.to_string()))?));
    }
    let mesh = s.uv.wrap_mesh.as_ref().ok_or_else(|| bad("Missing wrap topology"))?;
    // Spacing is measured on the surface, not across the UV seam's empty atlas gap.
    let mut brush = stroke.brush.clone();
    brush.seed = brush.seed.wrapping_add(stroke.index);
    let dabs = photocraft_paint::dabs(&Stroke { brush: brush.clone(), points: vec![StrokePoint::new(0.0, 0.0, pressure)] });
    let Some(dab) = dabs.first() else {
        s.uv.wrapped_stroke = Some(stroke);
        return Ok(Value::Null);
    };
    let ctx = BrushContext::new(&brush);
    let rect = ctx.dab_rect(dab, false).union(&ctx.dab_rect(dab, true));
    let count = u64::from(rect.width()) * u64::from(rect.height());
    if count > 4_194_304 {
        return Err(bad("Wrap stamp exceeds 4M pixels; reduce brush size"));
    }
    let reach = [rect.x0, rect.y0, rect.x1, rect.y1].into_iter().map(|v| f64::from(v).abs()).fold(0.0, f64::max) + 1.0;
    let patches = mesh.chart_scaled(&scene, triangle, position, size, reach, stroke.pixels_per_unit).map_err(|e| bad(e.to_string()))?;
    if let Some(root) = patches.first() {
        let t = scene.triangles.get(root.triangle).ok_or_else(|| bad("Missing triangle"))?;
        let (Some(a), Some(b)) = (t.corners.first().and_then(|c| scene.positions.get(c.vertex)), t.corners.get(1).and_then(|c| scene.positions.get(c.vertex)))
        else {
            return Err(bad("Missing wrap edge"));
        };
        let distance = a.iter().zip(b).map(|(a, b)| (a - b).powi(2)).sum::<f64>().sqrt();
        let pixels = (root.plane[0][0] - root.plane[1][0]).hypot(root.plane[0][1] - root.plane[1][1]);
        if distance > 1e-14 {
            stroke.pixels_per_unit = Some(pixels / distance);
        }
    }
    if let (Some(prev), Some(scale)) = (stroke.previous, stroke.pixels_per_unit) {
        let travelled = prev.iter().zip(position).map(|(a, b)| (a - b).powi(2)).sum::<f64>().sqrt();
        if travelled * scale < f64::from(brush.size * brush.spacing).max(0.5) {
            s.uv.wrapped_stroke = Some(stroke);
            return Ok(Value::Null);
        }
    }
    let (mut cov, mut depth, mut dual) = (Vec::new(), Vec::new(), Vec::new());
    ctx.rasterize_into(dab, false, rect, &mut cov, Some(&mut depth));
    ctx.rasterize(dab, true, rect, &mut dual);
    if depth.len() != cov.len() {
        depth.resize(cov.len(), 0.0);
    }
    // Tile maps merge duplicate UV coverage with max once per dab, never darkening
    // a shared edge twice or allocating the empty rectangle between distant shells.
    let mut tiles = BTreeMap::<(i32, i32), Vec<[f32; 3]>>::new();
    let mut visited = 0usize;
    let mut over_budget = false;
    for patch in patches {
        patch
            .rasterize(&scene, size, reach, |x, y, at| {
                visited = visited.saturating_add(1);
                if visited > 4_194_304 {
                    over_budget = true;
                    return;
                }
                let values = [bilinear(&cov, rect, at), bilinear(&dual, rect, at), bilinear(&depth, rect, at)];
                if values[0] <= 0.0 {
                    return;
                }
                let tile = tiles.entry((x.div_euclid(64), y.div_euclid(64))).or_insert_with(|| vec![[0.0; 3]; 4096]);
                if let Some(dst) = tile.get_mut((y.rem_euclid(64) * 64 + x.rem_euclid(64)) as usize) {
                    for (a, b) in dst.iter_mut().zip(values) {
                        *a = a.max(b);
                    }
                }
            })
            .map_err(|e| bad(e.to_string()))?;
        if over_budget {
            return Err(bad("Wrap footprint exceeds 4M texels; reduce brush size"));
        }
    }
    for ((x, y), tile) in tiles {
        let rect = Rect::from_xywh(x * 64, y * 64, 64, 64);
        let channels: [Vec<f32>; 3] = std::array::from_fn(|i| tile.iter().map(|p| p[i]).collect());
        if !stroke.renderer.accumulate_mapped(rect, &channels[0], &channels[1], &channels[2], dab) {
            return Err(bad("Invalid mapped brush coverage"));
        }
    }
    if stroke.renderer.bounds().is_empty() {
        s.uv.wrapped_stroke = Some(stroke);
        return Ok(Value::Null);
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let mut probe = (*st.doc).clone();
    let selection = probe.selection.clone();
    let (surface, lock) = crate::channel_cmds::target_surface(&mut probe, stroke.layer, p)?;
    let current = surface.clone();
    let cmyk = crate::brush_cmds::doc_cmyk_space(s);
    let damage = photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || stroke.renderer.composite(&stroke.pre, surface, selection.as_ref(), lock, false));
    if *surface == current {
        stroke.previous = Some(position);
        stroke.index = stroke.index.wrapping_add(1);
        s.uv.wrapped_stroke = Some(stroke);
        return Ok(json!({"damage":[0,0,0,0]}));
    }
    let painted = surface.clone();
    s.edit("Paint Wrap", |doc, _| {
        let (surface, _) = crate::channel_cmds::target_surface(doc, stroke.layer, p)?;
        *surface = painted;
        Ok(())
    })?;
    if let Some(st) = s.active_mut() {
        st.last_damage = Some(damage);
        stroke.revision = st.revision;
    }
    stroke.previous = Some(position);
    stroke.index = stroke.index.wrapping_add(1);
    s.uv.wrapped_stroke = Some(stroke);
    Ok(json!({"damage":[damage.x0,damage.y0,damage.width(),damage.height()]}))
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "paint.uvWrap",
        label: "Paint tangent wrap",
        params: r#"{"triangle":int,"position":[x,y,z],"pressure":0..1,"erase":bool,"pencil":bool,"target":string,"coalesce":string,"end":bool}"#,
        enabled,
        run: stamp,
        menu: &[],
        shortcut: None,
        journal: true,
    }]
}
