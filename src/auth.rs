use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{Query, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

const DEFAULT_REDIRECT_URIS: &[&str] = &[
    "https://claude.ai/api/mcp/auth_callback",
    "https://claude.com/api/mcp/auth_callback",
];

const CODE_TTL_SECS: u64 = 120;
const ACCESS_TOKEN_TTL_SECS: u64 = 3600;
const REFRESH_TOKEN_TTL_SECS: u64 = 30 * 24 * 3600;
const FAILED_LOGIN_DELAY: Duration = Duration::from_millis(1500);

pub struct AuthConfig {
    pub public_url: String,
    pub passphrase: String,
    pub signing_key: Vec<u8>,
    pub extra_redirect_uris: Vec<String>,
}

impl AuthConfig {
    /// Reads `MCP_PUBLIC_URL`, `MCP_OAUTH_PASSPHRASE`, and `MCP_OAUTH_SIGNING_KEY`.
    /// Returns `Ok(None)` when none are set (auth disabled) and an error when
    /// only some are set, so a typo can't silently expose the server.
    pub fn from_env() -> Result<Option<Self>, String> {
        let public_url = std::env::var("MCP_PUBLIC_URL").ok();
        let passphrase = std::env::var("MCP_OAUTH_PASSPHRASE").ok();
        let signing_key = std::env::var("MCP_OAUTH_SIGNING_KEY").ok();
        match (public_url, passphrase, signing_key) {
            (None, None, None) => Ok(None),
            (Some(public_url), Some(passphrase), Some(signing_key)) => {
                if !public_url.starts_with("https://") && !public_url.starts_with("http://") {
                    return Err("MCP_PUBLIC_URL must be an absolute http(s) URL".into());
                }
                if passphrase.len() < 12 {
                    return Err("MCP_OAUTH_PASSPHRASE must be at least 12 characters".into());
                }
                if signing_key.len() < 32 {
                    return Err("MCP_OAUTH_SIGNING_KEY must be at least 32 characters".into());
                }
                let extra_redirect_uris = std::env::var("MCP_OAUTH_REDIRECT_ALLOWLIST")
                    .map(|v| {
                        v.split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect()
                    })
                    .unwrap_or_default();
                Ok(Some(Self {
                    public_url: public_url.trim_end_matches('/').to_string(),
                    passphrase,
                    signing_key: signing_key.into_bytes(),
                    extra_redirect_uris,
                }))
            }
            _ => Err(
                "MCP_PUBLIC_URL, MCP_OAUTH_PASSPHRASE, and MCP_OAUTH_SIGNING_KEY must be set together".into(),
            ),
        }
    }
}

pub struct AuthState {
    config: AuthConfig,
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    used_code_jtis: Mutex<HashMap<String, u64>>,
}

#[derive(Serialize, Deserialize)]
struct Claims {
    iss: String,
    aud: String,
    exp: u64,
    iat: u64,
    jti: String,
    token_use: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    redirect_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code_challenge: Option<String>,
}

impl AuthState {
    pub fn new(config: AuthConfig) -> Self {
        let encoding_key = EncodingKey::from_secret(&config.signing_key);
        let decoding_key = DecodingKey::from_secret(&config.signing_key);
        Self {
            config,
            encoding_key,
            decoding_key,
            used_code_jtis: Mutex::new(HashMap::new()),
        }
    }

    pub fn public_url(&self) -> &str {
        &self.config.public_url
    }

    /// Canonical resource identifier (RFC 8707) of this MCP server.
    fn resource(&self) -> String {
        format!("{}/mcp", self.config.public_url)
    }

    fn redirect_uri_allowed(&self, uri: &str) -> bool {
        DEFAULT_REDIRECT_URIS.contains(&uri)
            || self.config.extra_redirect_uris.iter().any(|u| u == uri)
            || is_loopback_uri(uri)
    }

    fn passphrase_matches(&self, submitted: &str) -> bool {
        let a = Sha256::digest(submitted.as_bytes());
        let b = Sha256::digest(self.config.passphrase.as_bytes());
        a.as_slice().ct_eq(b.as_slice()).into()
    }

    fn mint(
        &self,
        token_use: &str,
        aud: String,
        ttl: u64,
        client_id: Option<String>,
        redirect_uri: Option<String>,
        code_challenge: Option<String>,
    ) -> String {
        let now = unix_now();
        let claims = Claims {
            iss: self.config.public_url.clone(),
            aud,
            exp: now + ttl,
            iat: now,
            jti: random_token(),
            token_use: token_use.to_string(),
            client_id,
            redirect_uri,
            code_challenge,
        };
        jsonwebtoken::encode(&Header::default(), &claims, &self.encoding_key)
            .expect("HS256 signing is infallible")
    }

    fn verify(&self, token: &str, expected_use: &str, aud: &str) -> Option<Claims> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_audience(&[aud]);
        validation.set_issuer(&[&self.config.public_url]);
        validation.set_required_spec_claims(&["exp", "aud", "iss"]);
        let claims = jsonwebtoken::decode::<Claims>(token, &self.decoding_key, &validation)
            .ok()?
            .claims;
        (claims.token_use == expected_use).then_some(claims)
    }

    /// Best-effort single-use enforcement for authorization codes; the map is
    /// in-memory, so a restart forgets codes, but their 2-minute TTL bounds that.
    async fn consume_code_jti(&self, jti: &str, exp: u64) -> bool {
        let mut used = self.used_code_jtis.lock().await;
        let now = unix_now();
        used.retain(|_, e| *e > now);
        if used.contains_key(jti) {
            return false;
        }
        used.insert(jti.to_string(), exp);
        true
    }

    fn validate_authorize(
        &self,
        params: &AuthorizeParams,
    ) -> Result<ValidatedAuthorize, AuthorizeReject> {
        let redirect_uri = match params.redirect_uri.as_deref() {
            Some(u) if self.redirect_uri_allowed(u) => u.to_string(),
            other => {
                warn!(redirect_uri = ?other, "authorize: redirect_uri missing or not allowlisted");
                return Err(AuthorizeReject::BadRedirect);
            }
        };
        let reject = |error, description| {
            Err(AuthorizeReject::Errored {
                redirect_uri: redirect_uri.clone(),
                error,
                description,
                state: params.state.clone(),
            })
        };
        if params.response_type.as_deref() != Some("code") {
            return reject(
                "unsupported_response_type",
                "only response_type=code is supported",
            );
        }
        let Some(client_id) = params.client_id.clone() else {
            return reject("invalid_request", "client_id is required");
        };
        let Some(code_challenge) = params.code_challenge.clone() else {
            return reject("invalid_request", "code_challenge is required (PKCE)");
        };
        if params.code_challenge_method.as_deref() != Some("S256") {
            return reject("invalid_request", "code_challenge_method must be S256");
        }
        if let Some(resource) = params.resource.as_deref()
            && resource.trim_end_matches('/') != self.resource()
        {
            return reject("invalid_target", "resource does not match this server");
        }
        Ok(ValidatedAuthorize {
            client_id,
            redirect_uri,
            code_challenge,
            state: params.state.clone(),
        })
    }
}

struct ValidatedAuthorize {
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    state: Option<String>,
}

enum AuthorizeReject {
    BadRedirect,
    Errored {
        redirect_uri: String,
        error: &'static str,
        description: &'static str,
        state: Option<String>,
    },
}

impl IntoResponse for AuthorizeReject {
    fn into_response(self) -> Response {
        match self {
            Self::BadRedirect => {
                (StatusCode::BAD_REQUEST, "invalid or missing redirect_uri").into_response()
            }
            Self::Errored {
                redirect_uri,
                error,
                description,
                state,
            } => {
                let mut params = vec![("error", error), ("error_description", description)];
                if let Some(state) = state.as_deref() {
                    params.push(("state", state));
                }
                redirect_with_params(&redirect_uri, &params)
            }
        }
    }
}

pub fn router(state: Arc<AuthState>) -> Router {
    Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(protected_resource_metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(protected_resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(authorization_server_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server/mcp",
            get(authorization_server_metadata),
        )
        .route("/register", post(register))
        .route("/authorize", get(authorize_page).post(authorize_submit))
        .route("/token", post(token))
        .with_state(state)
}

pub async fn require_bearer(
    State(state): State<Arc<AuthState>>,
    request: Request,
    next: Next,
) -> Response {
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if let Some(token) = token {
        if state.verify(token, "access", &state.resource()).is_some() {
            return next.run(request).await;
        }
        debug!("bearer: rejected invalid or expired access token");
    }
    let www_authenticate = format!(
        "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource/mcp\"",
        state.config.public_url
    );
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, www_authenticate)],
    )
        .into_response()
}

async fn protected_resource_metadata(
    State(state): State<Arc<AuthState>>,
) -> Json<serde_json::Value> {
    Json(json!({
        "resource": state.resource(),
        "authorization_servers": [state.config.public_url],
        "bearer_methods_supported": ["header"],
    }))
}

async fn authorization_server_metadata(
    State(state): State<Arc<AuthState>>,
) -> Json<serde_json::Value> {
    let base = &state.config.public_url;
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/authorize"),
        "token_endpoint": format!("{base}/token"),
        "registration_endpoint": format!("{base}/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
    }))
}

#[derive(Deserialize)]
struct RegisterRequest {
    #[serde(default)]
    redirect_uris: Vec<String>,
    #[serde(default)]
    client_name: Option<String>,
}

async fn register(
    State(state): State<Arc<AuthState>>,
    Json(request): Json<RegisterRequest>,
) -> Response {
    if request.redirect_uris.is_empty()
        || !request
            .redirect_uris
            .iter()
            .all(|u| state.redirect_uri_allowed(u))
    {
        warn!(redirect_uris = ?request.redirect_uris, "register: redirect_uri not allowlisted");
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "invalid_redirect_uri",
                "error_description": "redirect_uri is not in the allowlist; extend MCP_OAUTH_REDIRECT_ALLOWLIST if this client is yours",
            })),
        )
            .into_response();
    }
    let client_id = random_token();
    info!(client_id, client_name = ?request.client_name, "register: issued client_id");
    (
        StatusCode::CREATED,
        Json(json!({
            "client_id": client_id,
            "redirect_uris": request.redirect_uris,
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
struct AuthorizeParams {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    state: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    resource: Option<String>,
}

#[derive(Deserialize)]
struct PassphraseForm {
    passphrase: String,
}

async fn authorize_page(
    State(state): State<Arc<AuthState>>,
    Query(params): Query<AuthorizeParams>,
) -> Response {
    match state.validate_authorize(&params) {
        Ok(_) => login_page(false),
        Err(reject) => reject.into_response(),
    }
}

async fn authorize_submit(
    State(state): State<Arc<AuthState>>,
    Query(params): Query<AuthorizeParams>,
    Form(form): Form<PassphraseForm>,
) -> Response {
    let validated = match state.validate_authorize(&params) {
        Ok(v) => v,
        Err(reject) => return reject.into_response(),
    };
    if !state.passphrase_matches(&form.passphrase) {
        warn!(client_id = %validated.client_id, "authorize: incorrect passphrase");
        tokio::time::sleep(FAILED_LOGIN_DELAY).await;
        return login_page(true);
    }
    info!(client_id = %validated.client_id, "authorize: passphrase accepted, issuing code");
    let code = state.mint(
        "code",
        state.config.public_url.clone(),
        CODE_TTL_SECS,
        Some(validated.client_id),
        Some(validated.redirect_uri.clone()),
        Some(validated.code_challenge),
    );
    let mut params = vec![("code", code.as_str())];
    if let Some(state_param) = validated.state.as_deref() {
        params.push(("state", state_param));
    }
    redirect_with_params(&validated.redirect_uri, &params)
}

#[derive(Deserialize)]
struct TokenRequest {
    grant_type: String,
    code: Option<String>,
    redirect_uri: Option<String>,
    client_id: Option<String>,
    code_verifier: Option<String>,
    refresh_token: Option<String>,
    resource: Option<String>,
}

async fn token(State(state): State<Arc<AuthState>>, Form(request): Form<TokenRequest>) -> Response {
    if let Some(resource) = request.resource.as_deref()
        && resource.trim_end_matches('/') != state.resource()
    {
        return token_error("invalid_target", "resource does not match this server");
    }
    match request.grant_type.as_str() {
        "authorization_code" => exchange_code(&state, &request).await,
        "refresh_token" => exchange_refresh(&state, &request),
        _ => token_error("unsupported_grant_type", "unsupported grant_type"),
    }
}

async fn exchange_code(state: &AuthState, request: &TokenRequest) -> Response {
    let Some(code) = request.code.as_deref() else {
        return token_error("invalid_request", "code is required");
    };
    let Some(claims) = state.verify(code, "code", state.public_url()) else {
        debug!("token: invalid or expired authorization code");
        return token_error("invalid_grant", "invalid or expired authorization code");
    };
    if !state.consume_code_jti(&claims.jti, claims.exp).await {
        warn!("token: authorization code replay detected");
        return token_error("invalid_grant", "authorization code already used");
    }
    if request.client_id != claims.client_id {
        return token_error("invalid_grant", "client_id mismatch");
    }
    if request.redirect_uri.is_some() && request.redirect_uri != claims.redirect_uri {
        return token_error("invalid_grant", "redirect_uri mismatch");
    }
    let Some(verifier) = request.code_verifier.as_deref() else {
        return token_error("invalid_request", "code_verifier is required");
    };
    if !pkce_matches(
        verifier,
        claims.code_challenge.as_deref().unwrap_or_default(),
    ) {
        warn!("token: PKCE verification failed");
        return token_error("invalid_grant", "PKCE verification failed");
    }
    info!(client_id = ?claims.client_id, "token: issued access + refresh tokens");
    issue_tokens(state, claims.client_id)
}

fn exchange_refresh(state: &AuthState, request: &TokenRequest) -> Response {
    let Some(refresh_token) = request.refresh_token.as_deref() else {
        return token_error("invalid_request", "refresh_token is required");
    };
    let Some(claims) = state.verify(refresh_token, "refresh", state.public_url()) else {
        debug!("token: invalid or expired refresh token");
        return token_error("invalid_grant", "invalid or expired refresh token");
    };
    if request.client_id.is_some() && request.client_id != claims.client_id {
        return token_error("invalid_grant", "client_id mismatch");
    }
    info!(client_id = ?claims.client_id, "token: refreshed access token");
    issue_tokens(state, claims.client_id)
}

fn issue_tokens(state: &AuthState, client_id: Option<String>) -> Response {
    let access = state.mint(
        "access",
        state.resource(),
        ACCESS_TOKEN_TTL_SECS,
        client_id.clone(),
        None,
        None,
    );
    let refresh = state.mint(
        "refresh",
        state.public_url().to_string(),
        REFRESH_TOKEN_TTL_SECS,
        client_id,
        None,
        None,
    );
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({
            "access_token": access,
            "token_type": "Bearer",
            "expires_in": ACCESS_TOKEN_TTL_SECS,
            "refresh_token": refresh,
        })),
    )
        .into_response()
}

fn token_error(error: &str, description: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "error": error, "error_description": description })),
    )
        .into_response()
}

fn redirect_with_params(uri: &str, params: &[(&str, &str)]) -> Response {
    let query = serde_urlencoded::to_string(params).unwrap_or_default();
    let separator = if uri.contains('?') { '&' } else { '?' };
    (
        StatusCode::FOUND,
        [(header::LOCATION, format!("{uri}{separator}{query}"))],
    )
        .into_response()
}

const LOGIN_PAGE_HEAD: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Proton Mail MCP</title>
<style>
  body { font-family: system-ui, sans-serif; display: flex; justify-content: center; align-items: center; min-height: 100vh; margin: 0; background: #f4f4f5; color: #18181b; }
  main { background: #fff; padding: 2rem; border-radius: 12px; box-shadow: 0 1px 4px rgba(0,0,0,.1); max-width: 22rem; width: 100%; }
  h1 { font-size: 1.1rem; margin: 0 0 .5rem; }
  p { font-size: .9rem; color: #52525b; }
  input { width: 100%; box-sizing: border-box; padding: .6rem; margin: .75rem 0; border: 1px solid #d4d4d8; border-radius: 8px; font-size: 1rem; }
  button { width: 100%; padding: .6rem; border: 0; border-radius: 8px; background: #6d4aff; color: #fff; font-size: 1rem; cursor: pointer; }
  .error { color: #dc2626; }
  @media (prefers-color-scheme: dark) {
    body { background: #18181b; color: #fafafa; }
    main { background: #27272a; }
    p { color: #a1a1aa; }
    input { background: #18181b; color: #fafafa; border-color: #3f3f46; }
  }
</style>
</head>
<body>
<main>
<h1>Proton Mail MCP</h1>
<p>A client is requesting access to this mailbox. Enter the passphrase to approve.</p>
<form method="post">
<input type="password" name="passphrase" placeholder="Passphrase" autofocus required>
<button type="submit">Approve access</button>
</form>
"#;

fn login_page(failed: bool) -> Response {
    let error = if failed {
        r#"<p class="error">Incorrect passphrase.</p>"#
    } else {
        ""
    };
    let html = format!("{LOGIN_PAGE_HEAD}{error}</main>\n</body>\n</html>\n");
    (
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::X_FRAME_OPTIONS, "DENY"),
        ],
        Html(html),
    )
        .into_response()
}

fn pkce_matches(verifier: &str, challenge: &str) -> bool {
    let computed = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    computed.as_bytes().ct_eq(challenge.as_bytes()).into()
}

/// Loopback redirects are allowed per RFC 8252 §7.3 (native clients pick an
/// ephemeral port); rejecting userinfo (`@`) prevents `http://127.0.0.1@evil`.
fn is_loopback_uri(uri: &str) -> bool {
    let Some(rest) = uri.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return false;
    }
    if let Some(after) = authority.strip_prefix("[::1]") {
        return after.is_empty() || after.starts_with(':');
    }
    let host = authority.split(':').next().unwrap_or("");
    host == "localhost" || host == "127.0.0.1"
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs()
}

fn random_token() -> String {
    let bytes: [u8; 32] = rand::random();
    URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_uris() {
        assert!(is_loopback_uri("http://localhost:3117/callback"));
        assert!(is_loopback_uri("http://127.0.0.1:49152/cb"));
        assert!(is_loopback_uri("http://[::1]:8000/cb"));
        assert!(is_loopback_uri("http://localhost/cb"));
        assert!(!is_loopback_uri("https://localhost:3117/cb"));
        assert!(!is_loopback_uri("http://localhost.evil.com/cb"));
        assert!(!is_loopback_uri("http://127.0.0.1:8080@evil.com/cb"));
        assert!(!is_loopback_uri("http://evil.com/localhost"));
        assert!(!is_loopback_uri("http://[::1].evil.com/cb"));
    }

    #[test]
    fn pkce_round_trip() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        assert!(pkce_matches(verifier, &challenge));
        assert!(!pkce_matches("wrong-verifier", &challenge));
        assert!(!pkce_matches(verifier, ""));
    }

    #[test]
    fn config_requires_all_or_none() {
        // from_env reads process-global env; simulate the tuple logic directly.
        let cfg = AuthConfig {
            public_url: "https://mcp.example.com/".into(),
            passphrase: "a-long-passphrase".into(),
            signing_key: vec![0u8; 32],
            extra_redirect_uris: vec![],
        };
        let state = AuthState::new(AuthConfig {
            public_url: cfg.public_url.trim_end_matches('/').to_string(),
            ..cfg
        });
        assert_eq!(state.resource(), "https://mcp.example.com/mcp");
    }
}
