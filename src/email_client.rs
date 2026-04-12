use std::collections::HashSet;

use async_imap::{
    Session,
    types::{Fetch, Flag, Name, NameAttribute},
};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use futures::TryStreamExt;
use mailparse::{MailAddr, MailHeaderMap, addrparse_header};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio_native_tls::TlsStream;
use tracing::{debug, instrument, warn};

const FETCH_HEADERS: &str = "(UID FLAGS BODY.PEEK[HEADER])";

#[derive(Debug)]
pub struct EmailClient {
    session: Session<TlsStream<TcpStream>>,
}

#[allow(dead_code)]
#[derive(Debug, JsonSchema)]
pub struct ListMailboxesRequest;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct MailboxDescription {
    /// Mailbox name
    name: String,
    /// Mailbox attributes (e.g. NoSelect, NoInferiors)
    #[schemars(extend("examples" = ["NoSelect", "NoInferiors"]))]
    attributes: Vec<MailboxAttribute>,
    /// Hierarchy delimiter character
    delimiter: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub enum MailboxAttribute {
    /// It is not possible for any child levels of hierarchy to exist
    /// under this name; no child levels exist now and none can be
    /// created in the future.
    NoInferiors,

    /// It is not possible to use this name as a selectable mailbox.
    NoSelect,

    /// The mailbox has been marked "interesting" by the server; the
    /// mailbox probably contains messages that have been added since
    /// the last time the mailbox was selected.
    Marked,

    /// The mailbox does not contain any additional messages since the
    /// last time the mailbox was selected.
    Unmarked,

    /// A non-standard user- or server-defined name attribute.
    Custom(String),
}

impl From<NameAttribute<'_>> for MailboxAttribute {
    fn from(value: NameAttribute) -> Self {
        match value {
            NameAttribute::NoInferiors => Self::NoInferiors,
            NameAttribute::NoSelect => Self::NoSelect,
            NameAttribute::Marked => Self::Marked,
            NameAttribute::Unmarked => Self::Unmarked,
            NameAttribute::Extension(cow) => Self::Custom(cow.to_string()),
            other => Self::Custom(format!("{other:?}")),
        }
    }
}

impl From<&Name> for MailboxDescription {
    fn from(value: &Name) -> Self {
        Self {
            name: value.name().to_owned(),
            attributes: value
                .attributes()
                .iter()
                .cloned()
                .map(MailboxAttribute::from)
                .collect(),
            delimiter: value.delimiter().map(|v| v.to_owned()),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListMailboxesResponse(Vec<MailboxDescription>);

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub enum MessageFlag {
    Seen,
    Answered,
    Flagged,
    Deleted,
    Draft,
    Recent,
    MayCreate,
    Custom(String),
}

impl From<&Flag<'_>> for MessageFlag {
    fn from(value: &Flag<'_>) -> Self {
        match value {
            Flag::Seen => Self::Seen,
            Flag::Answered => Self::Answered,
            Flag::Flagged => Self::Flagged,
            Flag::Deleted => Self::Deleted,
            Flag::Draft => Self::Draft,
            Flag::Recent => Self::Recent,
            Flag::MayCreate => Self::MayCreate,
            Flag::Custom(cow) => Self::Custom(cow.to_string()),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct MessageSummary {
    /// Unique identifier of the message
    pub uid: u32,
    /// Subject line of the message
    pub subject: Option<String>,
    /// Sender addresses
    pub from: Vec<String>,
    /// Date the message was sent
    pub date: Option<String>,
    /// Message flags (e.g. Seen, Flagged)
    #[schemars(extend("examples" = ["Seen", "Flagged"]))]
    pub flags: Vec<MessageFlag>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AttachmentInfo {
    /// Filename of the attachment, if available
    pub filename: Option<String>,
    /// MIME type of the attachment
    pub mime_type: String,
    /// Size of the attachment in bytes
    pub size_bytes: usize,
}

fn default_limit() -> u32 {
    25
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListMessagesRequest {
    /// Name of the mailbox to list messages from (e.g. "INBOX")
    #[schemars(length(min = 1), example = &"INBOX")]
    pub mailbox_name: String,
    /// Maximum number of messages to return (default 25, max 100)
    #[serde(default = "default_limit")]
    #[schemars(range(min = 0, max = 100))]
    pub limit: u32,
    /// Return messages with UID less than this value (for pagination)
    #[schemars(range(min = 1))]
    pub before_uid: Option<u32>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListMessagesResponse {
    /// List of message summaries
    pub messages: Vec<MessageSummary>,
    /// Total number of messages in the mailbox
    pub total_in_mailbox: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetMessageRequest {
    /// Name of the mailbox containing the message
    #[schemars(length(min = 1), example = &"INBOX")]
    pub mailbox_name: String,
    /// UID of the message to retrieve
    #[schemars(range(min = 1))]
    pub uid: u32,
    /// Whether to include the raw HTML body in the response (default: false)
    #[serde(default)]
    pub include_html: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetMessageResponse {
    /// Unique identifier of the message
    pub uid: u32,
    /// Subject line
    pub subject: Option<String>,
    /// Sender addresses
    pub from: Vec<String>,
    /// Recipient addresses
    pub to: Vec<String>,
    /// CC addresses
    pub cc: Vec<String>,
    /// Date the message was sent
    pub date: Option<String>,
    /// Message flags
    pub flags: Vec<MessageFlag>,
    /// Plain text body (HTML is auto-converted if no text part exists)
    pub body_text: Option<String>,
    /// Raw HTML body (only present when include_html is true)
    pub body_html: Option<String>,
    /// List of attachments
    pub attachments: Vec<AttachmentInfo>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchMessagesRequest {
    /// Name of the mailbox to search
    #[schemars(length(min = 1), example = &"INBOX")]
    pub mailbox_name: String,
    /// Filter by sender
    pub from: Option<String>,
    /// Filter by recipient
    pub to: Option<String>,
    /// Filter by subject
    pub subject: Option<String>,
    /// Search message body text
    pub body: Option<String>,
    /// Messages since this date (DD-Mon-YYYY format, e.g. "01-Jan-2024")
    #[schemars(example = &"01-Jan-2024")]
    pub since: Option<String>,
    /// Messages before this date (DD-Mon-YYYY format, e.g. "31-Dec-2024")
    #[schemars(example = &"31-Dec-2024")]
    pub before: Option<String>,
    /// Filter by seen/unseen status
    pub seen: Option<bool>,
    /// Filter by flagged status
    pub flagged: Option<bool>,
    /// Maximum number of results (default 25, max 100)
    #[schemars(range(min = 1, max = 100))]
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchMessagesResponse {
    /// List of matching message summaries
    pub messages: Vec<MessageSummary>,
    /// Total number of messages matching the search criteria
    pub total_matches: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetMailboxStatusRequest {
    /// Name of the mailbox (e.g. "INBOX")
    #[schemars(length(min = 1), example = &"INBOX")]
    pub mailbox_name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetMailboxStatusResponse {
    /// Name of the mailbox
    pub mailbox_name: String,
    /// Total number of messages
    pub messages: u32,
    /// Number of recent messages
    pub recent: u32,
    /// Number of unseen messages
    pub unseen: Option<u32>,
    /// Next UID value to be assigned
    pub uid_next: Option<u32>,
    /// UID validity value
    pub uid_validity: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetAttachmentRequest {
    /// Name of the mailbox containing the message
    #[schemars(length(min = 1), example = &"INBOX")]
    pub mailbox_name: String,
    /// UID of the message
    #[schemars(range(min = 1))]
    pub uid: u32,
    /// 0-based attachment index (matches order from get_message)
    pub attachment_index: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetAttachmentResponse {
    /// UID of the message containing the attachment
    pub uid: u32,
    /// Index of the attachment in the message
    pub attachment_index: usize,
    /// Filename of the attachment, if available
    pub filename: Option<String>,
    /// MIME type of the attachment
    pub mime_type: String,
    /// Size of the attachment in bytes
    pub size_bytes: usize,
    /// Base64-encoded attachment content
    pub content_base64: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListMessagesByThreadRequest {
    /// Name of the mailbox to search
    #[schemars(length(min = 1), example = &"INBOX")]
    pub mailbox_name: String,
    /// Message-ID header value to find the thread for (at least one of message_id or subject is required)
    pub message_id: Option<String>,
    /// Subject to search for (at least one of message_id or subject is required)
    pub subject: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ThreadMessage {
    /// Unique identifier of the message
    pub uid: u32,
    /// Message-ID header value
    pub message_id: Option<String>,
    /// In-Reply-To header value
    pub in_reply_to: Option<String>,
    /// References header value
    pub references: Option<String>,
    /// Subject line
    pub subject: Option<String>,
    /// Sender addresses
    pub from: Vec<String>,
    /// Date the message was sent
    pub date: Option<String>,
    /// Message flags
    pub flags: Vec<MessageFlag>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListMessagesByThreadResponse {
    /// Messages in the thread, sorted by UID
    pub messages: Vec<ThreadMessage>,
    /// Total number of messages in the thread
    pub total_in_thread: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ArchiveMessageRequest {
    /// Name of the mailbox containing the message (e.g. "INBOX")
    #[schemars(length(min = 1), example = &"INBOX")]
    pub mailbox_name: String,
    /// UID of the message to archive
    #[schemars(range(min = 1))]
    pub uid: u32,
    /// Subject of the message (not used for the operation, but shown during tool approval for context)
    #[allow(dead_code)]
    #[serde(default)]
    pub subject: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ArchiveMessageResponse {
    /// UID of the archived message
    pub uid: u32,
    /// Original mailbox the message was in
    pub source_mailbox: String,
    /// Destination mailbox (Archive)
    pub destination_mailbox: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TrashMessageRequest {
    /// Name of the mailbox containing the message (e.g. "INBOX")
    #[schemars(length(min = 1), example = &"INBOX")]
    pub source_mailbox: String,
    /// UID of the message to trash
    #[schemars(range(min = 1))]
    pub uid: u32,
    /// Subject of the message (not used for the operation, but shown during tool approval for context)
    #[allow(dead_code)]
    #[serde(default)]
    pub subject: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TrashMessageResponse {
    /// UID of the trashed message
    pub uid: u32,
    /// Original mailbox the message was in
    pub source_mailbox: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveMessageRequest {
    /// Name of the mailbox containing the message
    #[schemars(length(min = 1), example = &"INBOX")]
    pub source_mailbox: String,
    /// Destination mailbox to move the message to (cannot be "Trash" — use trash_message instead)
    #[schemars(length(min = 1), example = &"Archive")]
    pub destination_mailbox: String,
    /// UID of the message to move
    #[schemars(range(min = 1))]
    pub uid: u32,
    /// Subject of the message (not used for the operation, but shown during tool approval for context)
    #[allow(dead_code)]
    #[serde(default)]
    pub subject: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MoveMessageResponse {
    /// UID of the moved message
    pub uid: u32,
    /// Original mailbox
    pub source_mailbox: String,
    /// Destination mailbox
    pub destination_mailbox: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MarkAsReadRequest {
    /// Name of the mailbox containing the message (e.g. "INBOX")
    #[schemars(length(min = 1), example = &"INBOX")]
    pub mailbox_name: String,
    /// UID of the message to mark
    #[schemars(range(min = 1))]
    pub uid: u32,
    /// Read status of the message. Set to false to mark as unread instead
    #[serde(default = "default_true")]
    pub read: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MarkAsReadResponse {
    /// UID of the message
    pub uid: u32,
    /// Mailbox the message is in
    pub mailbox_name: String,
    /// Whether the message is now marked as read
    pub read: bool,
}

fn format_addr(info: &mailparse::SingleInfo) -> String {
    match info.display_name {
        Some(ref name) if !name.is_empty() => format!("{name} <{}>", info.addr),
        _ => info.addr.clone(),
    }
}

fn parse_address_header(header: &mailparse::MailHeader) -> Vec<String> {
    addrparse_header(header)
        .map(|addrs| {
            addrs
                .iter()
                .flat_map(|addr| match addr {
                    MailAddr::Single(info) => vec![format_addr(info)],
                    MailAddr::Group(group) => group.addrs.iter().map(format_addr).collect(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_summaries(fetches: &[Fetch]) -> Vec<MessageSummary> {
    fetches
        .iter()
        .filter_map(|fetch| {
            let uid = fetch.uid?;
            let (headers, _) = mailparse::parse_headers(fetch.header()?).ok()?;

            let subject = headers.get_first_value("Subject");
            let date = headers.get_first_value("Date");
            let from = headers
                .get_first_header("From")
                .map(parse_address_header)
                .unwrap_or_default();
            let flags = fetch.flags().map(|f| MessageFlag::from(&f)).collect();

            Some(MessageSummary {
                uid,
                subject,
                from,
                date,
                flags,
            })
        })
        .collect()
}

fn extract_body_parts(mail: &mailparse::ParsedMail) -> (Option<String>, Option<String>) {
    if mail.subparts.is_empty() {
        let content_type = &mail.ctype.mimetype;
        match content_type.as_str() {
            "text/plain" => (mail.get_body().ok(), None),
            "text/html" => (None, mail.get_body().ok()),
            _ => (None, None),
        }
    } else {
        let mut text = None;
        let mut html = None;
        for part in &mail.subparts {
            let (t, h) = extract_body_parts(part);
            if text.is_none() {
                text = t;
            }
            if html.is_none() {
                html = h;
            }
        }
        (text, html)
    }
}

fn collect_attachments(mail: &mailparse::ParsedMail, attachments: &mut Vec<AttachmentInfo>) {
    let disposition = mail.get_content_disposition();
    let is_attachment = disposition.disposition == mailparse::DispositionType::Attachment
        || (disposition.disposition != mailparse::DispositionType::Inline
            && !mail.ctype.mimetype.starts_with("text/")
            && !mail.ctype.mimetype.starts_with("multipart/")
            && mail.subparts.is_empty());

    if is_attachment {
        let filename = disposition
            .params
            .get("filename")
            .or_else(|| mail.ctype.params.get("name"))
            .cloned();
        let size = mail.get_body_raw().map(|b| b.len()).unwrap_or(0);
        attachments.push(AttachmentInfo {
            filename,
            mime_type: mail.ctype.mimetype.clone(),
            size_bytes: size,
        });
    }

    for subpart in &mail.subparts {
        collect_attachments(subpart, attachments);
    }
}

fn collect_attachment_parts<'a>(
    mail: &'a mailparse::ParsedMail<'a>,
    parts: &mut Vec<&'a mailparse::ParsedMail<'a>>,
) {
    let disposition = mail.get_content_disposition();
    let is_attachment = disposition.disposition == mailparse::DispositionType::Attachment
        || (disposition.disposition != mailparse::DispositionType::Inline
            && !mail.ctype.mimetype.starts_with("text/")
            && !mail.ctype.mimetype.starts_with("multipart/")
            && mail.subparts.is_empty());

    if is_attachment {
        parts.push(mail);
    }

    for subpart in &mail.subparts {
        collect_attachment_parts(subpart, parts);
    }
}

fn extract_attachment_by_index(
    raw: &[u8],
    index: usize,
) -> Result<(Option<String>, String, Vec<u8>), String> {
    let parsed = mailparse::parse_mail(raw).map_err(|e| e.to_string())?;
    let mut parts = Vec::new();
    collect_attachment_parts(&parsed, &mut parts);

    let part = parts.get(index).ok_or_else(|| {
        format!(
            "Attachment index {index} out of range (message has {} attachments)",
            parts.len()
        )
    })?;

    let disposition = part.get_content_disposition();
    let filename = disposition
        .params
        .get("filename")
        .or_else(|| part.ctype.params.get("name"))
        .cloned();
    let mime_type = part.ctype.mimetype.clone();
    let body = part.get_body_raw().map_err(|e| e.to_string())?;

    Ok((filename, mime_type, body))
}

fn escape_search_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn build_search_query(req: &SearchMessagesRequest) -> String {
    let mut criteria = Vec::new();

    if let Some(ref from) = req.from {
        criteria.push(format!("FROM \"{}\"", escape_search_string(from)));
    }
    if let Some(ref to) = req.to {
        criteria.push(format!("TO \"{}\"", escape_search_string(to)));
    }
    if let Some(ref subject) = req.subject {
        criteria.push(format!("SUBJECT \"{}\"", escape_search_string(subject)));
    }
    if let Some(ref body) = req.body {
        criteria.push(format!("BODY \"{}\"", escape_search_string(body)));
    }
    if let Some(ref since) = req.since {
        criteria.push(format!("SINCE {since}"));
    }
    if let Some(ref before) = req.before {
        criteria.push(format!("BEFORE {before}"));
    }
    if let Some(seen) = req.seen {
        criteria.push(if seen { "SEEN".into() } else { "UNSEEN".into() });
    }
    if let Some(flagged) = req.flagged {
        criteria.push(if flagged {
            "FLAGGED".into()
        } else {
            "UNFLAGGED".into()
        });
    }

    if criteria.is_empty() {
        "ALL".into()
    } else {
        criteria.join(" ")
    }
}

impl EmailClient {
    pub async fn new(
        domain: &str,
        port: u16,
        username: &str,
        password: &str,
    ) -> Result<Self, String> {
        debug!(domain, port, "connecting to IMAP server");

        let tcp_stream = TcpStream::connect((domain, port)).await.map_err(|e| {
            warn!(error = %e, "TCP connection failed");
            e.to_string()
        })?;

        let mut client = async_imap::Client::new(tcp_stream);
        client
            .read_response()
            .await
            .map_err(|e| e.to_string())?
            .ok_or("no server greeting")?;
        client
            .run_command_and_check_ok("STARTTLS", None)
            .await
            .map_err(|e| {
                warn!(error = %e, "STARTTLS command failed");
                e.to_string()
            })?;

        let tcp_stream = client.into_inner();

        let tls_connector = native_tls::TlsConnector::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .map_err(|e| e.to_string())?;
        let tls_connector = tokio_native_tls::TlsConnector::from(tls_connector);
        let tls_stream: TlsStream<TcpStream> = tls_connector
            .connect(domain, tcp_stream)
            .await
            .map_err(|e| {
                warn!(error = %e, "TLS upgrade failed");
                e.to_string()
            })?;

        let client = async_imap::Client::new(tls_stream);
        let imap_session = client.login(username, password).await.map_err(|e| {
            warn!(error = %e.0, "IMAP login failed");
            e.0.to_string()
        })?;
        debug!("IMAP session established");
        Ok(Self {
            session: imap_session,
        })
    }

    /// Connect via IMAPS (direct TLS, no STARTTLS). Used for servers that
    /// provide TLS on connect (e.g. port 993 / GreenMail port 3993).
    pub async fn new_direct_tls(
        domain: &str,
        port: u16,
        username: &str,
        password: &str,
    ) -> Result<Self, String> {
        debug!(domain, port, "connecting to IMAPS server (direct TLS)");

        let tcp_stream = TcpStream::connect((domain, port)).await.map_err(|e| {
            warn!(error = %e, "TCP connection failed");
            e.to_string()
        })?;

        let tls_connector = native_tls::TlsConnector::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .map_err(|e| e.to_string())?;
        let tls_connector = tokio_native_tls::TlsConnector::from(tls_connector);
        let tls_stream: TlsStream<TcpStream> = tls_connector
            .connect(domain, tcp_stream)
            .await
            .map_err(|e| {
                warn!(error = %e, "TLS connection failed");
                e.to_string()
            })?;

        let client = async_imap::Client::new(tls_stream);
        let imap_session = client.login(username, password).await.map_err(|e| {
            warn!(error = %e.0, "IMAP login failed");
            e.0.to_string()
        })?;
        debug!("IMAPS session established");
        Ok(Self {
            session: imap_session,
        })
    }

    #[instrument(skip(self))]
    pub async fn list_mailboxes(&mut self) -> Result<ListMailboxesResponse, String> {
        let names: Vec<Name> = self
            .session
            .list(None, Some("*"))
            .await
            .map_err(|e| e.to_string())?
            .try_collect()
            .await
            .map_err(|e| e.to_string())?;
        let mailboxes: Vec<MailboxDescription> =
            names.iter().map(MailboxDescription::from).collect();
        debug!(count = mailboxes.len(), "listed mailboxes");
        Ok(ListMailboxesResponse(mailboxes))
    }

    #[instrument(skip(self), fields(mailbox = %req.mailbox_name, limit = req.limit, before_uid = ?req.before_uid))]
    pub async fn list_messages(
        &mut self,
        req: ListMessagesRequest,
    ) -> Result<ListMessagesResponse, String> {
        let limit = req.limit.min(100);
        let mailbox = self
            .session
            .examine(&req.mailbox_name)
            .await
            .map_err(|e| e.to_string())?;
        let total = mailbox.exists;
        debug!(total, "examined mailbox");

        if total == 0 {
            return Ok(ListMessagesResponse {
                messages: Vec::new(),
                total_in_mailbox: total,
            });
        }

        let fetches: Vec<Fetch> = match req.before_uid {
            Some(before_uid) => {
                if before_uid <= 1 {
                    return Ok(ListMessagesResponse {
                        messages: Vec::new(),
                        total_in_mailbox: total,
                    });
                }
                let uids = self
                    .session
                    .uid_search(format!("UID 1:{}", before_uid - 1))
                    .await
                    .map_err(|e| e.to_string())?;

                if uids.is_empty() {
                    return Ok(ListMessagesResponse {
                        messages: Vec::new(),
                        total_in_mailbox: total,
                    });
                }

                let mut uid_vec: Vec<u32> = uids.into_iter().collect();
                uid_vec.sort_unstable_by(|a, b| b.cmp(a));
                uid_vec.truncate(limit as usize);

                let uid_set = uid_vec
                    .iter()
                    .map(|u| u.to_string())
                    .collect::<Vec<_>>()
                    .join(",");

                self.session
                    .uid_fetch(uid_set, FETCH_HEADERS)
                    .await
                    .map_err(|e| e.to_string())?
                    .try_collect()
                    .await
                    .map_err(|e| e.to_string())?
            }
            None => {
                let start = total.saturating_sub(limit).saturating_add(1);
                self.session
                    .fetch(format!("{start}:{total}"), FETCH_HEADERS)
                    .await
                    .map_err(|e| e.to_string())?
                    .try_collect()
                    .await
                    .map_err(|e| e.to_string())?
            }
        };

        let mut messages = extract_summaries(&fetches);
        messages.sort_by_key(|a| std::cmp::Reverse(a.uid));
        debug!(returned = messages.len(), "fetched messages");

        Ok(ListMessagesResponse {
            messages,
            total_in_mailbox: total,
        })
    }

    #[instrument(skip(self), fields(mailbox = %req.mailbox_name, uid = req.uid))]
    pub async fn get_message(
        &mut self,
        req: GetMessageRequest,
    ) -> Result<GetMessageResponse, String> {
        self.session
            .examine(&req.mailbox_name)
            .await
            .map_err(|e| e.to_string())?;

        let fetches: Vec<Fetch> = self
            .session
            .uid_fetch(req.uid.to_string(), "(UID FLAGS BODY.PEEK[])")
            .await
            .map_err(|e| e.to_string())?
            .try_collect()
            .await
            .map_err(|e| e.to_string())?;

        let fetch = fetches
            .first()
            .ok_or_else(|| format!("Message with UID {} not found", req.uid))?;

        let raw = fetch.body().ok_or("Message has no body")?;
        let parsed = mailparse::parse_mail(raw).map_err(|e| e.to_string())?;

        let subject = parsed.headers.get_first_value("Subject");
        let date = parsed.headers.get_first_value("Date");
        let from = parsed
            .headers
            .get_first_header("From")
            .map(parse_address_header)
            .unwrap_or_default();
        let to = parsed
            .headers
            .get_first_header("To")
            .map(parse_address_header)
            .unwrap_or_default();
        let cc = parsed
            .headers
            .get_first_header("Cc")
            .map(parse_address_header)
            .unwrap_or_default();
        let flags = fetch.flags().map(|f| MessageFlag::from(&f)).collect();

        let (mut body_text, body_html) = extract_body_parts(&parsed);
        let mut attachments = Vec::new();
        collect_attachments(&parsed, &mut attachments);

        if body_text.is_none()
            && let Some(ref html) = body_html
        {
            body_text = html2text::from_read(html.as_bytes(), 80).ok();
        }

        debug!(
            has_text = body_text.is_some(),
            has_html = body_html.is_some(),
            attachment_count = attachments.len(),
            "fetched message"
        );

        Ok(GetMessageResponse {
            uid: req.uid,
            subject,
            from,
            to,
            cc,
            date,
            flags,
            body_text,
            body_html: if req.include_html { body_html } else { None },
            attachments,
        })
    }

    #[instrument(skip(self), fields(mailbox = %req.mailbox_name))]
    pub async fn search_messages(
        &mut self,
        req: SearchMessagesRequest,
    ) -> Result<SearchMessagesResponse, String> {
        let limit = req.limit.unwrap_or(25).min(100);
        self.session
            .examine(&req.mailbox_name)
            .await
            .map_err(|e| e.to_string())?;

        let query = build_search_query(&req);
        debug!(query = %query, "executing IMAP search");
        let uids = self
            .session
            .uid_search(&query)
            .await
            .map_err(|e| e.to_string())?;
        let total_matches = uids.len() as u32;
        debug!(total_matches, "search complete");

        if uids.is_empty() {
            return Ok(SearchMessagesResponse {
                messages: Vec::new(),
                total_matches: 0,
            });
        }

        let mut uid_vec: Vec<u32> = uids.into_iter().collect();
        uid_vec.sort_unstable_by(|a, b| b.cmp(a));
        uid_vec.truncate(limit as usize);

        let uid_set = uid_vec
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");

        let fetches: Vec<Fetch> = self
            .session
            .uid_fetch(uid_set, FETCH_HEADERS)
            .await
            .map_err(|e| e.to_string())?
            .try_collect()
            .await
            .map_err(|e| e.to_string())?;

        let mut messages = extract_summaries(&fetches);
        messages.sort_by_key(|a| std::cmp::Reverse(a.uid));

        Ok(SearchMessagesResponse {
            messages,
            total_matches,
        })
    }

    #[instrument(skip(self), fields(mailbox = %req.mailbox_name))]
    pub async fn get_mailbox_status(
        &mut self,
        req: GetMailboxStatusRequest,
    ) -> Result<GetMailboxStatusResponse, String> {
        let mailbox = self
            .session
            .examine(&req.mailbox_name)
            .await
            .map_err(|e| e.to_string())?;

        let unseen = self
            .session
            .uid_search("UNSEEN")
            .await
            .map(|uids| uids.len() as u32)
            .ok();

        debug!(
            messages = mailbox.exists,
            recent = mailbox.recent,
            "mailbox status"
        );

        Ok(GetMailboxStatusResponse {
            mailbox_name: req.mailbox_name,
            messages: mailbox.exists,
            recent: mailbox.recent,
            unseen,
            uid_next: mailbox.uid_next,
            uid_validity: mailbox.uid_validity,
        })
    }

    #[instrument(skip(self), fields(mailbox = %req.mailbox_name, uid = req.uid, index = req.attachment_index))]
    pub async fn get_attachment(
        &mut self,
        req: GetAttachmentRequest,
    ) -> Result<GetAttachmentResponse, String> {
        self.session
            .examine(&req.mailbox_name)
            .await
            .map_err(|e| e.to_string())?;

        let fetches: Vec<Fetch> = self
            .session
            .uid_fetch(req.uid.to_string(), "BODY.PEEK[]")
            .await
            .map_err(|e| e.to_string())?
            .try_collect()
            .await
            .map_err(|e| e.to_string())?;

        let fetch = fetches
            .first()
            .ok_or_else(|| format!("Message with UID {} not found", req.uid))?;

        let raw = fetch.body().ok_or("Message has no body")?;

        let (filename, mime_type, body) = extract_attachment_by_index(raw, req.attachment_index)?;
        let size_bytes = body.len();
        let content_base64 = BASE64.encode(&body);
        debug!(mime_type = %mime_type, size_bytes, "fetched attachment");

        Ok(GetAttachmentResponse {
            uid: req.uid,
            attachment_index: req.attachment_index,
            filename,
            mime_type,
            size_bytes,
            content_base64,
        })
    }

    #[instrument(skip(self), fields(mailbox = %req.mailbox_name, uid = req.uid))]
    pub async fn archive_message(
        &mut self,
        req: ArchiveMessageRequest,
    ) -> Result<ArchiveMessageResponse, String> {
        self.session
            .select(&req.mailbox_name)
            .await
            .map_err(|e| e.to_string())?;

        self.session
            .uid_mv(req.uid.to_string(), "Archive")
            .await
            .map_err(|e| e.to_string())?;

        debug!("message archived");

        Ok(ArchiveMessageResponse {
            uid: req.uid,
            source_mailbox: req.mailbox_name,
            destination_mailbox: "Archive".into(),
        })
    }

    #[instrument(skip(self), fields(source = %req.source_mailbox, uid = req.uid))]
    pub async fn trash_message(
        &mut self,
        req: TrashMessageRequest,
    ) -> Result<TrashMessageResponse, String> {
        self.session
            .select(&req.source_mailbox)
            .await
            .map_err(|e| e.to_string())?;

        self.session
            .uid_mv(req.uid.to_string(), "Trash")
            .await
            .map_err(|e| e.to_string())?;

        debug!("message trashed");

        Ok(TrashMessageResponse {
            uid: req.uid,
            source_mailbox: req.source_mailbox,
        })
    }

    #[instrument(skip(self), fields(source = %req.source_mailbox, destination = %req.destination_mailbox, uid = req.uid))]
    pub async fn move_message(
        &mut self,
        req: MoveMessageRequest,
    ) -> Result<MoveMessageResponse, String> {
        if req.destination_mailbox.eq_ignore_ascii_case("trash") {
            warn!("blocked move to Trash");
            return Err("Moving messages to Trash is not allowed via this tool. Use the trash_message tool instead.".into());
        }

        self.session
            .select(&req.source_mailbox)
            .await
            .map_err(|e| e.to_string())?;

        self.session
            .uid_mv(req.uid.to_string(), &req.destination_mailbox)
            .await
            .map_err(|e| e.to_string())?;

        debug!("message moved");

        Ok(MoveMessageResponse {
            uid: req.uid,
            source_mailbox: req.source_mailbox,
            destination_mailbox: req.destination_mailbox,
        })
    }

    #[instrument(skip(self), fields(mailbox = %req.mailbox_name, uid = req.uid, read = req.read))]
    pub async fn mark_as_read(
        &mut self,
        req: MarkAsReadRequest,
    ) -> Result<MarkAsReadResponse, String> {
        self.session
            .select(&req.mailbox_name)
            .await
            .map_err(|e| e.to_string())?;

        let flag_op = if req.read { "+FLAGS" } else { "-FLAGS" };
        self.session
            .uid_store(req.uid.to_string(), format!("{flag_op} (\\Seen)"))
            .await
            .map_err(|e| e.to_string())?
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| e.to_string())?;

        debug!(
            "message marked as {}",
            if req.read { "read" } else { "unread" }
        );

        Ok(MarkAsReadResponse {
            uid: req.uid,
            mailbox_name: req.mailbox_name,
            read: req.read,
        })
    }

    #[instrument(skip(self), fields(mailbox = %req.mailbox_name, has_message_id = req.message_id.is_some(), has_subject = req.subject.is_some()))]
    pub async fn list_messages_by_thread(
        &mut self,
        req: ListMessagesByThreadRequest,
    ) -> Result<ListMessagesByThreadResponse, String> {
        if req.message_id.is_none() && req.subject.is_none() {
            warn!("neither message_id nor subject provided");
            return Err("At least one of message_id or subject must be provided".into());
        }

        self.session
            .examine(&req.mailbox_name)
            .await
            .map_err(|e| e.to_string())?;

        let mut all_uids = HashSet::new();

        if let Some(ref mid) = req.message_id {
            let escaped = escape_search_string(mid);
            let searches = [
                format!("HEADER Message-ID \"{escaped}\""),
                format!("HEADER References \"{escaped}\""),
                format!("HEADER In-Reply-To \"{escaped}\""),
            ];
            for query in &searches {
                if let Ok(uids) = self.session.uid_search(query).await {
                    all_uids.extend(uids);
                }
            }
        }

        if let Some(ref subj) = req.subject {
            let query = format!("SUBJECT \"{}\"", escape_search_string(subj));
            if let Ok(uids) = self.session.uid_search(&query).await {
                all_uids.extend(uids);
            }
        }

        if all_uids.is_empty() {
            return Ok(ListMessagesByThreadResponse {
                messages: Vec::new(),
                total_in_thread: 0,
            });
        }

        let uid_set = all_uids
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");

        let fetches: Vec<Fetch> = self
            .session
            .uid_fetch(uid_set, FETCH_HEADERS)
            .await
            .map_err(|e| e.to_string())?
            .try_collect()
            .await
            .map_err(|e| e.to_string())?;

        let mut messages: Vec<ThreadMessage> = fetches
            .iter()
            .filter_map(|fetch| {
                let uid = fetch.uid?;
                let (headers, _) = mailparse::parse_headers(fetch.header()?).ok()?;

                let subject = headers.get_first_value("Subject");
                let date = headers.get_first_value("Date");
                let from = headers
                    .get_first_header("From")
                    .map(parse_address_header)
                    .unwrap_or_default();
                let flags = fetch.flags().map(|f| MessageFlag::from(&f)).collect();
                let message_id = headers.get_first_value("Message-ID");
                let in_reply_to = headers.get_first_value("In-Reply-To");
                let references = headers.get_first_value("References");

                Some(ThreadMessage {
                    uid,
                    message_id,
                    in_reply_to,
                    references,
                    subject,
                    from,
                    date,
                    flags,
                })
            })
            .collect();

        messages.sort_by_key(|m| m.uid);
        let total_in_thread = messages.len() as u32;
        debug!(total_in_thread, "found thread messages");

        Ok(ListMessagesByThreadResponse {
            messages,
            total_in_thread,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::schema_for;
    use serde_json::Value;

    fn get_property<'a>(schema: &'a Value, field: &str) -> &'a Value {
        &schema["properties"][field]
    }

    #[test]
    fn list_messages_request_schema() {
        let schema = serde_json::to_value(schema_for!(ListMessagesRequest)).unwrap();
        let mailbox = get_property(&schema, "mailbox_name");
        assert_eq!(
            mailbox["minLength"], 1,
            "mailbox_name should have minLength"
        );
        assert!(
            mailbox["examples"]
                .as_array()
                .unwrap()
                .contains(&Value::String("INBOX".into())),
            "mailbox_name should have INBOX example"
        );
        assert_eq!(
            mailbox["description"],
            "Name of the mailbox to list messages from (e.g. \"INBOX\")"
        );

        let limit = get_property(&schema, "limit");
        assert_eq!(limit["maximum"], 100);
        assert_eq!(limit["minimum"], 0);

        let before_uid = get_property(&schema, "before_uid");
        assert_eq!(before_uid["minimum"], 1, "before_uid should have minimum 1");
        assert_eq!(
            before_uid["description"],
            "Return messages with UID less than this value (for pagination)"
        );
    }

    #[test]
    fn get_message_request_schema() {
        let schema = serde_json::to_value(schema_for!(GetMessageRequest)).unwrap();
        let mailbox = get_property(&schema, "mailbox_name");
        assert_eq!(mailbox["minLength"], 1);
        assert!(
            mailbox["examples"]
                .as_array()
                .unwrap()
                .contains(&Value::String("INBOX".into()))
        );

        let uid = get_property(&schema, "uid");
        assert_eq!(uid["minimum"], 1, "uid should have minimum 1");
    }

    #[test]
    fn search_messages_request_schema() {
        let schema = serde_json::to_value(schema_for!(SearchMessagesRequest)).unwrap();
        let mailbox = get_property(&schema, "mailbox_name");
        assert_eq!(mailbox["minLength"], 1);

        let since = get_property(&schema, "since");
        assert!(
            since["examples"]
                .as_array()
                .unwrap()
                .contains(&Value::String("01-Jan-2024".into())),
            "since should have date example"
        );

        let before = get_property(&schema, "before");
        assert!(
            before["examples"]
                .as_array()
                .unwrap()
                .contains(&Value::String("31-Dec-2024".into())),
            "before should have date example"
        );

        let limit = get_property(&schema, "limit");
        assert_eq!(limit["minimum"], 1, "limit minimum should be 1");
        assert_eq!(limit["maximum"], 100, "limit maximum should be 100");
    }

    #[test]
    fn get_mailbox_status_request_schema() {
        let schema = serde_json::to_value(schema_for!(GetMailboxStatusRequest)).unwrap();
        let mailbox = get_property(&schema, "mailbox_name");
        assert_eq!(mailbox["minLength"], 1);
        assert!(
            mailbox["examples"]
                .as_array()
                .unwrap()
                .contains(&Value::String("INBOX".into()))
        );
    }

    #[test]
    fn get_attachment_request_schema() {
        let schema = serde_json::to_value(schema_for!(GetAttachmentRequest)).unwrap();
        let mailbox = get_property(&schema, "mailbox_name");
        assert_eq!(mailbox["minLength"], 1);

        let uid = get_property(&schema, "uid");
        assert_eq!(uid["minimum"], 1);
    }

    #[test]
    fn list_messages_by_thread_request_schema() {
        let schema = serde_json::to_value(schema_for!(ListMessagesByThreadRequest)).unwrap();
        let mailbox = get_property(&schema, "mailbox_name");
        assert_eq!(mailbox["minLength"], 1);

        let mid = get_property(&schema, "message_id");
        assert!(
            mid["description"]
                .as_str()
                .unwrap()
                .contains("at least one of"),
            "message_id description should mention requirement"
        );

        let subj = get_property(&schema, "subject");
        assert!(
            subj["description"]
                .as_str()
                .unwrap()
                .contains("at least one of"),
            "subject description should mention requirement"
        );
    }

    #[test]
    fn archive_message_request_schema() {
        let schema = serde_json::to_value(schema_for!(ArchiveMessageRequest)).unwrap();
        let mailbox = get_property(&schema, "mailbox_name");
        assert_eq!(mailbox["minLength"], 1);

        let uid = get_property(&schema, "uid");
        assert_eq!(uid["minimum"], 1);
    }

    #[test]
    fn move_message_request_schema() {
        let schema = serde_json::to_value(schema_for!(MoveMessageRequest)).unwrap();
        let source = get_property(&schema, "source_mailbox");
        assert_eq!(source["minLength"], 1);
        assert!(
            source["examples"]
                .as_array()
                .unwrap()
                .contains(&Value::String("INBOX".into()))
        );

        let dest = get_property(&schema, "destination_mailbox");
        assert_eq!(dest["minLength"], 1);
        assert!(
            dest["examples"]
                .as_array()
                .unwrap()
                .contains(&Value::String("Archive".into()))
        );

        let uid = get_property(&schema, "uid");
        assert_eq!(uid["minimum"], 1);
    }

    #[test]
    fn mark_as_read_request_schema() {
        let schema = serde_json::to_value(schema_for!(MarkAsReadRequest)).unwrap();
        let mailbox = get_property(&schema, "mailbox_name");
        assert_eq!(mailbox["minLength"], 1);
        assert!(
            mailbox["examples"]
                .as_array()
                .unwrap()
                .contains(&Value::String("INBOX".into()))
        );

        let uid = get_property(&schema, "uid");
        assert_eq!(uid["minimum"], 1);

        let read = get_property(&schema, "read");
        assert!(
            read["description"].as_str().unwrap().contains("unread"),
            "read description should mention unread"
        );
    }

    #[test]
    fn mark_as_read_request_defaults_to_true() {
        let req: MarkAsReadRequest =
            serde_json::from_str(r#"{"mailbox_name": "INBOX", "uid": 1}"#).unwrap();
        assert!(req.read, "read should default to true");
    }

    #[test]
    fn response_structs_have_field_descriptions() {
        let check_descriptions = |schema: Value, struct_name: &str, fields: &[&str]| {
            fields.iter().for_each(|field| {
                let prop = get_property(&schema, field);
                assert!(
                    prop.get("description").is_some()
                        || prop.get("$ref").is_some()
                        || prop.get("allOf").is_some(),
                    "{struct_name}.{field} should have a description (or be a $ref)"
                );
            });
        };

        check_descriptions(
            serde_json::to_value(schema_for!(ListMessagesResponse)).unwrap(),
            "ListMessagesResponse",
            &["messages", "total_in_mailbox"],
        );
        check_descriptions(
            serde_json::to_value(schema_for!(GetMessageResponse)).unwrap(),
            "GetMessageResponse",
            &[
                "uid",
                "subject",
                "from",
                "to",
                "cc",
                "date",
                "flags",
                "body_text",
                "body_html",
                "attachments",
            ],
        );
        check_descriptions(
            serde_json::to_value(schema_for!(SearchMessagesResponse)).unwrap(),
            "SearchMessagesResponse",
            &["messages", "total_matches"],
        );
        check_descriptions(
            serde_json::to_value(schema_for!(GetMailboxStatusResponse)).unwrap(),
            "GetMailboxStatusResponse",
            &[
                "mailbox_name",
                "messages",
                "recent",
                "unseen",
                "uid_next",
                "uid_validity",
            ],
        );
        check_descriptions(
            serde_json::to_value(schema_for!(GetAttachmentResponse)).unwrap(),
            "GetAttachmentResponse",
            &[
                "uid",
                "attachment_index",
                "filename",
                "mime_type",
                "size_bytes",
                "content_base64",
            ],
        );
        check_descriptions(
            serde_json::to_value(schema_for!(ListMessagesByThreadResponse)).unwrap(),
            "ListMessagesByThreadResponse",
            &["messages", "total_in_thread"],
        );
        check_descriptions(
            serde_json::to_value(schema_for!(ArchiveMessageResponse)).unwrap(),
            "ArchiveMessageResponse",
            &["uid", "source_mailbox", "destination_mailbox"],
        );
        check_descriptions(
            serde_json::to_value(schema_for!(MoveMessageResponse)).unwrap(),
            "MoveMessageResponse",
            &["uid", "source_mailbox", "destination_mailbox"],
        );
        check_descriptions(
            serde_json::to_value(schema_for!(MarkAsReadResponse)).unwrap(),
            "MarkAsReadResponse",
            &["uid", "mailbox_name", "read"],
        );
    }

    #[test]
    fn message_summary_has_field_descriptions() {
        let schema = serde_json::to_value(schema_for!(MessageSummary)).unwrap();
        ["uid", "subject", "from", "date", "flags"]
            .iter()
            .for_each(|field| {
                let prop = get_property(&schema, field);
                assert!(
                    prop.get("description").is_some()
                        || prop.get("$ref").is_some()
                        || prop.get("allOf").is_some(),
                    "MessageSummary.{field} should have a description"
                );
            });
    }

    #[test]
    fn attachment_info_has_field_descriptions() {
        let schema = serde_json::to_value(schema_for!(AttachmentInfo)).unwrap();
        ["filename", "mime_type", "size_bytes"]
            .iter()
            .for_each(|field| {
                let prop = get_property(&schema, field);
                assert!(
                    prop.get("description").is_some(),
                    "AttachmentInfo.{field} should have a description"
                );
            });
    }

    #[test]
    fn raw_utf8_in_envelope_causes_takwhile1_error() {
        // Proton Mail Bridge sends raw UTF-8 in ENVELOPE responses (e.g.
        // "José" instead of RFC 2047-encoded "=?UTF-8?Q?Jos=C3=A9?=").
        // imap-proto's parser rejects this with a TakeWhile1 error.
        let raw_response = b"* 6 FETCH (UID 253 ENVELOPE (\
            \"Mon, 23 Mar 2026 14:32:50 -0500\" \
            \"Hello\" \
            ((\"Ren\xc3\xa9 M\xc3\xbcller\" NIL \"rmuller\" \"example.org\")) \
            ((\"Ren\xc3\xa9 M\xc3\xbcller\" NIL \"rmuller\" \"example.org\")) \
            ((NIL NIL \"rmuller\" \"example.org\")) \
            ((NIL NIL \"user\" \"pm.me\")) \
            NIL NIL NIL \
            \"<msg@example.org>\") FLAGS (\\Recent))\r\n";

        let result = imap_proto::parser::parse_response(raw_response);
        let err = result.unwrap_err();
        let err_str = format!("{err:?}");
        assert!(
            err_str.contains("TakeWhile1"),
            "should produce the same TakeWhile1 error seen in prod, got: {err_str}"
        );

        let raw_headers = b"From: \"Ren\xc3\xa9 M\xc3\xbcller\" <rmuller@example.org>\r\n\
            To: user@pm.me\r\n\
            Subject: Hello\r\n\
            Date: Mon, 23 Mar 2026 14:32:50 -0500\r\n\r\n";

        let (headers, _) = mailparse::parse_headers(raw_headers).unwrap();
        assert_eq!(headers.get_first_value("Subject").as_deref(), Some("Hello"));

        let from_header = headers.get_first_header("From").unwrap();
        let addrs = parse_address_header(from_header);
        assert_eq!(addrs.len(), 1);
        assert!(
            addrs[0].contains("René"),
            "should preserve UTF-8 name, got: {}",
            addrs[0]
        );
        assert!(
            addrs[0].contains("rmuller@example.org"),
            "should contain email, got: {}",
            addrs[0]
        );
    }

    #[test]
    fn thread_message_has_field_descriptions() {
        let schema = serde_json::to_value(schema_for!(ThreadMessage)).unwrap();
        [
            "uid",
            "message_id",
            "in_reply_to",
            "references",
            "subject",
            "from",
            "date",
            "flags",
        ]
        .iter()
        .for_each(|field| {
            let prop = get_property(&schema, field);
            assert!(
                prop.get("description").is_some()
                    || prop.get("$ref").is_some()
                    || prop.get("allOf").is_some(),
                "ThreadMessage.{field} should have a description"
            );
        });
    }
}
