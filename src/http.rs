use rmcp::ServerHandler;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};

pub fn mcp_service<S>(
    service: S,
    public_url: Option<&str>,
) -> Result<StreamableHttpService<S, LocalSessionManager>, axum::http::uri::InvalidUri>
where
    S: ServerHandler + Clone + Send + Sync + 'static,
{
    let mut config = StreamableHttpServerConfig::default().with_sse_retry(None);
    if let Some(public_url) = public_url {
        let uri = public_url.parse::<axum::http::Uri>()?;
        if let Some(authority) = uri.authority() {
            config.allowed_hosts.push(authority.to_string());
        }
    }
    Ok(StreamableHttpService::new(
        move || Ok(service.clone()),
        LocalSessionManager::default().into(),
        config,
    ))
}
