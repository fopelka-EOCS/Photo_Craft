//! Direct loopback-only RoboDhan API client. No Python process in the editor.
use serde_json::{Value, json};
use std::{
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
fn request(path: &str, body: Option<Value>) -> Result<Value, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().proxy(None).max_redirects(0).timeout_global(Some(Duration::from_secs(600))).build().into();
    let url = format!("http://127.0.0.1:1234/v1{path}");
    let response =
        if let Some(body) = body { agent.post(&url).header("Content-Type", "application/json").send(body.to_string()) } else { agent.get(&url).call() };
    let mut response = response.map_err(|e| format!("RoboDhan: {e}. Pastikan API lokal port 1234 aktif."))?;
    let mut bytes = Vec::new();
    response.body_mut().as_reader().take(12 * 1024 * 1024 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > 12 * 1024 * 1024 {
        return Err("Respons RoboDhan terlalu besar".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("Respons RoboDhan tidak valid: {e}"))
}
pub fn call(req: Value, cancel: Arc<AtomicBool>) -> Result<Value, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Dibatalkan".into());
    }
    let op = req["operation"].as_str().unwrap_or("");
    if op == "models" {
        let v = request("/models", None)?;
        return Ok(
            json!({"models":v["data"].as_array().into_iter().flatten().filter(|m|m["id"].as_str().is_some_and(|id| !id.starts_with("mflux-community/"))).filter_map(|m|m["id"].as_str()).collect::<Vec<_>>()}),
        );
    }
    let prompt = req["prompt"].as_str().filter(|s| !s.trim().is_empty() && s.chars().count() <= 4000).ok_or("Prompt harus berisi 1–4000 karakter")?;
    let result = if op == "generate" {
        let size = req["size"].as_u64().filter(|n| [512, 768].contains(n)).ok_or("Ukuran tidak didukung")?;
        let v = request(
            "/images/generations",
            Some(
                json!({"model":"mflux-community/flux2-klein-4b-mflux-q4","prompt":prompt,"size":format!("{size}x{size}"),"seed":req["seed"],"n":1,"response_format":"b64_json"}),
            ),
        )?;
        let png = v["data"][0]["b64_json"].as_str().ok_or("RoboDhan tidak mengembalikan gambar")?;
        json!({"pngBase64":png})
    } else {
        let system = if op == "svg" {
            "You create editable vector illustrations. Return ONLY one complete SVG document, no markdown, no reasoning. Use viewBox 0 0 768 768, width and height 768. Allowed elements: svg g defs path rect circle ellipse line polyline polygon text tspan title desc linearGradient radialGradient stop clipPath. No scripts, images, use, external resources, metadata, data attributes or style elements. Use inline presentation attributes and simple geometry, under 12000 characters. For edits preserve the original composition unless asked otherwise."
        } else if op == "enhance" {
            "Rewrite the image idea as a concise English image-generation prompt. Preserve intent. Return only the prompt, under 180 words, no markdown or reasoning."
        } else {
            return Err("Operasi tidak dikenal".into());
        };
        let source = req["svg"].as_str().unwrap_or("");
        if source.len() > 512 * 1024 {
            return Err("SVG sumber terlalu besar".into());
        }
        let content =
            if source.is_empty() { prompt.to_owned() } else { format!("Edit this SVG according to the instruction.\nInstruction: {prompt}\nSVG:\n{source}") };
        let v = request(
            "/chat/completions",
            Some(json!({"stream":false,"max_tokens":4096,"messages":[{"role":"system","content":system},{"role":"user","content":content}]})),
        )?;
        let text = v["choices"][0]["message"]["content"].as_str().filter(|s| !s.trim().is_empty()).ok_or("RoboDhan tidak mengembalikan hasil")?;
        if op == "svg" {
            let start = text.find("<svg").ok_or("AI tidak mengembalikan SVG. Coba instruksi lebih sederhana.")?;
            let end = text.rfind("</svg>").map(|i| i + 6).ok_or("SVG dari AI terpotong. Coba instruksi lebih sederhana.")?;
            let svg = text.get(start..end).ok_or("SVG tidak valid")?;
            json!({"svg":svg})
        } else {
            json!({"prompt":text.trim()})
        }
    };
    if cancel.load(Ordering::Relaxed) { Err("Hasil diabaikan".into()) } else { Ok(result) }
}
