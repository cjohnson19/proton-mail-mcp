use std::sync::Arc;

use rmcp::{
    Json, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use tokio::sync::Mutex;
use tracing::{error, info};

use crate::email_client::{
    ArchiveMessageRequest, ArchiveMessageResponse, DraftEmailRequest, DraftEmailResponse,
    EmailClient, GetAttachmentRequest, GetAttachmentResponse, GetMailboxStatusRequest,
    GetMailboxStatusResponse, GetMessageRequest, GetMessageResponse, ListMailboxesResponse,
    ListMessagesByThreadRequest, ListMessagesByThreadResponse, ListMessagesRequest,
    ListMessagesResponse, MarkAsReadRequest, MarkAsReadResponse, MessageBatchRequest,
    MessageBatchResponse, MoveMessageRequest, MoveMessageResponse, MoveMessagesRequest,
    ResolveSpecialMailboxRequest, ResolveSpecialMailboxResponse, SearchMessagesRequest,
    SearchMessagesResponse, SetFlaggedRequest, SetFlaggedResponse, TrashMessageRequest,
    TrashMessageResponse,
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
        description = "List all mailboxes in the Proton Mail account. Each entry includes a `special_use` tag (\"inbox\", \"archive\", \"sent\", \"drafts\", \"trash\", \"junk\", \"all\", \"flagged\", \"important\") when the server marks the folder with an IMAP SPECIAL-USE attribute — use that instead of pattern-matching mailbox names.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn list_mailboxes(&self) -> Result<Json<ListMailboxesResponse>, String> {
        info!("tool.list_mailboxes");
        let mut client = self.client.lock().await;
        let result = client.list_mailboxes().await.map_err(|e| {
            error!(error = %e, "tool.list_mailboxes failed");
            e
        })?;
        Ok(Json(result))
    }

    #[tool(
        title = "Resolve Special Mailbox",
        description = "Resolve a well-known folder kind (inbox, archive, sent, drafts, trash, spam, all_mail, flagged, important) to its actual IMAP mailbox name. Use this to avoid guessing folder names — Proton exposes the canonical mapping via IMAP SPECIAL-USE attributes. Returns the name you can pass to any mailbox_name/source_mailbox parameter on other tools.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn resolve_special_mailbox(
        &self,
        Parameters(req): Parameters<ResolveSpecialMailboxRequest>,
    ) -> Result<Json<ResolveSpecialMailboxResponse>, String> {
        info!(kind = ?req.kind, "tool.resolve_special_mailbox");
        let mut client = self.client.lock().await;
        let result = client.resolve_special_mailbox(req).await.map_err(|e| {
            error!(error = %e, "tool.resolve_special_mailbox failed");
            e
        })?;
        Ok(Json(result))
    }

    #[tool(
        title = "List Messages",
        description = "List messages in a mailbox, sorted by most recent first. Supports pagination via before_uid — pass the lowest UID from the previous page to get the next page.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn list_messages(
        &self,
        Parameters(req): Parameters<ListMessagesRequest>,
    ) -> Result<Json<ListMessagesResponse>, String> {
        info!(mailbox = %req.mailbox_name, limit = req.limit, before_uid = ?req.before_uid, "tool.list_messages");
        let mut client = self.client.lock().await;
        let result = client.list_messages(req).await.map_err(|e| {
            error!(error = %e, "tool.list_messages failed");
            e
        })?;
        Ok(Json(result))
    }

    #[tool(
        title = "Get Message",
        description = "Retrieve a full email message by UID, including headers, body (text and HTML), and attachment metadata. Does not mark the message as read. HTML-only emails are automatically converted to plain text for body_text. Raw HTML is omitted by default to save tokens; set include_html=true to receive it.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn get_message(
        &self,
        Parameters(req): Parameters<GetMessageRequest>,
    ) -> Result<Json<GetMessageResponse>, String> {
        info!(mailbox = %req.mailbox_name, uid = req.uid, "tool.get_message");
        let mut client = self.client.lock().await;
        let result = client.get_message(req).await.map_err(|e| {
            error!(error = %e, "tool.get_message failed");
            e
        })?;
        Ok(Json(result))
    }

    #[tool(
        title = "Search Messages",
        description = "Search for messages in a mailbox using IMAP SEARCH. Supports filtering by from, to, subject, body text, date range (DD-Mon-YYYY format), seen/unseen status, and flagged status. Multiple criteria are combined with AND. Supports pagination via before_uid — pass the next_cursor from the previous response with the same filters.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn search_messages(
        &self,
        Parameters(req): Parameters<SearchMessagesRequest>,
    ) -> Result<Json<SearchMessagesResponse>, String> {
        info!(mailbox = %req.mailbox_name, before_uid = ?req.before_uid, "tool.search_messages");
        let mut client = self.client.lock().await;
        let result = client.search_messages(req).await.map_err(|e| {
            error!(error = %e, "tool.search_messages failed");
            e
        })?;
        Ok(Json(result))
    }

    #[tool(
        title = "Get Mailbox Status",
        description = "Get lightweight status for a mailbox: message count, recent count, unseen count, UIDNEXT, and UIDVALIDITY. Useful for checking activity without fetching messages.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn get_mailbox_status(
        &self,
        Parameters(req): Parameters<GetMailboxStatusRequest>,
    ) -> Result<Json<GetMailboxStatusResponse>, String> {
        info!(mailbox = %req.mailbox_name, "tool.get_mailbox_status");
        let mut client = self.client.lock().await;
        let result = client.get_mailbox_status(req).await.map_err(|e| {
            error!(error = %e, "tool.get_mailbox_status failed");
            e
        })?;
        Ok(Json(result))
    }

    #[tool(
        title = "Get Attachment",
        description = "Download a specific attachment from a message by UID and 0-based attachment index (matching the order from get_message). Returns base64-encoded content.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn get_attachment(
        &self,
        Parameters(req): Parameters<GetAttachmentRequest>,
    ) -> Result<Json<GetAttachmentResponse>, String> {
        info!(mailbox = %req.mailbox_name, uid = req.uid, index = req.attachment_index, "tool.get_attachment");
        let mut client = self.client.lock().await;
        let result = client.get_attachment(req).await.map_err(|e| {
            error!(error = %e, "tool.get_attachment failed");
            e
        })?;
        Ok(Json(result))
    }

    #[tool(
        title = "Archive Message",
        description = "Move a message to the Archive mailbox by UID. Prefer this over move_message when the user wants to archive — it finds the canonical Archive folder automatically and is the right choice for the common 'get this out of my inbox' case. To archive several messages at once, use archive_messages instead.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn archive_message(
        &self,
        Parameters(req): Parameters<ArchiveMessageRequest>,
    ) -> Result<Json<ArchiveMessageResponse>, String> {
        info!(source = %req.source_mailbox, uid = req.uid, "tool.archive_message");
        let mut client = self.client.lock().await;
        let result = client.archive_message(req).await.map_err(|e| {
            error!(error = %e, "tool.archive_message failed");
            e
        })?;
        Ok(Json(result))
    }

    #[tool(
        title = "Trash Message",
        description = "Move a message to the Trash mailbox by UID. Use this whenever the user wants to delete a message — move_message cannot target Trash. The message is not permanently deleted; Proton will purge it according to its own retention policy. To trash several messages at once, use trash_messages instead.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn trash_message(
        &self,
        Parameters(req): Parameters<TrashMessageRequest>,
    ) -> Result<Json<TrashMessageResponse>, String> {
        info!(source = %req.source_mailbox, uid = req.uid, "tool.trash_message");
        let mut client = self.client.lock().await;
        let result = client.trash_message(req).await.map_err(|e| {
            error!(error = %e, "tool.trash_message failed");
            e
        })?;
        Ok(Json(result))
    }

    #[tool(
        title = "Archive Messages",
        description = "Move several messages from one mailbox to Archive in a single operation. Prefer this over repeated archive_message calls whenever more than one message is involved (e.g. after a search). Returns `moved` (UIDs that were archived) and `not_found` (UIDs that no longer existed in the source mailbox).",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn archive_messages(
        &self,
        Parameters(req): Parameters<MessageBatchRequest>,
    ) -> Result<Json<MessageBatchResponse>, String> {
        info!(source = %req.source_mailbox, count = req.messages.len(), "tool.archive_messages");
        let mut client = self.client.lock().await;
        let result = client.archive_messages(req).await.map_err(|e| {
            error!(error = %e, "tool.archive_messages failed");
            e
        })?;
        Ok(Json(result))
    }

    #[tool(
        title = "Trash Messages",
        description = "Move several messages from one mailbox to Trash in a single operation. Prefer this over repeated trash_message calls whenever more than one message is involved (e.g. after a search). Messages are not permanently deleted. Returns `moved` (UIDs that were trashed) and `not_found` (UIDs that no longer existed in the source mailbox).",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn trash_messages(
        &self,
        Parameters(req): Parameters<MessageBatchRequest>,
    ) -> Result<Json<MessageBatchResponse>, String> {
        info!(source = %req.source_mailbox, count = req.messages.len(), "tool.trash_messages");
        let mut client = self.client.lock().await;
        let result = client.trash_messages(req).await.map_err(|e| {
            error!(error = %e, "tool.trash_messages failed");
            e
        })?;
        Ok(Json(result))
    }

    #[tool(
        title = "Move Message",
        description = "Move a message to an arbitrary destination mailbox by UID. Use this only for custom/user-created folders — for the common cases prefer archive_message (Archive) or trash_message (Trash). Moving to Trash is blocked here. To move several messages at once, use move_messages instead.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn move_message(
        &self,
        Parameters(req): Parameters<MoveMessageRequest>,
    ) -> Result<Json<MoveMessageResponse>, String> {
        info!(source = %req.source_mailbox, destination = %req.destination_mailbox, uid = req.uid, "tool.move_message");
        let mut client = self.client.lock().await;
        let result = client.move_message(req).await.map_err(|e| {
            error!(error = %e, "tool.move_message failed");
            e
        })?;
        Ok(Json(result))
    }

    #[tool(
        title = "Move Messages",
        description = "Move several messages from one mailbox to an arbitrary destination mailbox in a single operation. Use this for custom/user-created folders — for Archive or Trash prefer archive_messages / trash_messages. Moving to Trash is blocked here. Returns `moved` (UIDs that were moved) and `not_found` (UIDs that no longer existed in the source mailbox).",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn move_messages(
        &self,
        Parameters(req): Parameters<MoveMessagesRequest>,
    ) -> Result<Json<MessageBatchResponse>, String> {
        info!(source = %req.source_mailbox, destination = %req.destination_mailbox, count = req.messages.len(), "tool.move_messages");
        let mut client = self.client.lock().await;
        let result = client.move_messages(req).await.map_err(|e| {
            error!(error = %e, "tool.move_messages failed");
            e
        })?;
        Ok(Json(result))
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
    ) -> Result<Json<MarkAsReadResponse>, String> {
        info!(source = %req.source_mailbox, uid = req.uid, read = req.read, "tool.mark_as_read");
        let mut client = self.client.lock().await;
        let result = client.mark_as_read(req).await.map_err(|e| {
            error!(error = %e, "tool.mark_as_read failed");
            e
        })?;
        Ok(Json(result))
    }

    #[tool(
        title = "Set Flagged",
        description = "Star or unstar a message by UID. Defaults to starring the message; set flagged=false to remove the star.",
        annotations(
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_flagged(
        &self,
        Parameters(req): Parameters<SetFlaggedRequest>,
    ) -> Result<Json<SetFlaggedResponse>, String> {
        info!(source = %req.source_mailbox, uid = req.uid, flagged = req.flagged, "tool.set_flagged");
        let mut client = self.client.lock().await;
        let result = client.set_flagged(req).await.map_err(|e| {
            error!(error = %e, "tool.set_flagged failed");
            e
        })?;
        Ok(Json(result))
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
    ) -> Result<Json<DraftEmailResponse>, String> {
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
        Ok(Json(result))
    }

    #[tool(
        title = "List Messages by Thread",
        description = "Find related messages in a thread by UID, Message-ID, or subject. When given a UID, resolves the message's Message-ID, References, and In-Reply-To headers automatically. Searches thread headers to reconstruct the conversation. At least one of uid, message_id, or subject must be provided. Note: search is limited to a single mailbox.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn list_messages_by_thread(
        &self,
        Parameters(req): Parameters<ListMessagesByThreadRequest>,
    ) -> Result<Json<ListMessagesByThreadResponse>, String> {
        info!(mailbox = %req.mailbox_name, uid = ?req.uid, has_message_id = req.message_id.is_some(), has_subject = req.subject.is_some(), "tool.list_messages_by_thread");
        let mut client = self.client.lock().await;
        let result = client.list_messages_by_thread(req).await.map_err(|e| {
            error!(error = %e, "tool.list_messages_by_thread failed");
            e
        })?;
        Ok(Json(result))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ProtonMailService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(concat!(
                "Read and organize a Proton Mail inbox via IMAP. ",
                "Read-only by default; can also archive, move, mark, and draft messages.\n",
                "\n",
                "Capabilities:\n",
                "- Browse mailboxes and list/search messages with pagination\n",
                "- Read full message content (text auto-extracted from HTML)\n",
                "- Download attachments (base64-encoded)\n",
                "- Find conversation threads by UID, Message-ID, or subject\n",
                "- Archive, trash, or move messages (one at a time, or several per call with archive_messages/trash_messages/move_messages), mark read/unread, star/unstar\n",
                "- Save drafts (including threaded replies) to the Drafts mailbox\n",
                "\n",
                "Cannot: send messages or manage mailboxes. Drafts are saved for the user to send manually.\n",
                "\n",
                "Key conventions:\n",
                "- Messages are identified by UID + a mailbox name (e.g. \"INBOX\"). Read-only tools call the parameter `mailbox_name`; mutating tools (archive/trash/move/mark_as_read/set_flagged) call it `source_mailbox`.\n",
                "- Mailbox names are case-sensitive and server-defined. If you don't already know the exact name, call list_mailboxes first — use the `special_use` field on each entry (\"inbox\", \"archive\", \"sent\", \"drafts\", \"trash\", \"junk\", \"all\") to find canonical folders instead of string-matching on names.\n",
                "- UIDs are stable only within a mailbox + uid_validity pair. Don't reuse a UID across mailboxes.\n",
                "- Use list_messages or search_messages to find UIDs, then get_message for full content. Prefer search_messages with specific filters over list_messages + scanning when you know what you're looking for.\n",
                "- Pagination: list_messages and search_messages return a `next_cursor` field — pass it as `before_uid` on the next call. `None` means end of results.\n",
                "- Dates in search use DD-Mon-YYYY format (e.g. \"01-Jan-2024\"). `since` is inclusive, `before` is exclusive.\n",
                "- get_message does not mark messages as read; use mark_as_read separately.\n",
                "- draft_email writes to the Drafts mailbox; to reply, pass in_reply_to with the source mailbox and UID. Sending is not supported.",
            ))
            .with_server_info(
                Implementation::new("proton-mail-mcp", env!("CARGO_PKG_VERSION"))
                    .with_title("Proton Mail")
                    .with_description("MCP server for reading and organizing Proton Mail via IMAP through Proton Mail Bridge"),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::handler::server::tool::IntoCallToolResult;
    use rmcp::model::CallToolResponse;
    use serde_json::{Value, json};

    #[test]
    fn every_tool_advertises_an_output_schema() {
        let tools = ProtonMailService::tool_router().list_all();
        assert_eq!(tools.len(), 17);
        let missing: Vec<_> = tools
            .iter()
            .filter(|tool| tool.output_schema.is_none())
            .map(|tool| tool.name.as_ref())
            .collect();
        assert!(
            missing.is_empty(),
            "tools missing output schemas: {missing:?}"
        );
    }

    #[test]
    fn output_schemas_describe_each_tools_result_fields() {
        let tools = ProtonMailService::tool_router().list_all();
        let expected = [
            ("list_mailboxes", "array", vec![]),
            ("resolve_special_mailbox", "object", vec!["kind", "name"]),
            (
                "list_messages",
                "object",
                vec!["messages", "total_in_mailbox", "next_cursor"],
            ),
            (
                "get_message",
                "object",
                vec!["uid", "body_text", "body_html", "attachments"],
            ),
            (
                "search_messages",
                "object",
                vec!["messages", "total_matches", "next_cursor"],
            ),
            (
                "get_mailbox_status",
                "object",
                vec!["mailbox_name", "messages", "unseen", "uid_validity"],
            ),
            (
                "get_attachment",
                "object",
                vec!["uid", "attachment_index", "content_base64"],
            ),
            (
                "archive_message",
                "object",
                vec!["uid", "source_mailbox", "destination_mailbox"],
            ),
            ("trash_message", "object", vec!["uid", "source_mailbox"]),
            (
                "archive_messages",
                "object",
                vec![
                    "source_mailbox",
                    "destination_mailbox",
                    "moved",
                    "not_found",
                ],
            ),
            (
                "trash_messages",
                "object",
                vec![
                    "source_mailbox",
                    "destination_mailbox",
                    "moved",
                    "not_found",
                ],
            ),
            (
                "move_message",
                "object",
                vec!["uid", "source_mailbox", "destination_mailbox"],
            ),
            (
                "move_messages",
                "object",
                vec![
                    "source_mailbox",
                    "destination_mailbox",
                    "moved",
                    "not_found",
                ],
            ),
            (
                "mark_as_read",
                "object",
                vec!["uid", "source_mailbox", "read"],
            ),
            (
                "set_flagged",
                "object",
                vec!["uid", "source_mailbox", "flagged"],
            ),
            (
                "draft_email",
                "object",
                vec!["drafts_mailbox", "message_id", "threaded"],
            ),
            (
                "list_messages_by_thread",
                "object",
                vec!["messages", "total_in_thread"],
            ),
        ];
        expected.iter().for_each(|(name, kind, fields)| {
            let tool = tools.iter().find(|tool| tool.name == *name).unwrap();
            let wire = serde_json::to_value(tool).unwrap();
            let schema = &wire["outputSchema"];
            assert_eq!(schema["type"], *kind, "{name}");
            fields.iter().for_each(|field| {
                assert!(schema["properties"].get(field).is_some(), "{name}.{field}");
            });
            if *kind == "array" {
                assert!(schema.get("items").is_some(), "{name}");
            }
        });
    }

    fn assert_structured_result<T: serde::Serialize + schemars::JsonSchema + 'static>(value: T) {
        let expected = serde_json::to_value(&value).unwrap();
        let response: Result<Json<T>, String> = Ok(Json(value));
        let CallToolResponse::Complete(result) = response.into_call_tool_result().unwrap() else {
            panic!("expected a complete result");
        };
        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.structured_content, Some(expected.clone()));
        let text = result.content[0].as_text().unwrap();
        assert_eq!(serde_json::from_str::<Value>(&text.text).unwrap(), expected);
    }

    #[test]
    fn structured_results_preserve_objects_arrays_and_text_fallback() {
        assert_structured_result(ArchiveMessageResponse {
            uid: 42,
            source_mailbox: "INBOX".into(),
            destination_mailbox: "Archive".into(),
        });
        assert_structured_result(ListMessagesResponse {
            messages: vec![],
            total_in_mailbox: 0,
            next_cursor: None,
        });
        let mailboxes: ListMailboxesResponse = serde_json::from_value(json!([{
            "name": "INBOX", "attributes": [], "delimiter": "/", "special_use": "inbox"
        }]))
        .unwrap();
        assert_structured_result(mailboxes);
    }

    #[test]
    fn tool_errors_remain_unstructured_error_results() {
        let response: Result<Json<ArchiveMessageResponse>, String> =
            Err("message not found".into());
        let CallToolResponse::Complete(result) = response.into_call_tool_result().unwrap() else {
            panic!("expected a complete result");
        };
        assert_eq!(result.is_error, Some(true));
        assert!(result.structured_content.is_none());
        assert_eq!(
            result.content[0].as_text().unwrap().text,
            "message not found"
        );
    }
}
