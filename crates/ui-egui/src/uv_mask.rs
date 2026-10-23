//! Native channel/mask display projected through each material UV, independent of source maps.
use crate::PhotocraftApp;
pub(crate) fn source(app: &PhotocraftApp, material: usize) -> Option<usize> {
    let index = app.session.active_index()?;
    let st = app.session.documents().get(index)?;
    if st.channel_view.is_plain(&st.doc) {
        return None;
    }
    photocraft_uv::preview::Channel::MAPS.iter().any(|kind| app.session.uv.map_bindings(*kind).get(material) == Some(&Some(st.doc.id))).then_some(index)
}
pub(crate) fn key(app: &PhotocraftApp, index: usize) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    if let Some(st) = app.session.documents().get(index) {
        serde_json::to_string(&st.channel_view).unwrap_or_default().hash(&mut h);
        st.active_layer.hash(&mut h);
    }
    let look = crate::channel_view::Look::of(&app.session);
    look.in_color.hash(&mut h);
    look.mask_rgb.map(f32::to_bits).hash(&mut h);
    look.mask_opacity.to_bits().hash(&mut h);
    h.finish()
}
pub(crate) fn overlay(app: &PhotocraftApp, index: usize, doc: &photocraft_doc::Document, rect: photocraft_geom::Rect, pixels: &mut [[f32; 4]], factor: u32) {
    let Some(st) = app.session.documents().get(index) else {
        return;
    };
    if let Some(px) = crate::channel_view::render(doc, &st.channel_view, rect, factor, crate::channel_view::Look::of(&app.session), st.active_layer) {
        for (out, p) in pixels.iter_mut().zip(px) {
            let rgba = p.to_array().map(|v| v as f32 / 255.0);
            for i in 0..3 {
                out[i] = rgba[i] + out[i] * (1.0 - rgba[3]);
            }
            out[3] = 1.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uv_mask_cache_follows_native_overlay_display_options() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let first = key(&app, 0);
        app.session.prefs.edit(|p| p.layer_mask_overlay.color = "#00ff00".into());
        let green = key(&app, 0);
        assert_ne!(first, green);
        app.session.prefs.edit(|p| p.layer_mask_overlay.opacity = 75.0);
        assert_ne!(green, key(&app, 0));
    }
}
