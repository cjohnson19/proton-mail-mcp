use std::env;
use std::time::Duration;

use rmcp::ServiceExt;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use proton_mail_mcp::email_client::EmailClient;
use proton_mail_mcp::server::ProtonMailService;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    const RETRY_LIMIT: usize = 10;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::DEBUG.into()))
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

    info!("Connecting to Proton Mail Bridge at {hostname}:{port}");
    let mut last_err = None;
    for attempt in 1..=RETRY_LIMIT {
        match EmailClient::new(&hostname, port, &username, &password).await {
            Ok(client) => {
                info!("Connected successfully");
                let service = ProtonMailService::new(client);
                let server = service.serve(rmcp::transport::io::stdio()).await?;
                info!("MCP server running on stdio");
                server.waiting().await?;
                return Ok(());
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
