use std::sync::{Arc, Mutex};

use rmcp::{
    ServerHandler,
    handler::server::router::tool::ToolRouter,
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};

use crate::email_client::EmailClient;

#[derive(Clone)]
pub struct ProtonMailService {
    client: Arc<Mutex<EmailClient>>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl ProtonMailService {
    pub fn new(client: EmailClient) -> Self {
        Self {
            client: Arc::new(Mutex::new(client)),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        title = "List Mailboxes",
        description = "List all mailboxes in the Proton Mail account",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn list_mailboxes(&self) -> Result<String, String> {
        let client = self.client.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut client = client.lock().map_err(|e| e.to_string())?;
            client.list_mailboxes()
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))??;

        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ProtonMailService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Proton Mail MCP server — interact with Proton Mail via IMAP.".into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "proton-mail-mcp".into(),
                title: Some("Proton Mail".into()),
                version: env!("CARGO_PKG_VERSION").into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }
}
