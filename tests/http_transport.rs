use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use rmcp::ServerHandler;
use rmcp::model::{ServerCapabilities, ServerInfo};
use serde_json::{Value, json};
use tower::ServiceExt;

#[derive(Clone)]
struct TestService;

impl ServerHandler for TestService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
    }
}

fn request(message: Value, session: Option<&str>) -> Request<Body> {
    let mut request = Request::post("/mcp")
        .header("host", "localhost")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-03-26");
    if let Some(session) = session {
        request = request.header("mcp-session-id", session);
    }
    request.body(Body::from(message.to_string())).unwrap()
}

async fn rpc_response(response: axum::response::Response) -> Value {
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    let body = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        response.into_body().collect(),
    )
    .await
    .expect("response stream should finish")
    .unwrap()
    .to_bytes();
    let body = std::str::from_utf8(&body).unwrap();
    let data = body
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .find(|data| !data.is_empty())
        .unwrap();
    serde_json::from_str(data).expect("SSE response must contain valid JSON-RPC")
}

#[tokio::test]
async fn legacy_initialization_and_tool_discovery_remain_supported() {
    let app = Router::new().nest_service(
        "/mcp",
        proton_mail_mcp::http::mcp_service(TestService, Some("https://mail.example.com")).unwrap(),
    );
    let response = app
        .clone()
        .oneshot(request(
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {
                    "protocolVersion": "2025-03-26", "capabilities": {},
                    "clientInfo": {"name": "transport-test", "version": "1"}
                }
            }),
            None,
        ))
        .await
        .unwrap();
    let session = response.headers()["mcp-session-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let initialized = rpc_response(response).await;
    assert_eq!(initialized["id"], 1);
    assert_eq!(initialized["result"]["protocolVersion"], "2025-03-26");
    let response = app
        .clone()
        .oneshot(request(
            json!({
                "jsonrpc": "2.0", "method": "notifications/initialized"
            }),
            Some(&session),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let response = app
        .oneshot(request(
            json!({
                "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}
            }),
            Some(&session),
        ))
        .await
        .unwrap();
    let tools = rpc_response(response).await;
    assert_eq!(tools["id"], 2);
    assert_eq!(tools["result"]["tools"], json!([]));
}

fn discovery_request(method: &str, id: u32) -> Request<Body> {
    Request::post("/mcp")
        .header("host", "mail.example.com")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2026-07-28")
        .header("mcp-method", method)
        .body(Body::from(
            json!({
                "jsonrpc": "2.0", "id": id, "method": method,
                "params": {"_meta": {
                    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                    "io.modelcontextprotocol/clientInfo": {"name": "ChatGPT", "version": "1.0.0"},
                    "io.modelcontextprotocol/clientCapabilities": {}
                }}
            })
            .to_string(),
        ))
        .unwrap()
}

#[tokio::test]
async fn chatgpt_discovers_server_and_tools_without_initializing() {
    let app = Router::new().nest_service(
        "/mcp",
        proton_mail_mcp::http::mcp_service(TestService, Some("https://mail.example.com")).unwrap(),
    );
    let response = app
        .clone()
        .oneshot(discovery_request("server/discover", 1))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get("mcp-session-id").is_none());
    let discovery = rpc_response(response).await;
    assert_eq!(discovery["result"]["resultType"], "complete");
    assert!(
        discovery["result"]["supportedVersions"]
            .as_array()
            .unwrap()
            .contains(&json!("2026-07-28"))
    );
    let response = app
        .oneshot(discovery_request("tools/list", 2))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get("mcp-session-id").is_none());
    let tools = rpc_response(response).await;
    assert_eq!(tools["result"]["resultType"], "complete");
    assert_eq!(tools["result"]["tools"], json!([]));
}

#[tokio::test]
async fn discovery_rejects_an_unconfigured_host() {
    let app = Router::new().nest_service(
        "/mcp",
        proton_mail_mcp::http::mcp_service(TestService, Some("https://mail.example.com")).unwrap(),
    );
    let mut request = discovery_request("server/discover", 1);
    request
        .headers_mut()
        .insert("host", "untrusted.example.com".parse().unwrap());
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
