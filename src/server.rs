use std::sync::{Arc, Mutex};

use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};

use crate::email_client::{
    ArchiveMessageRequest, EmailClient, GetAttachmentRequest, GetMailboxStatusRequest,
    GetMessageRequest, ListMessagesByThreadRequest, ListMessagesRequest, MoveMessageRequest,
    SearchMessagesRequest,
};

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

    #[tool(
        title = "List Messages",
        description = "List messages in a mailbox, sorted by most recent first. Supports pagination via before_uid — pass the lowest UID from the previous page to get the next page.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn list_messages(
        &self,
        Parameters(req): Parameters<ListMessagesRequest>,
    ) -> Result<String, String> {
        let client = self.client.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut client = client.lock().map_err(|e| e.to_string())?;
            client.list_messages(req)
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))??;

        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    }

    #[tool(
        title = "Get Message",
        description = "Retrieve a full email message by UID, including headers, body (text and HTML), and attachment metadata. Does not mark the message as read. HTML-only emails are automatically converted to plain text for body_text. Raw HTML is omitted by default to save tokens; set include_html=true to receive it.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn get_message(
        &self,
        Parameters(req): Parameters<GetMessageRequest>,
    ) -> Result<String, String> {
        let client = self.client.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut client = client.lock().map_err(|e| e.to_string())?;
            client.get_message(req)
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))??;

        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    }

    #[tool(
        title = "Search Messages",
        description = "Search for messages in a mailbox using IMAP SEARCH. Supports filtering by from, to, subject, body text, date range (DD-Mon-YYYY format), seen/unseen status, and flagged status. Multiple criteria are combined with AND.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn search_messages(
        &self,
        Parameters(req): Parameters<SearchMessagesRequest>,
    ) -> Result<String, String> {
        let client = self.client.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut client = client.lock().map_err(|e| e.to_string())?;
            client.search_messages(req)
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))??;

        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    }

    #[tool(
        title = "Get Mailbox Status",
        description = "Get lightweight status for a mailbox: message count, recent count, unseen count, UIDNEXT, and UIDVALIDITY. Useful for checking activity without fetching messages.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn get_mailbox_status(
        &self,
        Parameters(req): Parameters<GetMailboxStatusRequest>,
    ) -> Result<String, String> {
        let client = self.client.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut client = client.lock().map_err(|e| e.to_string())?;
            client.get_mailbox_status(req)
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))??;

        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    }

    #[tool(
        title = "Get Attachment",
        description = "Download a specific attachment from a message by UID and 0-based attachment index (matching the order from get_message). Returns base64-encoded content.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn get_attachment(
        &self,
        Parameters(req): Parameters<GetAttachmentRequest>,
    ) -> Result<String, String> {
        let client = self.client.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut client = client.lock().map_err(|e| e.to_string())?;
            client.get_attachment(req)
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))??;

        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    }

    #[tool(
        title = "Archive Message",
        description = "Move a message to the Archive mailbox by UID. Requires the source mailbox name and the message UID.",
        annotations(read_only_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn archive_message(
        &self,
        Parameters(req): Parameters<ArchiveMessageRequest>,
    ) -> Result<String, String> {
        let client = self.client.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut client = client.lock().map_err(|e| e.to_string())?;
            client.archive_message(req)
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))??;

        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    }

    #[tool(
        title = "Move Message",
        description = "Move a message to a different mailbox by UID. Requires the source mailbox, destination mailbox, and message UID. Moving to Trash is blocked to prevent accidental deletion — use your email client for that.",
        annotations(read_only_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn move_message(
        &self,
        Parameters(req): Parameters<MoveMessageRequest>,
    ) -> Result<String, String> {
        let client = self.client.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut client = client.lock().map_err(|e| e.to_string())?;
            client.move_message(req)
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))??;

        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    }

    #[tool(
        title = "List Messages by Thread",
        description = "Find related messages in a thread by Message-ID or subject. Searches In-Reply-To, References, and Message-ID headers to reconstruct conversation threads. At least one of message_id or subject must be provided. Note: search is limited to a single mailbox.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn list_messages_by_thread(
        &self,
        Parameters(req): Parameters<ListMessagesByThreadRequest>,
    ) -> Result<String, String> {
        let client = self.client.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut client = client.lock().map_err(|e| e.to_string())?;
            client.list_messages_by_thread(req)
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
