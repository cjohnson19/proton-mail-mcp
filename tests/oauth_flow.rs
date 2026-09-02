use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::middleware;
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use http_body_util::BodyExt;
use sha2::{Digest, Sha256};
use tower::util::ServiceExt;

use proton_mail_mcp::auth::{self, AuthConfig, AuthState};

const PUBLIC_URL: &str = "https://mcp.example.com";
const PASSPHRASE: &str = "correct horse battery staple";
const CALLBACK: &str = "https://claude.ai/api/mcp/auth_callback";
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

fn test_app() -> Router {
    let config = AuthConfig {
        public_url: PUBLIC_URL.into(),
        passphrase: PASSPHRASE.into(),
        signing_key: b"an-hs256-test-key-of-32-plus-bytes!!".to_vec(),
        extra_redirect_uris: vec![],
    };
    let state = Arc::new(AuthState::new(config));
    Router::new()
        .route("/mcp", get(|| async { "protected" }))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_bearer,
        ))
        .merge(auth::router(state))
}

fn challenge() -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()))
}

fn authorize_uri(redirect_uri: &str, resource: Option<&str>) -> String {
    let mut params = vec![
        ("response_type", "code"),
        ("client_id", "test-client"),
        ("redirect_uri", redirect_uri),
        ("state", "opaque-state-123"),
        ("code_challenge_method", "S256"),
    ];
    let ch = challenge();
    params.push(("code_challenge", &ch));
    if let Some(r) = resource {
        params.push(("resource", r));
    }
    format!(
        "/authorize?{}",
        serde_urlencoded::to_string(params).unwrap()
    )
}

async fn body_string(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn submit_passphrase(app: &Router, uri: &str, passphrase: &str) -> axum::response::Response {
    let body = serde_urlencoded::to_string([("passphrase", passphrase)]).unwrap();
    app.clone()
        .oneshot(
            Request::post(uri)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
}

fn location_params(response: &axum::response::Response) -> HashMap<String, String> {
    let location = response
        .headers()
        .get(header::LOCATION)
        .expect("expected Location header")
        .to_str()
        .unwrap();
    let (_, query) = location.split_once('?').expect("expected query string");
    serde_urlencoded::from_str(query).unwrap()
}

async fn obtain_code(app: &Router) -> String {
    let response = submit_passphrase(app, &authorize_uri(CALLBACK, None), PASSPHRASE).await;
    assert_eq!(response.status(), StatusCode::FOUND);
    let location = response.headers()[header::LOCATION].to_str().unwrap();
    assert!(location.starts_with(CALLBACK));
    let params = location_params(&response);
    assert_eq!(params["state"], "opaque-state-123");
    params["code"].clone()
}

async fn exchange(app: &Router, fields: &[(&str, &str)]) -> (StatusCode, serde_json::Value) {
    let body = serde_urlencoded::to_string(fields).unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::post("/token")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let json: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
    (status, json)
}

async fn get_with_bearer(app: &Router, token: Option<&str>) -> axum::response::Response {
    let mut request = Request::get("/mcp");
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    app.clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn metadata_endpoints() {
    let app = test_app();
    for path in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp",
    ] {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
        assert_eq!(json["resource"], format!("{PUBLIC_URL}/mcp"));
        assert_eq!(json["authorization_servers"][0], PUBLIC_URL);
    }
    let response = app
        .clone()
        .oneshot(
            Request::get("/.well-known/oauth-authorization-server")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
    assert_eq!(json["issuer"], PUBLIC_URL);
    assert_eq!(
        json["authorization_endpoint"],
        format!("{PUBLIC_URL}/authorize")
    );
    assert_eq!(json["code_challenge_methods_supported"][0], "S256");
}

#[tokio::test]
async fn register_enforces_allowlist() {
    let app = test_app();
    let ok = app
        .clone()
        .oneshot(
            Request::post("/register")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "redirect_uris": [CALLBACK], "client_name": "Claude" })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::CREATED);
    let json: serde_json::Value = serde_json::from_str(&body_string(ok).await).unwrap();
    assert!(json["client_id"].as_str().unwrap().len() >= 32);

    let bad = app
        .clone()
        .oneshot(
            Request::post("/register")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "redirect_uris": ["https://evil.example.com/cb"] })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn full_flow_authorize_token_bearer_refresh() {
    let app = test_app();

    let page = app
        .clone()
        .oneshot(
            Request::get(authorize_uri(CALLBACK, Some(&format!("{PUBLIC_URL}/mcp"))))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    assert!(body_string(page).await.contains("name=\"passphrase\""));

    let code = obtain_code(&app).await;
    let (status, tokens) = exchange(
        &app,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", "test-client"),
            ("redirect_uri", CALLBACK),
            ("code_verifier", VERIFIER),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tokens["token_type"], "Bearer");
    let access = tokens["access_token"].as_str().unwrap();
    let refresh = tokens["refresh_token"].as_str().unwrap();

    let protected = get_with_bearer(&app, Some(access)).await;
    assert_eq!(protected.status(), StatusCode::OK);
    assert_eq!(body_string(protected).await, "protected");

    let (status, refreshed) = exchange(
        &app,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", "test-client"),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let new_access = refreshed["access_token"].as_str().unwrap();
    assert_eq!(
        get_with_bearer(&app, Some(new_access)).await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn code_is_single_use() {
    let app = test_app();
    let code = obtain_code(&app).await;
    let fields = [
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("client_id", "test-client"),
        ("code_verifier", VERIFIER),
    ];
    let (first, _) = exchange(&app, &fields).await;
    assert_eq!(first, StatusCode::OK);
    let (second, error) = exchange(&app, &fields).await;
    assert_eq!(second, StatusCode::BAD_REQUEST);
    assert_eq!(error["error"], "invalid_grant");
}

#[tokio::test]
async fn token_rejects_bad_pkce_and_client() {
    let app = test_app();

    let code = obtain_code(&app).await;
    let (status, error) = exchange(
        &app,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", "test-client"),
            ("code_verifier", "not-the-right-verifier-at-all-nope"),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["error"], "invalid_grant");

    let code = obtain_code(&app).await;
    let (status, error) = exchange(
        &app,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", "someone-else"),
            ("code_verifier", VERIFIER),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["error"], "invalid_grant");
}

#[tokio::test(start_paused = true)]
async fn wrong_passphrase_shows_error_and_no_redirect() {
    let app = test_app();
    let response = submit_passphrase(&app, &authorize_uri(CALLBACK, None), "wrong-guess").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get(header::LOCATION).is_none());
    assert!(body_string(response).await.contains("Incorrect passphrase"));
}

#[tokio::test]
async fn authorize_rejects_unlisted_redirect_without_redirecting() {
    let app = test_app();
    let response = app
        .clone()
        .oneshot(
            Request::get(authorize_uri("https://evil.example.com/cb", None))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(response.headers().get(header::LOCATION).is_none());
}

#[tokio::test]
async fn authorize_rejects_foreign_resource() {
    let app = test_app();
    let response = app
        .clone()
        .oneshot(
            Request::get(authorize_uri(
                CALLBACK,
                Some("https://other.example.com/mcp"),
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FOUND);
    let params = location_params(&response);
    assert_eq!(params["error"], "invalid_target");
    assert_eq!(params["state"], "opaque-state-123");
}

#[tokio::test]
async fn bearer_required_on_mcp() {
    let app = test_app();

    let missing = get_with_bearer(&app, None).await;
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
    let www = missing.headers()[header::WWW_AUTHENTICATE]
        .to_str()
        .unwrap();
    assert!(www.contains("resource_metadata="));
    assert!(www.contains("/.well-known/oauth-protected-resource/mcp"));

    let garbage = get_with_bearer(&app, Some("not-a-jwt")).await;
    assert_eq!(garbage.status(), StatusCode::UNAUTHORIZED);

    // A refresh token must not pass as an access token.
    let code = obtain_code(&app).await;
    let (_, tokens) = exchange(
        &app,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", "test-client"),
            ("code_verifier", VERIFIER),
        ],
    )
    .await;
    let refresh = tokens["refresh_token"].as_str().unwrap();
    assert_eq!(
        get_with_bearer(&app, Some(refresh)).await.status(),
        StatusCode::UNAUTHORIZED
    );
}
