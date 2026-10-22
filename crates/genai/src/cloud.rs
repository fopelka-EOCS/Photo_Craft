//! User-configured HTTPS image editing endpoints. No sockets or credentials in preferences.
use std::io::Cursor;

use crate::{ApiKey, Capabilities, EditRequest, GenError, GeneratedImage, HttpRequest, HttpTransport, ImageProvider, ProviderInfo, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

pub const OPENAI: &str = "openai-images";
pub const GEMINI: &str = "gemini-content";

pub struct CloudProvider<T: HttpTransport> {
    protocol: &'static str,
    base: url::Url,
    model: String,
    key: ApiKey,
    transport: T,
}

impl<T: HttpTransport> CloudProvider<T> {
    pub fn new(protocol: &str, base: &str, model: &str, key: ApiKey, transport: T) -> Result<Self> {
        let protocol = match protocol {
            OPENAI => OPENAI,
            GEMINI => GEMINI,
            _ => return Err(GenError::Invalid("unsupported image editing protocol".into())),
        };
        let base = url::Url::parse(base.trim().trim_end_matches('/')).map_err(|_| GenError::Invalid("enter a valid HTTPS API base URL".into()))?;
        if base.scheme() != "https"
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(GenError::Invalid("API URL must use HTTPS without credentials, query or fragment".into()));
        }
        let model = model.trim();
        if model.is_empty() || model.len() > 160 || model.chars().any(char::is_control) {
            return Err(GenError::Invalid("enter a model name (1–160 characters)".into()));
        }
        Ok(Self { protocol, base, model: model.into(), key, transport })
    }

    fn endpoint(&self, tail: &str) -> String {
        format!("{}{tail}", self.base.as_str().trim_end_matches('/'))
    }

    fn send(&self, url: String, content_type: String, body: Vec<u8>) -> Result<Value> {
        let auth =
            if self.protocol == OPENAI { ("authorization", format!("Bearer {}", self.key.expose())) } else { ("x-goog-api-key", self.key.expose().to_owned()) };
        let response = self.transport.post(HttpRequest { url, headers: vec![auth, ("content-type", content_type)], body })?;
        if response.body.len() > crate::MAX_RESPONSE {
            return Err(GenError::BadResponse("response exceeds size limit"));
        }
        if response.status == 401 || response.status == 403 {
            return Err(GenError::Auth);
        }
        if response.status == 429 {
            return Err(GenError::Quota);
        }
        if !(200..300).contains(&response.status) {
            // Provider bodies can echo credentials or personal image descriptions. Never display them.
            return Err(GenError::Http { status: response.status, message: "check endpoint, model and provider dashboard".into() });
        }
        serde_json::from_slice(&response.body).map_err(|_| GenError::BadResponse("expected JSON response"))
    }

    fn decoded(&self, value: &str, mime: &str) -> Result<GeneratedImage> {
        if value.len() > crate::MAX_RESPONSE {
            return Err(GenError::BadResponse("encoded image is too large"));
        }
        let bytes = STANDARD.decode(value).map_err(|_| GenError::BadResponse("invalid base64 image"))?;
        if bytes.is_empty() {
            return Err(GenError::BadResponse("empty image"));
        }
        Ok(GeneratedImage { bytes, mime: mime.into() })
    }
}

impl<T: HttpTransport> ImageProvider for CloudProvider<T> {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: self.protocol,
            name: if self.protocol == OPENAI { "OpenAI-compatible Images" } else { "Gemini-compatible Images" },
            model: self.model.clone(),
            host: self.base.host_str().unwrap_or("").to_owned(),
        }
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { edit: true, ..Capabilities::default() }
    }
    fn edit_aspect(&self, w: u32, h: u32) -> (u32, u32) {
        if self.protocol == OPENAI { crate::nearest_ratio(w, h, &[(1, 1), (3, 2), (2, 3)]).unwrap_or((1, 1)) } else { (w, h) }
    }
    fn edit(&self, request: &EditRequest) -> Result<GeneratedImage> {
        crate::check_prompt(&request.prompt, false)?;
        let size = crate::png_size(&request.image).ok_or_else(|| GenError::Invalid("input must be PNG".into()))?;
        if size.0 == 0 || size.1 == 0 || size.0 > 4096 || size.1 > 4096 || crate::png_size(&request.mask) != Some(size) {
            return Err(GenError::Invalid("image and mask must have matching bounded dimensions".into()));
        }
        let prompt = if request.prompt.is_empty() { "Remove the masked content and naturally continue the surroundings." } else { &request.prompt };
        if self.protocol == OPENAI {
            let mask = alpha_mask(&request.mask)?;
            // Fixed header token; select a token absent from binary parts and user input.
            let mut boundary = "photocraft-cloud-boundary".to_owned();
            while request.image.windows(boundary.len()).any(|w| w == boundary.as_bytes())
                || mask.windows(boundary.len()).any(|w| w == boundary.as_bytes())
                || prompt.contains(&boundary)
                || self.model.contains(&boundary)
            {
                boundary.push('x');
                if boundary.len() > 128 {
                    return Err(GenError::Invalid("cannot encode multipart input".into()));
                }
            }
            let mut body = Vec::new();
            for (name, value) in [
                ("model", self.model.as_str()),
                ("prompt", prompt),
                ("n", "1"),
                (
                    "size",
                    if size.0 > size.1 {
                        "1536x1024"
                    } else if size.1 > size.0 {
                        "1024x1536"
                    } else {
                        "1024x1024"
                    },
                ),
            ] {
                body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
            }
            for (name, bytes) in [("image", request.image.as_slice()), ("mask", mask.as_slice())] {
                body.extend_from_slice(
                    format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{name}.png\"\r\nContent-Type: image/png\r\n\r\n")
                        .as_bytes(),
                );
                body.extend_from_slice(bytes);
                body.extend_from_slice(b"\r\n");
            }
            body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
            let value = self.send(self.endpoint("/images/edits"), format!("multipart/form-data; boundary={boundary}"), body)?;
            let encoded = value
                .get("data")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|v| v.get("b64_json"))
                .and_then(Value::as_str)
                .ok_or(GenError::BadResponse("expected data[0].b64_json; URL-only results are unsupported"))?;
            self.decoded(encoded, "image/png")
        } else {
            let mut url = self.base.clone();
            {
                let mut segments = url.path_segments_mut().map_err(|_| GenError::Invalid("invalid API URL".into()))?;
                segments.pop_if_empty();
                segments.push("models");
                segments.push(&format!("{}:generateContent", self.model));
            }
            let body = json!({"contents": [{"parts": [
                {"text": format!("Edit the FIRST image. The SECOND image is a mask: white means change, black means preserve. Keep the exact composition and dimensions. {prompt}")},
                {"inlineData": {"mimeType": "image/png", "data": STANDARD.encode(&request.image)}},
                {"inlineData": {"mimeType": "image/png", "data": STANDARD.encode(&request.mask)}}
            ]}], "generationConfig": {"responseModalities": ["TEXT", "IMAGE"]}});
            let value = self.send(
                url.into(),
                "application/json".into(),
                serde_json::to_vec(&body).map_err(|_| GenError::Invalid("cannot encode image request".into()))?,
            )?;
            if let Some(candidates) = value.get("candidates").and_then(Value::as_array) {
                for candidate in candidates {
                    if let Some(parts) = candidate.pointer("/content/parts").and_then(Value::as_array) {
                        for part in parts {
                            if let Some(data) = part.get("inlineData").or_else(|| part.get("inline_data"))
                                && let Some(encoded) = data.get("data").and_then(Value::as_str)
                            {
                                let mime = data.get("mimeType").or_else(|| data.get("mime_type")).and_then(Value::as_str).unwrap_or("image/png");
                                return self.decoded(encoded, mime);
                            }
                        }
                    }
                }
            }
            Err(GenError::BadResponse("provider returned no image; check model image-edit capability"))
        }
    }
}

/// Engine mask: white = fill. Images API mask: transparent = fill, opaque = preserve.
fn alpha_mask(input: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = png::Decoder::new(Cursor::new(input));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|_| GenError::Invalid("invalid mask PNG".into()))?;
    let length = reader.output_buffer_size().ok_or_else(|| GenError::Invalid("mask is too large".into()))?;
    if length > 64 * 1024 * 1024 {
        return Err(GenError::Invalid("mask is too large".into()));
    }
    let mut data = vec![0; length];
    let info = reader.next_frame(&mut data).map_err(|_| GenError::Invalid("invalid mask pixels".into()))?;
    let channels = info.color_type.samples();
    let pixels = data.get(..info.buffer_size()).ok_or(GenError::BadResponse("invalid mask length"))?;
    let mut rgba = Vec::with_capacity(pixels.len().saturating_div(channels).saturating_mul(4));
    for pixel in pixels.chunks_exact(channels) {
        let white = pixel.first().copied().unwrap_or(0);
        rgba.extend_from_slice(&[255, 255, 255, 255 - white]);
    }
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut output, info.width, info.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|_| GenError::Invalid("cannot encode mask".into()))?;
        writer.write_image_data(&rgba).map_err(|_| GenError::Invalid("cannot encode mask pixels".into()))?;
    }
    if output.len() >= 4 * 1024 * 1024 {
        return Err(GenError::Invalid("encoded mask exceeds 4 MiB".into()));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HttpResponse;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct Fake {
        sent: Arc<Mutex<Vec<HttpRequest>>>,
        status: u16,
        response: Vec<u8>,
    }
    impl HttpTransport for Fake {
        fn post(&self, request: HttpRequest) -> Result<HttpResponse> {
            self.sent.lock().unwrap().push(request);
            Ok(HttpResponse { status: self.status, body: self.response.clone() })
        }
    }
    fn png(values: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut output, values.len() as u32, 1);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(values).unwrap();
        }
        output
    }
    fn fake(response: Value) -> Fake {
        Fake { sent: Default::default(), status: 200, response: serde_json::to_vec(&response).unwrap() }
    }
    fn key() -> ApiKey {
        ApiKey::new("test-only-key").unwrap()
    }
    fn request() -> EditRequest {
        EditRequest { image: png(&[40, 70, 90]), mask: png(&[0, 128, 255]), prompt: String::new(), size: Default::default() }
    }
    #[test]
    fn mask_preserves_black_and_feathering_while_making_white_transparent() {
        let mask = alpha_mask(&png(&[0, 128, 255])).unwrap();
        let mut decoder = png::Decoder::new(Cursor::new(mask)).read_info().unwrap();
        let mut pixels = vec![0; decoder.output_buffer_size().unwrap()];
        let frame = decoder.next_frame(&mut pixels).unwrap();
        assert_eq!(frame.color_type, png::ColorType::Rgba);
        assert_eq!(pixels, [255, 255, 255, 255, 255, 255, 255, 127, 255, 255, 255, 0]);
    }
    #[test]
    fn images_protocol_sends_multipart_and_decodes_inline_result() {
        let transport = fake(json!({"data": [{"b64_json": STANDARD.encode(b"image-result")}]}));
        let provider = CloudProvider::new(OPENAI, "https://images.example/v1/", "custom-model", key(), transport.clone()).unwrap();
        assert_eq!(provider.edit(&request()).unwrap().bytes, b"image-result");
        let sent = transport.sent.lock().unwrap();
        assert_eq!(sent[0].url, "https://images.example/v1/images/edits");
        assert!(sent[0].headers.iter().any(|(name, value)| *name == "authorization" && value == "Bearer test-only-key"));
        let body = String::from_utf8_lossy(&sent[0].body);
        assert!(body.contains("name=\"mask\"; filename=\"mask.png\""));
        assert!(body.contains("custom-model"));
        assert!(!body.contains("test-only-key"));
    }
    #[test]
    fn gemini_protocol_sends_image_and_white_mask_separately() {
        let transport = fake(json!({"candidates": [{"content": {"parts": [{"inlineData": {"data": STANDARD.encode(b"png"), "mimeType": "image/png"}}]}}]}));
        let provider = CloudProvider::new(GEMINI, "https://images.example/v1beta", "image-model", key(), transport.clone()).unwrap();
        assert_eq!(provider.edit(&request()).unwrap().bytes, b"png");
        let sent = transport.sent.lock().unwrap();
        assert_eq!(sent[0].url, "https://images.example/v1beta/models/image-model:generateContent");
        let body: Value = serde_json::from_slice(&sent[0].body).unwrap();
        assert_eq!(body["contents"][0]["parts"][2]["inlineData"]["data"], STANDARD.encode(request().mask));
    }
    #[test]
    fn invalid_endpoint_cannot_send_and_errors_never_echo_provider_secrets() {
        for endpoint in ["http://example.com", "https://user:secret@example.com/v1", "https://example.com/v1?key=secret", "https://example.com/#secret"] {
            assert!(CloudProvider::new(OPENAI, endpoint, "model", key(), fake(json!({}))).is_err());
        }
        let mut transport = fake(json!({"error": {"message": "test-only-key private response"}}));
        transport.status = 500;
        let provider = CloudProvider::new(OPENAI, "https://example.com/v1", "model", key(), transport).unwrap();
        let error = provider.edit(&request()).unwrap_err().to_string();
        assert!(!error.contains("test-only-key"));
        assert!(!error.contains("private response"));
    }
    #[test]
    fn url_only_or_empty_success_and_mismatched_masks_are_rejected() {
        let transport = fake(json!({"data": [{"url": "https://example.com/result.png"}]}));
        let provider = CloudProvider::new(OPENAI, "https://example.com/v1", "model", key(), transport.clone()).unwrap();
        assert!(provider.edit(&request()).is_err());
        let mut invalid = request();
        invalid.mask = png(&[255]);
        assert!(provider.edit(&invalid).is_err());
        assert_eq!(transport.sent.lock().unwrap().len(), 1);
    }
}
