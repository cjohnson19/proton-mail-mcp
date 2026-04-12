use std::sync::Arc;

use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use tokio::sync::Mutex;
use tracing::{error, info};

use crate::email_client::{
    ArchiveMessageRequest, DraftEmailRequest, EmailClient, GetAttachmentRequest,
    GetMailboxStatusRequest, GetMessageRequest, ListMessagesByThreadRequest, ListMessagesRequest,
    MarkAsReadRequest, MoveMessageRequest, SearchMessagesRequest, TrashMessageRequest,
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
        info!("tool.list_mailboxes");
        let mut client = self.client.lock().await;
        let result = client.list_mailboxes().await.map_err(|e| {
            error!(error = %e, "tool.list_mailboxes failed");
            e
        })?;
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
        info!(mailbox = %req.mailbox_name, limit = req.limit, before_uid = ?req.before_uid, "tool.list_messages");
        let mut client = self.client.lock().await;
        let result = client.list_messages(req).await.map_err(|e| {
            error!(error = %e, "tool.list_messages failed");
            e
        })?;
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
        info!(mailbox = %req.mailbox_name, uid = req.uid, "tool.get_message");
        let mut client = self.client.lock().await;
        let result = client.get_message(req).await.map_err(|e| {
            error!(error = %e, "tool.get_message failed");
            e
        })?;
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
        info!(mailbox = %req.mailbox_name, "tool.search_messages");
        let mut client = self.client.lock().await;
        let result = client.search_messages(req).await.map_err(|e| {
            error!(error = %e, "tool.search_messages failed");
            e
        })?;
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
        info!(mailbox = %req.mailbox_name, "tool.get_mailbox_status");
        let mut client = self.client.lock().await;
        let result = client.get_mailbox_status(req).await.map_err(|e| {
            error!(error = %e, "tool.get_mailbox_status failed");
            e
        })?;
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
        info!(mailbox = %req.mailbox_name, uid = req.uid, index = req.attachment_index, "tool.get_attachment");
        let mut client = self.client.lock().await;
        let result = client.get_attachment(req).await.map_err(|e| {
            error!(error = %e, "tool.get_attachment failed");
            e
        })?;
        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    }

    #[tool(
        title = "Archive Message",
        description = "Move a message to the Archive mailbox by UID. Requires the source mailbox name and the message UID.",
        annotations(
            read_only_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn archive_message(
        &self,
        Parameters(req): Parameters<ArchiveMessageRequest>,
    ) -> Result<String, String> {
        info!(mailbox = %req.mailbox_name, uid = req.uid, "tool.archive_message");
        let mut client = self.client.lock().await;
        let result = client.archive_message(req).await.map_err(|e| {
            error!(error = %e, "tool.archive_message failed");
            e
        })?;
        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    }

    #[tool(
        title = "Trash Message",
        description = "Move a message to the Trash mailbox by UID. Requires the source mailbox name and the message UID.",
        annotations(
            read_only_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn trash_message(
        &self,
        Parameters(req): Parameters<TrashMessageRequest>,
    ) -> Result<String, String> {
        info!(source = %req.source_mailbox, uid = req.uid, "tool.trash_message");
        let mut client = self.client.lock().await;
        let result = client.trash_message(req).await.map_err(|e| {
            error!(error = %e, "tool.trash_message failed");
            e
        })?;
        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    }

    #[tool(
        title = "Move Message",
        description = "Move a message to a different mailbox by UID. Requires the source mailbox, destination mailbox, and message UID. Moving to Trash is blocked — use the trash_message tool instead.",
        annotations(
            read_only_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn move_message(
        &self,
        Parameters(req): Parameters<MoveMessageRequest>,
    ) -> Result<String, String> {
        info!(source = %req.source_mailbox, destination = %req.destination_mailbox, uid = req.uid, "tool.move_message");
        let mut client = self.client.lock().await;
        let result = client.move_message(req).await.map_err(|e| {
            error!(error = %e, "tool.move_message failed");
            e
        })?;
        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    }

    #[tool(
        title = "Mark as Read",
        description = "Mark a message as read or unread by UID. Defaults to marking as read; set read=false to mark as unread.",
        annotations(
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn mark_as_read(
        &self,
        Parameters(req): Parameters<MarkAsReadRequest>,
    ) -> Result<String, String> {
        info!(mailbox = %req.mailbox_name, uid = req.uid, read = req.read, "tool.mark_as_read");
        let mut client = self.client.lock().await;
        let result = client.mark_as_read(req).await.map_err(|e| {
            error!(error = %e, "tool.mark_as_read failed");
            e
        })?;
        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    }

    #[tool(
        title = "Draft Email",
        description = "Compose an email and save it to the Drafts mailbox. Supports To/Cc/Bcc, subject, and a plain-text body. Pass in_reply_to (source mailbox + UID) to thread as a reply — the original Message-ID and References are fetched and set on the draft. The draft is NOT sent; the user must review and send it from their mail client.",
        annotations(
            read_only_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn draft_email(
        &self,
        Parameters(req): Parameters<DraftEmailRequest>,
    ) -> Result<String, String> {
        info!(
            drafts_mailbox = %req.drafts_mailbox,
            to_count = req.to.len(),
            threaded = req.in_reply_to.is_some(),
            "tool.draft_email"
        );
        let mut client = self.client.lock().await;
        let result = client.draft_email(req).await.map_err(|e| {
            error!(error = %e, "tool.draft_email failed");
            e
        })?;
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
        info!(mailbox = %req.mailbox_name, has_message_id = req.message_id.is_some(), has_subject = req.subject.is_some(), "tool.list_messages_by_thread");
        let mut client = self.client.lock().await;
        let result = client.list_messages_by_thread(req).await.map_err(|e| {
            error!(error = %e, "tool.list_messages_by_thread failed");
            e
        })?;
        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ProtonMailService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(concat!(
                "Read and organize a Proton Mail inbox via IMAP. ",
                "Read-only by default; can also archive, move, mark, and draft messages.\n",
                "\n",
                "Capabilities:\n",
                "- Browse mailboxes and list/search messages with pagination\n",
                "- Read full message content (text auto-extracted from HTML)\n",
                "- Download attachments (base64-encoded)\n",
                "- Find conversation threads by Message-ID or subject\n",
                "- Archive messages, trash messages, move between mailboxes, mark read/unread\n",
                "- Save drafts (including threaded replies) to the Drafts mailbox\n",
                "\n",
                "Cannot: send messages or manage mailboxes. Drafts are saved for the user to send manually.\n",
                "\n",
                "Key conventions:\n",
                "- Messages are identified by UID + mailbox_name (e.g. \"INBOX\")\n",
                "- Use list_mailboxes to discover available mailbox names\n",
                "- Use list_messages or search_messages to find UIDs, then get_message for full content\n",
                "- Pagination: pass the lowest UID from results as before_uid for the next page\n",
                "- Dates in search use DD-Mon-YYYY format (e.g. \"01-Jan-2024\")\n",
                "- get_message does not mark messages as read; use mark_as_read separately\n",
                "- draft_email writes to the Drafts mailbox; to reply, pass in_reply_to with the source mailbox and UID",
            ).into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "proton-mail-mcp".into(),
                title: Some("Proton Mail".into()),
                version: env!("CARGO_PKG_VERSION").into(),
                description: Some("MCP server for reading and organizing Proton Mail via IMAP through Proton Mail Bridge".into()),
                ..Default::default()
            },
            ..Default::default()
        }
    }
}
