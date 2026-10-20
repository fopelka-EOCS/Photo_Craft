//! End-to-end NPU subject selection adapter example for PhotoCraft.
//! Run: `cargo run -p photocraft-engine --example adapter_npu_subject`
//!
//! Demonstrates how an AI subject selection adapter (e.g., powered by Qualcomm Hexagon NPU
//! via DirectML/QNN or Windows AI ImageForegroundExtractor) integrates with PhotoCraft's
//! `select.subject` command, document selection writeback, and single-step undo.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::sync::Arc;

use photocraft_engine::{EngineError, Session};
use photocraft_plugins::adapter::{
    self, Adapter, Context, Descriptor, Device, DeviceKind, Execution, Image, Limits, Operation, Output, OutputKind, Request, Response,
};
use serde_json::json;

const ADAPTER_ID: &str = "org.photocraft.adapter.snapdragon-npu-subject";
const DEVICE_ID: &str = "npu.snapdragon.hexagon";

/// Example adapter simulating an on-device AI subject segmentation model (like Affinity Photo's
/// ML selection pipeline) running on Qualcomm Snapdragon X Elite Hexagon NPU.
struct NpuSubjectSelectionAdapter;

impl Adapter for NpuSubjectSelectionAdapter {
    fn descriptor(&self) -> Descriptor {
        Descriptor {
            api_version: adapter::API_VERSION,
            id: ADAPTER_ID.into(),
            name: "Snapdragon Hexagon NPU Subject Selection".into(),
            version: "1.0.0".into(),
            operations: vec![Operation { id: "select-subject".into(), output_kind: OutputKind::Mask, color_modes: vec!["rgb".into()] }],
            devices: vec![Device {
                id: DEVICE_ID.into(),
                kind: DeviceKind::Npu,
                backend: "directml-qnn".into(),
                name: "Qualcomm Snapdragon X Elite Hexagon NPU".into(),
                operations: vec!["select-subject".into()],
            }],
            resource_formats: vec!["onnx-qdq".into(), "windows-ai".into()],
            params_schema: json!({}),
        }
    }

    fn run(&self, request: &Request, ctx: &dyn Context) -> adapter::Result<Response> {
        let limits = Limits::default();
        request.image.validate(&limits)?;
        if ctx.cancelled() {
            return Err(adapter::Error::Canceled);
        }

        let (w, h) = (request.image.width, request.image.height);
        let count = limits.check_image_size(w, h, 1)?;
        let mut mask_data = Vec::new();
        mask_data.try_reserve_exact(count).map_err(|e| adapter::Error::Limit(e.to_string()))?;

        // In a production worker, this buffer is passed to the NPU via DirectML or Windows AI APIs.
        // For this clean standalone example, we produce a centered subject foreground probability mask.
        let cx = w as f32 * 0.5;
        let cy = h as f32 * 0.5;
        let rx = w as f32 * 0.3;
        let ry = h as f32 * 0.3;

        for y in 0..h {
            if y % 32 == 0 && ctx.cancelled() {
                return Err(adapter::Error::Canceled);
            }
            let dy = (y as f32 - cy) / ry;
            for x in 0..w {
                let dx = (x as f32 - cx) / rx;
                let dist_sq = dx * dx + dy * dy;
                let prob = if dist_sq <= 1.0 { 1.0 } else { 0.0 };
                mask_data.push(prob);
            }
        }

        Ok(Response {
            output: Output {
                kind: OutputKind::Mask,
                image: Image {
                    x: request.image.x,
                    y: request.image.y,
                    width: w,
                    height: h,
                    channels: 1,
                    color_mode: "gray".into(),
                    source_depth: 32,
                    alpha: false,
                    icc_profile: Vec::new(),
                    data: mask_data,
                },
            },
            execution: Execution { devices: vec![DEVICE_ID.into()], backend: "directml-qnn".into(), fallback_reason: None },
        })
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 200, "height": 160, "depth": 8}))?;

    // 1. Register the trusted NPU adapter in the session
    session.register_adapter(Arc::new(NpuSubjectSelectionAdapter))?;
    println!("Registered NPU adapter: {}", session.execute("plugin.adapter.list", json!({}))?);

    // 2. Dispatch Select Subject: automatically routes to the registered NPU adapter
    let res = session.execute("select.subject", json!({}))?;
    println!("Select Subject result (via NPU adapter): {res}");

    let doc = session.active().ok_or(EngineError::NoDocument)?;
    if doc.doc.selection.is_none() {
        return Err(EngineError::Other("expected selection to be created by NPU adapter".into()).into());
    }
    println!("Document now has active selection created by NPU adapter.");

    // 3. Single-step undo clears the selection
    session.execute("edit.undo", json!({}))?;
    if session.active().ok_or(EngineError::NoDocument)?.doc.selection.is_some() {
        return Err(EngineError::Other("undo did not clear the selection".into()).into());
    }
    println!("Undo successfully cleared the selection.");

    // 4. Clean unregistration
    if !session.unregister_adapter(ADAPTER_ID) {
        return Err(EngineError::Other("NPU adapter registration was missing".into()).into());
    }
    println!("Successfully unregistered adapter.");

    Ok(())
}
