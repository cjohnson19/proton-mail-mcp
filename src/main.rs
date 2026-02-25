use std::env;

use rmcp::ServiceExt;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use crate::email_client::EmailClient;
use crate::server::ProtonMailService;

mod email_client;
mod server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    let client = EmailClient::new(&hostname, port, &username, &password).map_err(|e| {
        error!(error = %e, "failed to connect to IMAP server");
        e
    })?;
    info!("Connected successfully");

    let service = ProtonMailService::new(client);
    let server = service.serve(rmcp::transport::io::stdio()).await?;
    info!("MCP server running on stdio");
    server.waiting().await?;

    Ok(())
}
