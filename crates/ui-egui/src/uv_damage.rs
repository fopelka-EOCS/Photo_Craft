//! Retained COW snapshots recover sparse changes across multiple commits in one frame.
use photocraft_doc::{Document, LayerContent};
use photocraft_geom::Rect;
use std::sync::Arc;

pub(crate) fn regions(previous: &mut Option<Arc<Document>>, doc: &Arc<Document>, fallback: Option<Rect>) -> Vec<Rect> {
    let old = previous.replace(doc.clone());
    let regions = match sparse(old.as_deref(), doc) {
        Some(regions) => regions.into_iter().map(|r| fallback.map_or(r, |f| r.intersect(&f))).filter(|r| !r.is_empty()).collect(),
        None => vec![fallback.unwrap_or_else(|| doc.bounds())],
    };
    coalesce(regions, doc.bounds())
}

fn coalesce(mut regions: Vec<Rect>, bounds: Rect) -> Vec<Rect> {
    // Full adjustments must become one upload, not one submission per raster tile.
    regions.sort_by_key(|r| (r.y0, r.y1, r.x0, r.x1));
    let mut rows: Vec<Rect> = Vec::new();
    for r in regions {
        if let Some(last) = rows.last_mut()
            && last.y0 == r.y0
            && last.y1 == r.y1
            && r.x0 <= last.x1
        {
            last.x1 = last.x1.max(r.x1);
        } else {
            rows.push(r);
        }
    }
    rows.sort_by_key(|r| (r.x0, r.x1, r.y0, r.y1));
    let mut out: Vec<Rect> = Vec::new();
    for r in rows {
        if let Some(last) = out.last_mut()
            && last.x0 == r.x0
            && last.x1 == r.x1
            && r.y0 <= last.y1
        {
            last.y1 = last.y1.max(r.y1);
        } else {
            out.push(r);
        }
    }
    let area: u64 = out.iter().map(|r| u64::from(r.width()) * u64::from(r.height())).sum();
    if out.len() > 64 && area > u64::from(bounds.width()) * u64::from(bounds.height()) / 4 {
        vec![out.iter().fold(Rect::EMPTY, |a, b| a.union(b))]
    } else {
        out
    }
}

// Compare only changed encoded tiles. Equal rows are a byte comparison, without
// allocating float buffers or decoding untouched texels at a distant UV seam.
fn changed_bounds(a: Option<&[u8]>, b: Option<&[u8]>, default: &[u8], bpp: usize) -> Option<Rect> {
    let side = photocraft_geom::TILE_SIZE as usize;
    let stride = side.checked_mul(bpp)?;
    if stride == 0 {
        return None;
    }
    let mut bounds = Rect::EMPTY;
    for y in 0..side {
        let range = y * stride..(y + 1) * stride;
        let (a, b) = (a.and_then(|a| a.get(range.clone())), b.and_then(|b| b.get(range)));
        if a.zip(b).is_some_and(|(a, b)| a == b) {
            continue;
        }
        let equal = |x: usize| {
            let range = x * bpp..(x + 1) * bpp;
            a.and_then(|v| v.get(range.clone())).unwrap_or(default) == b.and_then(|v| v.get(range)).unwrap_or(default)
        };
        if let Some(left) = (0..side).find(|x| !equal(*x)) {
            let right = (left..side).rfind(|x| !equal(*x)).unwrap_or(left) + 1;
            bounds = bounds.union(&Rect::new(left as i32, y as i32, right as i32, y as i32 + 1));
        }
    }
    (!bounds.is_empty()).then_some(bounds)
}
fn sparse(old: Option<&Document>, doc: &Document) -> Option<Vec<Rect>> {
    let old = old.filter(|old| old.id == doc.id && old.size == doc.size && old.layers.len() == doc.layers.len())?;
    let mut probe = doc.clone();
    let mut regions = Vec::new();
    // Group/effect/adjustment changes use the compositor's conservative damage path.
    for (a, b) in old.layers.iter().zip(&mut probe.layers) {
        match (&a.content, &mut b.content) {
            (LayerContent::Raster(x), LayerContent::Raster(y)) => {
                if x.format() != y.format() || x.default_bytes() != y.default_bytes() {
                    return None;
                }
                let coords: std::collections::BTreeSet<_> = x.tiles().chain(y.tiles()).map(|(coord, _)| *coord).collect();
                for coord in coords {
                    let (a, b) = (x.tile(coord), y.tile(coord));
                    if a.zip(b).is_some_and(|(a, b)| Arc::ptr_eq(a, b)) {
                        continue;
                    }
                    if let Some(local) = changed_bounds(a.map(|t| t.bytes()), b.map(|t| t.bytes()), x.default_bytes(), x.format().bytes_per_pixel()) {
                        let origin = coord.rect();
                        regions.push(Rect::new(origin.x0 + local.x0, origin.y0 + local.y0, origin.x0 + local.x1, origin.y0 + local.y1));
                    }
                }
                *y = x.clone();
            }
            _ => return None,
        }
    }
    if !photocraft_engine::layer_multi_cmds::same_pixels(old, &probe) {
        return None;
    }
    let reach = crate::canvas::effect_reach(&doc.layers);
    Some(regions.into_iter().map(|r| r.inflate(reach).intersect(&doc.bounds())).filter(|r| !r.is_empty()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_adjustments_merge_tiles_but_distant_islands_remain_separate() {
        let _gpu = photocraft_testkit::gpu_lock();
        let bounds = Rect::from_xywh(0, 0, 4096, 4096);
        let tiles = (0..16).flat_map(|y| (0..16).map(move |x| Rect::from_xywh(x * 256, y * 256, 256, 256))).collect();
        assert_eq!(coalesce(tiles, bounds), vec![bounds]);
        let islands = vec![Rect::from_xywh(1, 1, 20, 20), Rect::from_xywh(3800, 1, 20, 20)];
        assert_eq!(coalesce(islands.clone(), bounds), islands);
    }
    #[test]
    fn uv_damage_recovers_distant_tiles_across_multiple_commits_and_undo() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", serde_json::json!({"width":4096,"height":4096})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let mut previous = Some(before.clone());
        for x in [20, 3800] {
            s.edit("point", |doc, layer| {
                doc.layer_mut(layer.unwrap()).unwrap().surface_mut().unwrap().write_pixel(x, 20, &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        }
        let current = s.active().unwrap().doc.clone();
        let rects = regions(&mut previous, &current, None);
        assert_eq!(rects.len(), 2);
        assert_eq!(rects.iter().map(|r| u64::from(r.width()) * u64::from(r.height())).sum::<u64>(), 2);
        s.execute("edit.undo", serde_json::json!({})).unwrap();
        assert_eq!(regions(&mut previous, &s.active().unwrap().doc, None).len(), 1);
    }
}
