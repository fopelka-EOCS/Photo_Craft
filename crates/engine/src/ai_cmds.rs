//! Import generated PNGs through the same command boundary as ordinary document edits.
use crate::{EngineError, Session, commands::CommandSpec};
use base64::Engine as _;
use serde_json::{Value, json};

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "ai.openGeneratedImage",
        label: "Open AI-generated image",
        menu: &[],
        shortcut: None,
        params: r#"{"pngBase64":string}"#,
        enabled: crate::commands::always,
        run: open,
        journal: false,
    }]
}

fn open(s: &mut Session, p: &Value) -> crate::Result<Value> {
    let fail = |message: &str| EngineError::Other(message.to_owned());
    let encoded = p.get("pngBase64").and_then(Value::as_str).ok_or_else(|| fail("Missing generated PNG"))?;
    if encoded.len() > 12 * 1024 * 1024 {
        return Err(fail("Generated image exceeds 9 MiB"));
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|_| fail("Invalid image encoding"))?;
    if bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n") || bytes.get(12..16) != Some(b"IHDR") {
        return Err(fail("The generator did not return a PNG image"));
    }
    let dimension = |range: std::ops::Range<usize>| -> Option<u32> { Some(u32::from_be_bytes(bytes.get(range)?.try_into().ok()?)) };
    let (w, h) = (dimension(16..20).unwrap_or(0), dimension(20..24).unwrap_or(0));
    if w == 0 || h == 0 || w > 1024 || h > 1024 {
        return Err(fail("Generated image must be between 1 and 1024 pixels per side"));
    }
    let result = crate::file_cmds::open_bytes_as(s, "AI Generated.png", &bytes, Some("png"), None)?;
    // Unlike an opened disk file, this generated document has never been saved.
    if let Some(st) = s.active_mut() {
        st.saved_revision = 0;
    }
    Ok(json!({"opened": result, "width": w, "height": h}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_input_preserves_existing_document() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":8,"height":8})).unwrap();
        let before = s.active().unwrap().doc.id;
        for p in [json!({}), json!({"pngBase64":"not base64"}), json!({"pngBase64":"aGVsbG8="})] {
            assert!(s.execute("ai.openGeneratedImage", p).is_err());
            assert_eq!(s.active().unwrap().doc.id, before);
        }
    }
    #[test]
    fn rejects_oversized_dimensions_before_decode() {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        bytes.extend_from_slice(&u32::MAX.to_be_bytes());
        bytes.extend_from_slice(&512u32.to_be_bytes());
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        let mut s = Session::new();
        assert!(s.execute("ai.openGeneratedImage", json!({"pngBase64":encoded})).is_err());
        assert!(s.active().is_none());
    }
}

#[cfg(test)]
mod import_tests {
    use super::*;
    #[test]
    fn generated_png_opens_as_an_unsaved_document_without_replacing_existing_work() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":8,"height":8})).unwrap();
        let original = s.active().unwrap().doc.id;
        s.execute(
            "ai.openGeneratedImage",
            json!({"pngBase64":"iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEklEQVR4nGMUqbjEwMDAxAAGAA/UAWKaEbxiAAAAAElFTkSuQmCC"}),
        )
        .unwrap();
        assert_eq!(s.documents().len(), 2);
        let active = s.active().unwrap();
        assert_ne!(active.doc.id, original);
        assert_eq!(active.doc.size.width, 2);
        assert!(active.path.is_none());
        assert!(active.is_dirty());
    }
}
