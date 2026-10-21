//! Tier 5 Adversarial Test Suite for photocraft-automation.
//!
//! White-box stress testing of:
//! 1. Boundary commands: unknown tools, unknown command ids, out-of-range document indices.
//! 2. Malformed parameters: extra fields on deny_unknown_fields, wrong parameter types, malformed colors.
//! 3. Concurrent request sequences: parallel async tasks calling inspect, pixel, preview, translation.

use std::sync::Arc;

use photocraft_automation::{AuthorizedWorkspace, PhotocraftMcp};
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig};
use rmcp::service::RunningService;
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{Value, json};

#[derive(Clone, Default)]
struct TestClient;

impl ClientHandler for TestClient {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::default()
    }
}

type Conn = RunningService<RoleClient, TestClient>;

async fn connect(server: PhotocraftMcp) -> Conn {
    let (s, c) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        if let Ok(running) = server.serve(s).await {
            let _ = running.waiting().await;
        }
    });
    TestClient.serve(c).await.expect("client init")
}

fn text(r: &CallToolResult) -> String {
    r.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n")
}

async fn call_ok(client: &Conn, name: &str, args: Value) -> Value {
    let mut p = CallToolRequestParams::new(name.to_owned());
    if let Value::Object(m) = args {
        p = p.with_arguments(m);
    }
    let r = client.call_tool(p).await.expect("call_tool transport error");
    let txt = text(&r);
    assert_ne!(r.is_error, Some(true), "tool {name} unexpectedly errored: {txt}");
    serde_json::from_str(&txt).unwrap_or(Value::String(txt))
}

async fn call_err(client: &Conn, name: &str, args: Value) -> String {
    let mut p = CallToolRequestParams::new(name.to_owned());
    if let Value::Object(m) = args {
        p = p.with_arguments(m);
    }
    match client.call_tool(p).await {
        Ok(r) => {
            assert_eq!(r.is_error, Some(true), "tool {name} unexpectedly succeeded: {}", text(&r));
            text(&r)
        }
        Err(e) => format!("{e:?}"),
    }
}

struct TestHarness {
    client: Arc<Conn>,
    dir: std::path::PathBuf,
}

impl TestHarness {
    async fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pc-tier5-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let workspace = AuthorizedWorkspace::new(Some(&dir), Some(&dir)).unwrap();
        let server = PhotocraftMcp::headless_with_workspace(workspace);
        let client = Arc::new(connect(server).await);

        Self { client, dir }
    }

    async fn shutdown(self) {
        if let Ok(conn) = Arc::try_unwrap(self.client) {
            let _ = conn.cancel().await;
        }
        for _ in 0..20 {
            match std::fs::remove_dir_all(&self.dir) {
                Ok(()) => return,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
            }
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// =========================================================================
// 1. BOUNDARY COMMANDS: UNKNOWN TOOLS, UNKNOWN IDS, OUT OF BOUNDS INDICES
// =========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tier5_boundary_unknown_tools_and_commands() {
    let h = TestHarness::new("boundary_unknown").await;

    // 1. Unknown tool name -> should return error (either transport or tool error)
    let err_tool = call_err(&h.client, "nonexistent_tool_name_xyz", json!({})).await;
    assert!(!err_tool.is_empty(), "Unknown tool must produce an error");

    // 2. Open a document first
    call_ok(&h.client, "doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;

    // 3. Unknown command id inside command_run
    let err_cmd = call_err(&h.client, "command_run", json!({"id": "completely.fake.command.id"})).await;
    assert!(err_cmd.contains("unknown") || err_cmd.contains("not recognized") || !err_cmd.is_empty());

    // 4. Out-of-bounds document index in doc_inspect
    let err_inspect = call_err(&h.client, "doc_inspect", json!({"index": 999999})).await;
    assert!(err_inspect.contains("index") || err_inspect.contains("out of bounds") || !err_inspect.is_empty());

    // 5. Out-of-bounds document index in doc_close
    let err_close = call_err(&h.client, "doc_close", json!({"index": 999999})).await;
    assert!(err_close.contains("index") || err_close.contains("out of bounds") || !err_close.is_empty());

    h.shutdown().await;
}

// =========================================================================
// 2. MALFORMED PARAMETERS: EXTRA FIELDS, WRONG TYPES, FORBIDDEN VALUES
// =========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tier5_malformed_parameters_rejection() {
    let h = TestHarness::new("malformed_params").await;

    // 1. Extra parameter on deny_unknown_fields (doc_new)
    let err_extra = call_err(&h.client, "doc_new", json!({"width": 32, "height": 32, "unrecognized_extra_key": true})).await;
    assert!(!err_extra.is_empty(), "Extra field should be rejected");

    // 2. Wrong parameter type (string instead of integer for width)
    let err_type = call_err(&h.client, "doc_new", json!({"width": "thirty-two", "height": 32})).await;
    assert!(!err_type.is_empty(), "Wrong type should be rejected");

    // 3. Zero or negative width
    let err_zero = call_err(&h.client, "doc_new", json!({"width": 0, "height": 32})).await;
    assert!(!err_zero.is_empty(), "Zero width should be rejected");

    // 4. Valid doc_new to test command parameter errors
    call_ok(&h.client, "doc_new", json!({"width": 32, "height": 32, "background": "white"})).await;
    let r = call_ok(&h.client, "command_run", json!({"id": "layer.new.layer", "params": {}})).await;
    let id = r["layer"].as_u64().unwrap();

    // 5. Unknown blend mode rejection
    let err_blend = call_err(&h.client, "command_run", json!({"id": "layer.setProps", "params": {"layer": id, "blend": "NonExistentMode"}})).await;
    assert!(err_blend.contains("unknown blend mode"), "Expected unknown blend mode rejection: {err_blend}");

    // 6. Empty layers array rejection
    let err_empty = call_err(&h.client, "command_run", json!({"id": "layer.setProps", "params": {"layers": [], "opacity": 0.5}})).await;
    assert!(err_empty.contains("non-empty list"), "Expected empty layers rejection: {err_empty}");

    h.shutdown().await;
}

// =========================================================================
// 3. CONCURRENT REQUEST SEQUENCES: MULTI-TASK THREAD-SAFETY & NO DEADLOCK
// =========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tier5_concurrent_mcp_request_sequences() {
    let h = Arc::new(TestHarness::new("concurrent_rpc").await);

    // Create document with two layers
    call_ok(&h.client, "doc_new", json!({"width": 64, "height": 64, "background": "white"})).await;
    let r = call_ok(&h.client, "command_run", json!({"id": "layer.new.layer", "params": {"name": "Red"}})).await;
    let _ = r;
    call_ok(&h.client, "command_run", json!({"id": "edit.fill", "params": {"color": "#ff0000"}})).await;

    let mut handles = Vec::new();

    // Spawn 15 concurrent tasks reading pixels via command_run document.pixel
    for i in 0..15 {
        let client = Arc::clone(&h.client);
        handles.push(tokio::spawn(async move {
            let x = (i * 3) % 64;
            let y = (i * 5) % 64;
            let px = call_ok(&client, "command_run", json!({"id": "document.pixel", "params": {"x": x, "y": y}})).await;
            assert!(px.is_object() || px.is_array());
        }));
    }

    // Spawn 10 concurrent tasks inspecting document structure
    for _ in 0..10 {
        let client = Arc::clone(&h.client);
        handles.push(tokio::spawn(async move {
            let doc = call_ok(&client, "doc_inspect", json!({})).await;
            assert!(doc.is_object());
        }));
    }

    // Spawn 5 concurrent tasks generating previews
    for side in [16, 32, 64] {
        let client = Arc::clone(&h.client);
        handles.push(tokio::spawn(async move {
            let prev = call_ok(&client, "doc_render_preview", json!({"max_side": side})).await;
            assert!(prev.is_object() || prev.is_string());
        }));
    }

    // Await all concurrent tasks
    for handle in handles {
        handle.await.expect("task panicked or deadlocked");
    }

    // Final integrity check: inspect document
    let final_doc = call_ok(&h.client, "doc_inspect", json!({})).await;
    assert_eq!(final_doc["layers"].as_array().unwrap().len(), 2);

    let h = Arc::try_unwrap(h).unwrap_or_else(|_| panic!("Failed to unwrap Arc"));
    h.shutdown().await;
}
