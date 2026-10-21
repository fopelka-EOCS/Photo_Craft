//! An agent can check a protected region through the existing MCP command tools.
use photocraft_automation::PhotocraftMcp;
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig};
use rmcp::service::RunningService;
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{Value, json};

#[derive(Clone, Default)]
struct Client;
impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::default()
    }
}

/// Keep the transport in memory: no files or external app are needed for the check.
async fn connect() -> RunningService<RoleClient, Client> {
    let (server_io, client_io) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        if let Ok(server) = PhotocraftMcp::headless().serve(server_io).await {
            let _ = server.waiting().await;
        }
    });
    Client.serve(client_io).await.unwrap()
}

/// Exercise normal tool dispatch, including its command authorization and result envelope.
async fn call(client: &RunningService<RoleClient, Client>, name: &str, args: Value) -> CallToolResult {
    let request = CallToolRequestParams::new(name.to_owned()).with_arguments(args.as_object().unwrap().clone());
    client.call_tool(request).await.unwrap()
}

/// Tool errors must stay visible rather than being interpreted as an unchanged image.
fn result(reply: &CallToolResult) -> Value {
    let text = reply.content.iter().filter_map(|part| part.as_text()).map(|part| part.text.as_str()).collect::<Vec<_>>().join("\n");
    assert_ne!(reply.is_error, Some(true), "{text}");
    serde_json::from_str(&text).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn snapshot_comparison_is_discoverable_and_read_only_through_mcp() {
    let client = connect().await;
    result(&call(&client, "doc_new", json!({"width":32,"height":16,"background":"white"})).await);
    let listed = result(&call(&client, "command_list", json!({"filter":"document.comparePixels"})).await);
    assert!(listed.to_string().contains("document.comparePixels"));
    let snapshot = result(&call(&client, "command_run", json!({"id":"history.newSnapshot","params":{"from":"fullDocument"}})).await)["snapshot"].clone();
    result(&call(&client, "command_run", json!({"id":"select.rect","params":{"x":2,"y":3,"width":4,"height":5,"feather":0}})).await);
    result(&call(&client, "command_run", json!({"id":"edit.fill","params":{"color":"#000000"}})).await);
    let before = result(&call(&client, "doc_inspect", json!({})).await);
    let params = json!({"snapshot":snapshot,"regions":[{"x":2,"y":3,"width":4,"height":5},{"x":15,"y":3,"width":4,"height":5}]});
    let comparison = result(&call(&client, "command_run", json!({"id":"document.comparePixels","params":params})).await);
    assert_eq!(comparison["space"], "composited-rgba-f32");
    assert_eq!(comparison["regions"][0]["comparedPixels"], 20);
    assert_eq!(comparison["regions"][0]["changedPixels"], 20);
    assert_eq!(comparison["regions"][0]["maxChannelDifference"], 1.0);
    assert_eq!(comparison["regions"][1]["comparedPixels"], 20);
    assert_eq!(comparison["regions"][1]["changedPixels"], 0);
    assert_eq!(comparison["regions"][1]["maxChannelDifference"], 0.0);
    assert_eq!(result(&call(&client, "doc_inspect", json!({})).await), before);

    let rejected = call(
        &client,
        "command_run",
        json!({"id":"document.comparePixels","params":{"snapshot":snapshot,"regions":[{"x":0,"y":0,"width":u32::MAX,"height":1}]}}),
    )
    .await;
    assert_eq!(rejected.is_error, Some(true));
    assert_eq!(result(&call(&client, "doc_inspect", json!({})).await), before);
    client.cancel().await.unwrap();
}
