use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rmcp::ServiceExt;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use proton_mail_mcp::attachments::{self, AttachmentSigner, DeliveryConfig, LinkDelivery};
use proton_mail_mcp::auth::{self, AuthConfig, AuthState};
use proton_mail_mcp::email_client::EmailClient;
use proton_mail_mcp::server::ProtonMailService;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let username = env::var("PROTON_USERNAME").map_err(|_| {
        error!("PROTON_USERNAME environment variable is required");
        "PROTON_USERNAME environment variable is required"
    })?;
    let password = env::var("PROTON_PASSWORD").map_err(|_| {
        error!("PROTON_PASSWORD environment variable is required");
        "PROTON_PASSWORD environment variable is required"
    })?;
    let hostname = env::var("PROTON_HOSTNAME").unwrap_or_else(|_| "127.0.0.1".into());
    let port: u16 = env::var("PROTON_PORT")
        .unwrap_or_else(|_| "1143".into())
        .parse()
        .map_err(|_| {
            error!("PROTON_PORT must be a valid port number");
            "PROTON_PORT must be a valid port number"
        })?;

    let tls_mode = env::var("PROTON_TLS_MODE").unwrap_or_else(|_| "starttls".into());
    let direct_tls = match tls_mode.as_str() {
        "starttls" => false,
        "tls" => true,
        _ => {
            error!("PROTON_TLS_MODE must be \"starttls\" or \"tls\"");
            return Err("PROTON_TLS_MODE must be \"starttls\" or \"tls\"".into());
        }
    };

    let inline_max_bytes = match env::var("MCP_ATTACHMENT_INLINE_MAX_BYTES") {
        Ok(raw) => raw.parse().map_err(|_| {
            error!("MCP_ATTACHMENT_INLINE_MAX_BYTES must be a byte count");
            "MCP_ATTACHMENT_INLINE_MAX_BYTES must be a byte count"
        })?,
        Err(_) => attachments::DEFAULT_INLINE_MAX_BYTES,
    };

    let client = connect_with_retry(&hostname, port, &username, &password, direct_tls).await?;

    match env::var("MCP_BIND") {
        Ok(bind) => serve_http(client, &bind, inline_max_bytes).await,
        Err(_) => serve_stdio(client, inline_max_bytes).await,
    }
}

async fn connect_with_retry(
    hostname: &str,
    port: u16,
    username: &str,
    password: &str,
    direct_tls: bool,
) -> Result<EmailClient, Box<dyn std::error::Error>> {
    const RETRY_LIMIT: usize = 10;
    info!("Connecting to Proton Mail Bridge at {hostname}:{port}");
    let mut last_err = None;
    for attempt in 1..=RETRY_LIMIT {
        let connect = if direct_tls {
            EmailClient::new_direct_tls(hostname, port, username, password).await
        } else {
            EmailClient::new(hostname, port, username, password).await
        };
        match connect {
            Ok(client) => {
                info!("Connected successfully");
                return Ok(client);
            }
            Err(e) => {
                error!("attempt {attempt}/{RETRY_LIMIT} failed to connect: {e}");
                last_err = Some(e);
                if attempt < RETRY_LIMIT {
                    tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
                }
            }
        }
    }
    Err(last_err.unwrap().into())
}

async fn serve_stdio(
    client: EmailClient,
    inline_max_bytes: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let dir = env::var_os("MCP_ATTACHMENT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join("proton-mail-mcp"));
    info!("large attachments will be written under {}", dir.display());
    let delivery = Arc::new(DeliveryConfig {
        inline_max_bytes,
        link: LinkDelivery::File { dir },
    });
    let service = ProtonMailService::new(client, delivery);
    let server = service.serve(rmcp::transport::io::stdio()).await?;
    info!("MCP server running on stdio");
    server.waiting().await?;
    Ok(())
}

async fn serve_http(
    client: EmailClient,
    bind: &str,
    inline_max_bytes: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    // One ProtonMailService is cloned per HTTP session; clones share the same
    // Arc<Mutex<EmailClient>>, so all sessions reuse a single Bridge connection.
    let auth_config = AuthConfig::from_env().map_err(|e| {
        error!("{e}");
        e
    })?;
    let signer = Arc::new(match &auth_config {
        Some(config) => AttachmentSigner::new(&config.public_url, &config.signing_key),
        None => AttachmentSigner::ephemeral(&local_base_url(bind)),
    });
    let delivery = Arc::new(DeliveryConfig {
        inline_max_bytes,
        link: LinkDelivery::SignedUrl(signer.clone()),
    });
    let service = ProtonMailService::new(client, delivery);
    let attachment_routes = attachments::router(signer, service.email_client());
    let mcp_service = proton_mail_mcp::http::mcp_service(
        service,
        auth_config
            .as_ref()
            .map(|config| config.public_url.as_str()),
    )?;
    let app = match auth_config {
        Some(config) => {
            info!("OAuth enabled; issuer {}", config.public_url);
            let state = Arc::new(AuthState::new(config));
            axum::Router::new()
                .nest_service("/mcp", mcp_service)
                .route_layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    auth::require_bearer,
                ))
                .merge(auth::router(state))
        }
        None => {
            warn!("MCP_OAUTH_* not set — serving /mcp WITHOUT authentication");
            axum::Router::new().nest_service("/mcp", mcp_service)
        }
    }
    .merge(attachment_routes)
    .route("/healthz", axum::routing::get(|| async { "ok" }));
    let listener = tokio::net::TcpListener::bind(bind).await?;
    info!("MCP server (streamable-http) listening on {bind}, endpoint /mcp");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Origin for attachment links when no `MCP_PUBLIC_URL` is configured:
/// the bind address, with wildcard hosts rewritten to loopback.
fn local_base_url(bind: &str) -> String {
    let (host, port) = bind.rsplit_once(':').unwrap_or((bind, "80"));
    let host = match host {
        "0.0.0.0" | "[::]" | "::" | "" => "127.0.0.1",
        other => other,
    };
    format!("http://{host}:{port}")
}
