//! Isolated iOS C ABI. Upstream PhotoCraft is vendored unchanged.
use base64::Engine;
use photocraft_engine::Session;
use serde_json::{Value, json};
use std::{
    ffi::{CStr, CString, c_char},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Mutex, OnceLock},
};

static SESSION: OnceLock<Mutex<Session>> = OnceLock::new();
static WRITER: OnceLock<Mutex<photocraft_format::PcraftWriter>> = OnceLock::new();
const COMMANDS: &[&str] = &[
    "edit.undo",
    "edit.redo",
    "image.crop",
    "image.imageRotation.90cw",
    "image.imageRotation.flipCanvasHorizontal",
    "layer.select",
    "layer.setProps",
    "layer.duplicate",
    "layer.delete",
    "layer.newAdjustmentLayer.brightnessContrast",
    "layer.newAdjustmentLayer.hueSaturation",
    "layer.newAdjustmentLayer.invert",
];
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v.get(key).and_then(Value::as_str).ok_or_else(|| format!("Missing {key}"))
}
fn handle(s: &mut Session, request: &Value) -> Result<Value, String> {
    match field(request, "op")? {
        "open" => {
            let path = field(request, "path")?;
            let length = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
            if length > 384 * 1024 * 1024 {
                return Err("Datei überschreitet 384 MB.".into());
            }
            let bytes = photocraft_format::read_file(std::path::Path::new(path)).map_err(|e| e.to_string())?;
            if bytes.len() > 384 * 1024 * 1024 {
                return Err("Datei überschreitet 384 MB.".into());
            }
            let mut next = Session::new();
            if path.ends_with(".pcraft") {
                let opts = photocraft_format::LoadOptions {
                    max_canvas_side: 8192,
                    max_canvas_pixels: 50_000_000,
                    max_total_bytes: 768 * 1024 * 1024,
                    max_blob_bytes: 256 * 1024 * 1024,
                    ..Default::default()
                };
                let doc = photocraft_format::load_from_bytes_with(&bytes, &opts).map_err(|e| e.to_string())?;
                next.add_document(doc, Some(path.to_owned()));
            } else {
                // Native shell normalizes photos to a bounded, oriented PNG before import.
                photocraft_engine::file_cmds::open_bytes_as(&mut next, "Foto.png", &bytes, Some("png"), None).map_err(|e| e.to_string())?;
            }
            let d = next.active().ok_or("Kein Dokument")?;
            if d.doc.size.width > 8192 || d.doc.size.height > 8192 || u64::from(d.doc.size.width) * u64::from(d.doc.size.height) > 50_000_000 {
                return Err("Maximal 50 Megapixel und 8192 Pixel pro Seite.".into());
            }
            if d.doc.layers.len() > 24 {
                return Err("Maximal 24 Ebenen.".into());
            }
            if let Some(d) = next.active_mut() {
                d.history.max_states = 20;
                d.history.max_bytes = 256 * 1024 * 1024;
            }
            photocraft_compose::set_effect_cache_budget(32 * 1024 * 1024);
            *s = next;
        }
        "attachSource" => {
            let path = field(request, "path")?;
            if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 128 * 1024 * 1024 {
                return Err("Original überschreitet 128 MB.".into());
            }
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            let kind = field(request, "type")?;
            let initial = request.get("state").filter(|v| v.is_object()).cloned();
            let d = s.active_mut().ok_or("Kein Dokument")?;
            let doc = std::sync::Arc::make_mut(&mut d.doc);
            // Private, content-addressed binary resource: Arc shares the original across undo states.
            doc.metadata.psd_resources.retain(|(_, name, _)| !name.starts_with("photocraft.ios.source.v1:"));
            doc.metadata.psd_resources.push((4000, format!("photocraft.ios.source.v1:{kind}"), std::sync::Arc::new(bytes)));
            if let Some(state) = initial {
                doc.metadata.text.retain(|(k, _)| k != "photocraft.ios.development.v1");
                doc.metadata.text.push(("photocraft.ios.development.v1".into(), state.to_string()));
            }
        }
        "extractSource" => {
            let d = s.active().ok_or("Kein Dokument")?;
            if let Some((_, _, bytes)) = d.doc.metadata.psd_resources.iter().find(|(_, n, _)| n.starts_with("photocraft.ios.source.v1:")) {
                photocraft_format::atomic_write(std::path::Path::new(field(request, "path")?), bytes).map_err(|e| e.to_string())?;
                return Ok(json!({"ok":true,"path":request["path"]}));
            }
            return Ok(json!({"ok":true}));
        }
        "native" => {
            let state = request.get("state").ok_or("Fehlende Einstellungen")?.clone();
            if !state.is_object() {
                return Err("Ungültige Einstellungen".into());
            }
            s.edit("Foto entwickeln", |doc, _| {
                doc.metadata.text.retain(|(k, _)| k != "photocraft.ios.development.v1");
                doc.metadata.text.push(("photocraft.ios.development.v1".into(), state.to_string()));
                Ok(())
            })
            .map_err(|e| e.to_string())?;
            return Ok(json!({"ok":true,"document":photocraft_engine::inspect::document(s.active().unwrap())}));
        }
        "preview" => {
            let d = s.active().ok_or("Kein Dokument")?;
            if !photocraft_compose::proxy::proxy_faithful(&d.doc) {
                return Err("Live-Anpassung ist bei diesem importierten Projekt nicht verfügbar.".into());
            }
            let factor = d.doc.size.width.max(d.doc.size.height).div_ceil(900).max(1);
            let doc = photocraft_compose::proxy::proxy_document(&d.doc, factor);
            let mut preview = Session::new();
            preview.add_document(doc, None);
            develop(&mut preview, request.get("values").unwrap_or(&Value::Null), "preview")?;
            return snapshot(&preview, 900);
        }
        "develop" => {
            let key = field(request, "key")?;
            develop(s, request.get("values").unwrap_or(&Value::Null), key)?;
        }
        "command" => {
            let id = field(request, "id")?;
            if !COMMANDS.contains(&id) {
                return Err(format!("Nicht unterstützter Befehl: {id}"));
            }
            if (id.starts_with("layer.new") || id == "layer.duplicate") && s.active().is_some_and(|d| d.doc.layers.len() >= 24) {
                return Err("Maximal 24 Ebenen.".into());
            }
            let old_size = s.active().map(|d| (d.doc.size.width as f64, d.doc.size.height as f64));
            s.execute(id, request.get("params").cloned().unwrap_or(json!({}))).map_err(|e| e.to_string())?;
            if let Some((w, h)) = old_size
                && ["image.crop", "image.imageRotation.90cw", "image.imageRotation.flipCanvasHorizontal"].contains(&id)
                && let Some(d) = s.active_mut()
            {
                let doc = std::sync::Arc::make_mut(&mut d.doc);
                for (k, text) in &mut doc.metadata.text {
                    if k != "photocraft.ios.development.v1" {
                        continue;
                    }
                    if let Ok(mut state) = serde_json::from_str::<Value>(text) {
                        let p = &request["params"];
                        let transform = |point: &mut Value| {
                            let x = point["x"].as_f64().unwrap_or(0.0);
                            let y = point["y"].as_f64().unwrap_or(0.0);
                            let (nx, ny) = match id {
                                "image.crop" => (
                                    (x * w - p["x"].as_f64().unwrap_or(0.0)) / doc.size.width as f64,
                                    (y * h - p["y"].as_f64().unwrap_or(0.0)) / doc.size.height as f64,
                                ),
                                "image.imageRotation.90cw" => (1.0 - y, x),
                                _ => (1.0 - x, y),
                            };
                            point["x"] = json!(nx);
                            point["y"] = json!(ny);
                        };
                        if let Some(masks) = state["masks"].as_array_mut() {
                            for mask in masks {
                                transform(&mut mask["start"]);
                                transform(&mut mask["end"]);
                                if let Some(points) = mask["rasterCorners"].as_array_mut() {
                                    for point in points {
                                        transform(point);
                                    }
                                }
                                if let Some(strokes) = mask["strokes"].as_array_mut() {
                                    for stroke in strokes {
                                        if let Some(points) = stroke["points"].as_array_mut() {
                                            for point in points {
                                                transform(point);
                                            }
                                        }
                                        if let Some(radius) = stroke["radius"].as_f64() {
                                            stroke["radius"] = json!(radius * w / doc.size.width as f64);
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(edits) = state["sourceEdits"].as_array_mut() {
                            edits.push(json!({"kind":id,"x":p["x"].as_f64().unwrap_or(0.0)/w,"y":p["y"].as_f64().unwrap_or(0.0)/h,"width":doc.size.width as f64/w,"height":doc.size.height as f64/h}));
                        }
                        if let Some(spots) = state["advanced"]["repairs"].as_array_mut() {
                            for spot in spots {
                                transform(&mut spot["source"]);
                                transform(&mut spot["destination"]);
                                if let Some(radius) = spot["radius"].as_f64() {
                                    spot["radius"] = json!(radius * w / doc.size.width as f64);
                                }
                            }
                        }
                        *text = state.to_string();
                    }
                }
            }
        }
        "save" | "export" => {
            let path = field(request, "path")?;
            let d = s.active().ok_or("Kein Dokument")?;
            let bytes = if request["op"] == "save" {
                WRITER
                    .get_or_init(|| Mutex::new(photocraft_format::PcraftWriter::new()))
                    .lock()
                    .map_err(|_| "Speicherstatus muss neu gestartet werden")?
                    .save_zip(&d.doc, &Default::default())
                    .map_err(|e| e.to_string())?
                    .0
            } else {
                let format = field(request, "format")?;
                if !["png", "jpg"].contains(&format) {
                    return Err("Ungültiges Exportformat".into());
                }
                photocraft_io::export(&d.doc, format, &Default::default()).map_err(|e| e.to_string())?.bytes
            };
            photocraft_format::atomic_write(std::path::Path::new(path), &bytes).map_err(|e| e.to_string())?;
            return Ok(json!({"ok":true,"path":path}));
        }
        "state" => {}
        _ => return Err("Unbekannte Anfrage".into()),
    }
    snapshot(s, 1600)
}
fn snapshot(s: &Session, max_side: u32) -> Result<Value, String> {
    let Some(d) = s.active() else {
        return Ok(json!({"ok":true}));
    };
    let rgba = photocraft_compose::thumbnail(&d.doc, max_side);
    let mut encoded = Vec::new();
    use image::ImageEncoder;
    image::codecs::png::PngEncoder::new(&mut encoded)
        .write_image(&rgba.pixels, rgba.width, rgba.height, image::ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    Ok(json!({"ok":true,"document":photocraft_engine::inspect::document(d),
        "nativeState": d.doc.metadata.text.iter().find(|(k,_)| k == "photocraft.ios.development.v1").and_then(|(_,v)|serde_json::from_str::<Value>(v).ok()),
        "sourceType":d.doc.metadata.psd_resources.iter().find_map(|(_,n,_)|n.strip_prefix("photocraft.ios.source.v1:")),
        "development": development_values(s), "preview":base64::engine::general_purpose::STANDARD.encode(encoded)}))
}

const DEVELOP_NAMES: [&str; 4] = ["PhotoCraft · Belichtung", "PhotoCraft · Licht", "PhotoCraft · Farbe", "PhotoCraft · Weißabgleich"];
fn development_values(s: &Session) -> Value {
    let mut v = json!({"exposure":0.0,"brightness":0.0,"contrast":0.0,"saturation":0.0,"vibrance":0.0,"warmth":0.0,"tint":0.0});
    if let Some(d) = s.active() {
        for layer in &d.doc.layers {
            let info = photocraft_engine::inspect::layer(layer);
            let p = &info["adjustment"]["params"];
            match layer.name.as_str() {
                "PhotoCraft · Belichtung" => v["exposure"] = p["exposure"].clone(),
                "PhotoCraft · Licht" => {
                    v["brightness"] = p["brightness"].clone();
                    v["contrast"] = p["contrast"].clone();
                }
                "PhotoCraft · Farbe" => {
                    v["saturation"] = p["saturation"].clone();
                    v["vibrance"] = p["vibrance"].clone();
                }
                "PhotoCraft · Weißabgleich" => {
                    v["warmth"] = json!(-p["midtones"][2].as_f64().unwrap_or(0.0));
                    v["tint"] = p["midtones"][1].clone();
                }
                _ => {}
            }
        }
    }
    v
}
fn develop(s: &mut Session, values: &Value, key: &str) -> Result<(), String> {
    let number = |name: &str, min: f64, max: f64| -> Result<f64, String> {
        let value = values.get(name).and_then(Value::as_f64).ok_or_else(|| format!("Ungültiger Wert: {name}"))?;
        if !value.is_finite() || value < min || value > max {
            return Err(format!("Wert außerhalb des Bereichs: {name}"));
        }
        Ok(value)
    };
    let e = number("exposure", -4.0, 4.0)?;
    let b = number("brightness", -150.0, 150.0)?;
    let c = number("contrast", -50.0, 100.0)?;
    let sat = number("saturation", -100.0, 100.0)?;
    let vib = number("vibrance", -100.0, 100.0)?;
    let w = number("warmth", -100.0, 100.0)?;
    let t = number("tint", -100.0, 100.0)?;
    let before = s.active().ok_or("Kein Dokument")?.clone();
    let journal_len = s.journal.len();
    let configs = [
        ("exposure", json!({"exposure":e}), e != 0.0),
        ("brightnessContrast", json!({"brightness":b,"contrast":c}), b != 0.0 || c != 0.0),
        ("vibrance", json!({"saturation":sat,"vibrance":vib}), sat != 0.0 || vib != 0.0),
        ("colorBalance", json!({"midtones":[0.0,t,-w],"preserveLuminosity":true}), w != 0.0 || t != 0.0),
    ];
    let result = (|| {
        for ((kind, mut params, changed), name) in configs.into_iter().zip(DEVELOP_NAMES) {
            let existing = s
                .active()
                .and_then(|d| d.doc.layers.iter().rev().find(|l| l.name == name && photocraft_engine::inspect::layer(l)["adjustment"]["kind"] == kind))
                .map(|l| l.id.0);
            params["coalesce"] = json!(key);
            if let Some(id) = existing {
                params["layer"] = json!(id);
                s.execute("layer.setAdjustment", params).map_err(|e| e.to_string())?;
            } else if changed {
                if s.active().is_some_and(|d| d.doc.layers.len() >= 24) {
                    return Err("Maximal 24 Ebenen.".into());
                }
                if let Some(top) = s.active().and_then(|d| d.doc.top_layer()) {
                    s.execute("layer.select", json!({"layer":top.0})).map_err(|e| e.to_string())?;
                }
                params["name"] = json!(name);
                let created = s.execute(&format!("layer.newAdjustmentLayer.{kind}"), params).map_err(|e| e.to_string())?;
                s.execute("layer.setProps", json!({"layer":created["layer"],"name":name,"coalesce":key})).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    })();
    if result.is_err() {
        if let Some(d) = s.active_mut() {
            *d = before;
        }
        s.journal.truncate(journal_len);
    }
    result
}

/// # Safety
/// `request` must point to a live NUL-terminated UTF-8 string for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pc_request(request: *const c_char) -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<Value, String> {
        if request.is_null() {
            return Err("Null request".into());
        }
        // SAFETY: caller contract; Swift withCString keeps this alive until return.
        let input = unsafe { CStr::from_ptr(request) }.to_str().map_err(|e| e.to_string())?;
        if input.len() > 2 * 1024 * 1024 {
            return Err("Request too large".into());
        }
        let value: Value = serde_json::from_str(input).map_err(|e| e.to_string())?;
        let mut s = SESSION.get_or_init(|| Mutex::new(Session::new())).lock().map_err(|_| "Engine muss neu gestartet werden")?;
        handle(&mut s, &value)
    }));
    let value = match result {
        Ok(Ok(value)) => value,
        Ok(Err(e)) => json!({"ok":false,"error":e}),
        Err(_) => json!({"ok":false,"error":"Bearbeitung fehlgeschlagen. App bitte neu starten."}),
    };
    CString::new(value.to_string()).map(CString::into_raw).unwrap_or(std::ptr::null_mut())
}
/// # Safety
/// `response` must be a pointer returned by pc_request and not previously freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pc_free(response: *mut c_char) {
    if !response.is_null() {
        // SAFETY: owned allocation from pc_request, consumed exactly once by caller.
        drop(unsafe { CString::from_raw(response) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_resource_survives_save_and_undo() {
        let mut s = Session::new();
        let dir = std::env::temp_dir();
        let image_path = dir.join(format!("source-test-{}.png", std::process::id()));
        image::RgbaImage::from_pixel(20, 30, image::Rgba([20, 40, 60, 255])).save(&image_path).unwrap();
        handle(&mut s, &json!({"op":"open","path":image_path})).unwrap();
        let raw = dir.join(format!("original-{}.raw", std::process::id()));
        std::fs::write(&raw, b"original bytes").unwrap();
        handle(&mut s, &json!({"op":"attachSource","path":raw,"type":"public.camera-raw-image"})).unwrap();
        let state = json!({"development":{},"masks":[],"sourceEdits":[],"raw":{"temperature":5200}});
        handle(&mut s, &json!({"op":"native","state":state})).unwrap();
        handle(&mut s, &json!({"op":"command","id":"image.imageRotation.90cw"})).unwrap();
        assert_eq!(snapshot(&s, 100).unwrap()["nativeState"]["sourceEdits"][0]["kind"], "image.imageRotation.90cw");
        handle(&mut s, &json!({"op":"command","id":"edit.undo"})).unwrap();
        assert_eq!(snapshot(&s, 100).unwrap()["nativeState"], state);
        let project = dir.join(format!("source-{}.pcraft", std::process::id()));
        handle(&mut s, &json!({"op":"save","path":project})).unwrap();
        handle(&mut s, &json!({"op":"open","path":project})).unwrap();
        let out = dir.join(format!("extracted-{}.raw", std::process::id()));
        handle(&mut s, &json!({"op":"extractSource","path":out})).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"original bytes");
        for path in [image_path, raw, project, out] {
            let _ = std::fs::remove_file(path);
        }
    }
    #[test]
    fn native_settings_masks_undo_and_crop_roundtrip() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":100,"height":80})).unwrap();
        let state = json!({"development":{"exposure":1.0},"masks":[{"start":{"x":0.2,"y":0.25},"end":{"x":0.8,"y":0.75},"strokes":[]}]});
        handle(&mut s, &json!({"op":"native","state":state})).unwrap();
        assert_eq!(snapshot(&s, 100).unwrap()["nativeState"], state);
        handle(&mut s, &json!({"op":"command","id":"image.crop","params":{"x":10,"y":0,"width":80,"height":80}})).unwrap();
        assert_eq!(snapshot(&s, 100).unwrap()["nativeState"]["masks"][0]["start"]["x"], 0.125);
        handle(&mut s, &json!({"op":"command","id":"edit.undo"})).unwrap();
        assert_eq!(snapshot(&s, 100).unwrap()["nativeState"], state);
        let path = std::env::temp_dir().join(format!("photocraft-native-{}.pcraft", std::process::id()));
        handle(&mut s, &json!({"op":"save","path":path})).unwrap();
        let mut reopened = Session::new();
        handle(&mut reopened, &json!({"op":"open","path":path})).unwrap();
        assert_eq!(snapshot(&reopened, 100).unwrap()["nativeState"], state);
        std::fs::remove_file(path).unwrap();
        handle(&mut s, &json!({"op":"command","id":"edit.undo"})).unwrap();
        assert!(snapshot(&s, 100).unwrap()["nativeState"].is_null());
    }
    #[test]
    fn raster_mask_geometry_survives_crop_rotate_flip_and_undo() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":100,"height":80})).unwrap();
        let state = json!({"masks":[{"start":{"x":0.0,"y":0.0},"end":{"x":1.0,"y":1.0},"rasterPNG":"opaque-base64-data","rasterCorners":[{"x":0.0,"y":0.0},{"x":1.0,"y":0.0},{"x":0.0,"y":1.0}],"strokes":[]}]});
        handle(&mut s, &json!({"op":"native","state":state})).unwrap();
        for (id, params, expected) in [
            ("image.crop", json!({"x":20,"y":10,"width":60,"height":60}), json!({"x":-1.0/3.0,"y":-1.0/6.0})),
            ("image.imageRotation.90cw", json!({}), json!({"x":1.0,"y":0.0})),
            ("image.imageRotation.flipCanvasHorizontal", json!({}), json!({"x":1.0,"y":0.0})),
        ] {
            handle(&mut s, &json!({"op":"command","id":id,"params":params})).unwrap();
            let actual = snapshot(&s, 100).unwrap();
            assert_eq!(actual["nativeState"]["masks"][0]["rasterCorners"][0], expected);
            assert_eq!(actual["nativeState"]["masks"][0]["rasterPNG"], "opaque-base64-data");
            handle(&mut s, &json!({"op":"command","id":"edit.undo"})).unwrap();
            assert_eq!(snapshot(&s, 100).unwrap()["nativeState"], state);
        }
    }
    #[test]
    fn rejects_unsupported_commands_and_empty_documents() {
        let mut s = Session::new();
        assert!(handle(&mut s, &json!({"op":"command","id":"file.open"})).is_err());
        assert!(handle(&mut s, &json!({"op":"command","id":"image.crop"})).is_err());
    }
    #[test]
    fn real_engine_edit_undo_save_reopen() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":16,"height":12})).unwrap();
        for id in COMMANDS {
            assert!(photocraft_engine::commands::find(id).is_some(), "{id}");
        }
        let v = handle(&mut s, &json!({"op":"command","id":"image.imageRotation.90cw"})).unwrap();
        assert_eq!(v["document"]["width"], 12);
        handle(&mut s, &json!({"op":"command","id":"edit.undo"})).unwrap();
        assert_eq!(s.active().unwrap().doc.size.width, 16);
        let v = handle(&mut s, &json!({"op":"command","id":"layer.newAdjustmentLayer.brightnessContrast","params":{"brightness":20,"contrast":10}})).unwrap();
        assert_eq!(v["document"]["layers"].as_array().unwrap().len(), 2);
        let path = std::env::temp_dir().join(format!("photocraft-ios-{}.pcraft", std::process::id()));
        handle(&mut s, &json!({"op":"save","path":path})).unwrap();
        let mut reopened = Session::new();
        handle(&mut reopened, &json!({"op":"open","path":path})).unwrap();
        assert_eq!(reopened.active().unwrap().doc.layers.len(), 2);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn crop_export_and_import_failure_preserve_document() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":24,"height":16})).unwrap();
        let v = handle(&mut s, &json!({"op":"command","id":"image.crop","params":{"x":4,"y":0,"width":16,"height":16}})).unwrap();
        assert_eq!(v["document"]["width"], 16);
        assert!(handle(&mut s, &json!({"op":"open","path":"/not/a/real/file.png"})).is_err());
        assert_eq!(s.active().unwrap().doc.size.width, 16);
        let path = std::env::temp_dir().join(format!("photocraft-export-{}.png", std::process::id()));
        handle(&mut s, &json!({"op":"export","path":path,"format":"png"})).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let image = image::load_from_memory(&bytes).unwrap();
        assert_eq!((image.width(), image.height()), (16, 16));
        std::fs::remove_file(path).unwrap();
        for _ in 0..23 {
            handle(&mut s, &json!({"op":"command","id":"layer.duplicate"})).unwrap();
        }
        assert!(handle(&mut s, &json!({"op":"command","id":"layer.duplicate"})).is_err());
    }
    #[test]
    fn live_preview_is_non_destructive_and_develop_is_one_undo() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":20,"height":16})).unwrap();
        let values = json!({"exposure":1.0,"brightness":10.0,"contrast":20.0,"saturation":15.0,"vibrance":10.0,"warmth":30.0,"tint":5.0});
        let preview = handle(&mut s, &json!({"op":"preview","values":values})).unwrap();
        assert_eq!(s.active().unwrap().doc.layers.len(), 1);
        assert_eq!(preview["development"]["warmth"], 30.0);
        handle(&mut s, &json!({"op":"develop","values":values,"key":"gesture1"})).unwrap();
        assert_eq!(s.active().unwrap().doc.layers.len(), 5);
        let mut next = values.clone();
        next["exposure"] = json!(2.0);
        handle(&mut s, &json!({"op":"develop","values":next,"key":"gesture2"})).unwrap();
        assert_eq!(s.active().unwrap().doc.layers.len(), 5);
        handle(&mut s, &json!({"op":"command","id":"edit.undo"})).unwrap();
        assert_eq!(development_values(&s)["exposure"], 1.0);
        handle(&mut s, &json!({"op":"command","id":"edit.undo"})).unwrap();
        assert_eq!(s.active().unwrap().doc.layers.len(), 1);
        let mut invalid = values.clone();
        invalid["contrast"] = json!(1000);
        assert!(handle(&mut s, &json!({"op":"develop","values":invalid,"key":"bad"})).is_err());
        assert_eq!(s.active().unwrap().doc.layers.len(), 1);
    }
    #[test]
    fn malformed_ffi_is_an_error() {
        let input = CString::new("{bad").unwrap();
        unsafe {
            let p = pc_request(input.as_ptr());
            assert!(!p.is_null());
            let v: Value = serde_json::from_str(CStr::from_ptr(p).to_str().unwrap()).unwrap();
            assert_eq!(v["ok"], false);
            pc_free(p);
        }
    }
}
