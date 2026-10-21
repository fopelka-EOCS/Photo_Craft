//! Tile references owned by history snapshots. The snapshots keep these addresses alive;
//! COW edits cannot change a retained tile in place. Count occurrences (including aliases
//! within one document), so removing a state releases a tile only after its last reference.
use photocraft_doc::{Document, LayerContent, Surface};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub(crate) struct Tiles {
    refs: HashMap<usize, (u128, usize)>,
    bytes: u128,
    /// Lazy surfaces the states hold (see `Surface::lazy`), by identity: (references, one of
    /// them). Their tiles aren't counted per state, since they decode later (or never); the
    /// decoded ones are charged once, when the budget is checked.
    lazy: HashMap<usize, (u128, Surface)>,
}

pub(crate) fn surfaces(doc: &Document, mut visit: impl FnMut(&Surface)) {
    for (_, _, layer) in doc.walk() {
        if let Some(surface) = layer.surface() {
            visit(surface);
        }
        if let Some(mask) = &layer.mask {
            visit(&mask.surface);
        }
        if let Some(cache) = &layer.fill_cache {
            visit(&cache.surface);
        }
        if let LayerContent::Smart(smart) = &layer.content
            && let Some(mask) = &smart.filter_mask
        {
            visit(&mask.surface);
        }
        if let Some(video) = &layer.video {
            // Undo retains the whole clip, not only the displayed frame. Visit aliases too:
            // Tiles tracks every owner and counts their shared allocations only once.
            for frame in video.frames.iter().chain(video.original_frames.iter().flatten()) {
                visit(frame);
            }
        }
    }
    for channel in doc.channels.iter().chain(&doc.quick_mask) {
        visit(&channel.surface);
    }
    // A pattern fill or layer style copies its pattern into the document.
    for pattern in &doc.patterns {
        visit(&pattern.surface);
    }
    if let Some(selection) = &doc.selection {
        visit(selection);
    }
}

impl Tiles {
    // A lazy surface's tiles (see `Surface::lazy`) are not counted per state: they are not in
    // memory until decoded, and decoded ones are counted once, with the current document.
    pub(crate) fn add(&mut self, doc: &Document) {
        surfaces(doc, |surface| {
            if let Some(id) = surface.lazy_identity() {
                self.lazy.entry(id).or_insert_with(|| (0, surface.clone())).0 += 1;
            }
            for (_, tile) in surface.owned_tiles() {
                let count = self.refs.entry(Arc::as_ptr(tile) as usize).or_insert_with(|| {
                    self.bytes += tile.bytes().len() as u128;
                    (0, tile.bytes().len())
                });
                count.0 += 1;
            }
        });
    }

    pub(crate) fn remove(&mut self, doc: &Document) {
        surfaces(doc, |surface| {
            if let Some(id) = surface.lazy_identity()
                && let Some(entry) = self.lazy.get_mut(&id)
            {
                entry.0 -= 1;
                if entry.0 == 0 {
                    self.lazy.remove(&id);
                }
            }
            for (_, tile) in surface.owned_tiles() {
                let key = Arc::as_ptr(tile) as usize;
                if let Some(count) = self.refs.get_mut(&key) {
                    count.0 -= 1;
                    if count.0 == 0 {
                        self.bytes -= count.1 as u128;
                        self.refs.remove(&key);
                    }
                }
            }
        });
    }

    /// Retained bytes are incremental; only current tiles need examining below the limit.
    /// Current can change without recording (coalescing), and tile transfers can carry a
    /// different byte length, so a tile-count estimate must not underestimate its memory.
    pub(crate) fn fits(&self, current: &Document, budget: usize) -> bool {
        self.bytes <= budget as u128 && self.bytes_with(current) <= budget as u128
    }

    pub(crate) fn bytes_with(&self, current: &Document) -> u128 {
        let mut total = self.bytes;
        let mut seen = HashSet::new();
        let mut lazy = HashSet::new();
        surfaces(current, |surface| {
            for (_, tile) in surface.owned_tiles() {
                let key = Arc::as_ptr(tile) as usize;
                if !self.refs.contains_key(&key) && seen.insert(key) {
                    total += tile.bytes().len() as u128;
                }
            }
            if let Some((id, tiles)) = surface.lazy_decoded()
                && lazy.insert(id)
            {
                for (_, tile) in tiles {
                    if seen.insert(Arc::as_ptr(tile) as usize) {
                        total += tile.bytes().len() as u128;
                    }
                }
            }
        });
        // Lazy surfaces only History still holds, decoded since they were recorded.
        for (_, surface) in self.lazy.values() {
            if let Some((id, tiles)) = surface.lazy_decoded()
                && lazy.insert(id)
            {
                for (_, tile) in tiles {
                    if seen.insert(Arc::as_ptr(tile) as usize) {
                        total += tile.bytes().len() as u128;
                    }
                }
            }
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, Document, Layer, LayerContent, PixelFormat, Rect, SampleType, Size};

    /// A lazy layer decoded after its state was recorded is charged once, also when only History
    /// still holds it.
    #[test]
    fn decoded_lazy_tiles_held_by_history_are_counted() {
        let fmt = PixelFormat::RGBA8;
        let r = Rect::new(0, 0, 256, 256);
        let lazy = Surface::lazy(
            fmt,
            r,
            None,
            Box::new(move || {
                let mut s = Surface::new(fmt);
                s.fill_rect(r, &[1.0, 0.0, 0.0, 1.0]);
                s.write_pixel(0, 0, &[0.0, 0.0, 0.0, 1.0]);
                Ok(s)
            }),
        );
        let mut held = Document::new("held", Size::new(256, 256), ColorMode::Rgb, SampleType::U8);
        let mut layer = Layer::raster("hidden", fmt);
        layer.content = LayerContent::Raster(lazy.clone());
        held.layers.push(layer);
        let mut tiles = Tiles::default();
        tiles.add(&held);
        let empty = Document::new("current", Size::new(256, 256), ColorMode::Rgb, SampleType::U8);
        assert_eq!(tiles.bytes_with(&empty), 0, "pending: nothing in memory");
        lazy.tile_count();
        let tile = 256 * 256 * 4;
        assert_eq!(tiles.bytes_with(&empty), tile, "decoded, held by History only");
        assert_eq!(tiles.bytes_with(&held), tile, "and counted once when current holds it too");
        tiles.remove(&held);
        assert_eq!(tiles.bytes_with(&empty), 0);
    }
}
