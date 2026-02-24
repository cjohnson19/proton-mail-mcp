use std::collections::HashSet;
use std::net::TcpStream;

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use imap::{
    Session,
    types::{Fetch, Flag, Name, NameAttribute},
};
use mailparse::MailHeaderMap;
use native_tls::TlsStream;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct EmailClient {
    session: Session<TlsStream<TcpStream>>,
}

#[allow(dead_code)]
#[derive(Debug, JsonSchema)]
pub struct ListMailboxesRequest;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct MailboxDescription {
    name: String,
    attributes: Vec<MailboxAttribute>,
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
            NameAttribute::Custom(cow) => Self::Custom(cow.to_string()),
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
    pub uid: u32,
    pub subject: Option<String>,
    pub from: Vec<String>,
    pub date: Option<String>,
    pub flags: Vec<MessageFlag>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AttachmentInfo {
    pub filename: Option<String>,
    pub mime_type: String,
    pub size_bytes: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListMessagesRequest {
    /// Name of the mailbox to list messages from (e.g. "INBOX")
    pub mailbox_name: String,
    /// Maximum number of messages to return (default 25, max 100)
    pub limit: Option<u32>,
    /// Return messages with UID less than this value (for pagination)
    pub before_uid: Option<u32>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListMessagesResponse {
    pub messages: Vec<MessageSummary>,
    pub total_in_mailbox: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetMessageRequest {
    /// Name of the mailbox containing the message
    pub mailbox_name: String,
    /// UID of the message to retrieve
    pub uid: u32,
    /// Whether to include the raw HTML body in the response (default: false)
    pub include_html: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetMessageResponse {
    pub uid: u32,
    pub subject: Option<String>,
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub date: Option<String>,
    pub flags: Vec<MessageFlag>,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub attachments: Vec<AttachmentInfo>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchMessagesRequest {
    /// Name of the mailbox to search
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
    pub since: Option<String>,
    /// Messages before this date (DD-Mon-YYYY format, e.g. "31-Dec-2024")
    pub before: Option<String>,
    /// Filter by seen/unseen status
    pub seen: Option<bool>,
    /// Filter by flagged status
    pub flagged: Option<bool>,
    /// Maximum number of results (default 25, max 100)
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchMessagesResponse {
    pub messages: Vec<MessageSummary>,
    pub total_matches: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetMailboxStatusRequest {
    /// Name of the mailbox (e.g. "INBOX")
    pub mailbox_name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetMailboxStatusResponse {
    pub mailbox_name: String,
    pub messages: u32,
    pub recent: u32,
    pub unseen: Option<u32>,
    pub uid_next: Option<u32>,
    pub uid_validity: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetAttachmentRequest {
    /// Name of the mailbox containing the message
    pub mailbox_name: String,
    /// UID of the message
    pub uid: u32,
    /// 0-based attachment index (matches order from get_message)
    pub attachment_index: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetAttachmentResponse {
    pub uid: u32,
    pub attachment_index: usize,
    pub filename: Option<String>,
    pub mime_type: String,
    pub size_bytes: usize,
    pub content_base64: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListMessagesByThreadRequest {
    /// Name of the mailbox to search
    pub mailbox_name: String,
    /// Message-ID header value to find the thread for
    pub message_id: Option<String>,
    /// Subject to search for (used if message_id not provided)
    pub subject: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ThreadMessage {
    pub uid: u32,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub subject: Option<String>,
    pub from: Vec<String>,
    pub date: Option<String>,
    pub flags: Vec<MessageFlag>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListMessagesByThreadResponse {
    pub messages: Vec<ThreadMessage>,
    pub total_in_thread: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ArchiveMessageRequest {
    /// Name of the mailbox containing the message (e.g. "INBOX")
    pub mailbox_name: String,
    /// UID of the message to archive
    pub uid: u32,
    /// Subject of the message (not used for the operation, but shown during tool approval for context)
    #[allow(dead_code)]
    #[serde(default)]
    pub subject: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ArchiveMessageResponse {
    pub uid: u32,
    pub source_mailbox: String,
    pub destination_mailbox: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveMessageRequest {
    /// Name of the mailbox containing the message
    pub source_mailbox: String,
    /// Destination mailbox to move the message to (cannot be "Trash")
    pub destination_mailbox: String,
    /// UID of the message to move
    pub uid: u32,
    /// Subject of the message (not used for the operation, but shown during tool approval for context)
    #[allow(dead_code)]
    #[serde(default)]
    pub subject: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MoveMessageResponse {
    pub uid: u32,
    pub source_mailbox: String,
    pub destination_mailbox: String,
}

fn decode_rfc2047(raw: &[u8]) -> String {
    let lossy = String::from_utf8_lossy(raw);
    let header_line = format!("Subject: {lossy}");
    mailparse::parse_header(header_line.as_bytes())
        .map(|(header, _)| header.get_value())
        .unwrap_or_else(|_| lossy.into_owned())
}

fn format_address_parts(
    name: Option<&[u8]>,
    mailbox: Option<&[u8]>,
    host: Option<&[u8]>,
) -> String {
    let mailbox_str = mailbox
        .map(|m| String::from_utf8_lossy(m))
        .unwrap_or_default();
    let host_str = host
        .map(|h| String::from_utf8_lossy(h))
        .unwrap_or_default();
    let email = format!("{mailbox_str}@{host_str}");

    match name {
        Some(n) if !n.is_empty() => {
            let decoded = decode_rfc2047(n);
            if decoded.is_empty() {
                email
            } else {
                format!("{decoded} <{email}>")
            }
        }
        _ => email,
    }
}

fn extract_summaries(fetches: &[Fetch]) -> Vec<MessageSummary> {
    fetches
        .iter()
        .filter_map(|fetch| {
            let uid = fetch.uid?;
            let envelope = fetch.envelope();

            let subject = envelope
                .and_then(|e| e.subject)
                .map(decode_rfc2047);

            let date = envelope
                .and_then(|e| e.date)
                .map(|d| String::from_utf8_lossy(d).into_owned());

            let from = envelope
                .and_then(|e| e.from.as_ref())
                .map(|addrs| {
                    addrs
                        .iter()
                        .map(|a| format_address_parts(a.name, a.mailbox, a.host))
                        .collect()
                })
                .unwrap_or_default();

            let flags = fetch.flags().iter().map(MessageFlag::from).collect();

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

    let part = parts
        .get(index)
        .ok_or_else(|| format!("Attachment index {index} out of range (message has {} attachments)", parts.len()))?;

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
    pub fn new(domain: &str, port: u16, username: &str, password: &str) -> Result<Self, String> {
        let tls = native_tls::TlsConnector::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap();
        let client =
            imap::connect_starttls((domain, port), domain, &tls).map_err(|e| e.to_string())?;
        let imap_session = client
            .login(username, password)
            .map_err(|e| e.0.to_string())?;
        Ok(Self {
            session: imap_session,
        })
    }

    pub fn list_mailboxes(&mut self) -> Result<ListMailboxesResponse, String> {
        self.session.list(None, Some("*")).map_or_else(
            |e| Err(e.to_string()),
            |r| {
                Ok(ListMailboxesResponse(
                    r.iter().map(MailboxDescription::from).collect(),
                ))
            },
        )
    }

    pub fn list_messages(&mut self, req: ListMessagesRequest) -> Result<ListMessagesResponse, String> {
        let limit = req.limit.unwrap_or(25).min(100);
        let mailbox = self
            .session
            .examine(&req.mailbox_name)
            .map_err(|e| e.to_string())?;
        let total = mailbox.exists;

        if total == 0 {
            return Ok(ListMessagesResponse {
                messages: Vec::new(),
                total_in_mailbox: total,
            });
        }

        let fetches = match req.before_uid {
            Some(before_uid) => {
                if before_uid <= 1 {
                    return Ok(ListMessagesResponse {
                        messages: Vec::new(),
                        total_in_mailbox: total,
                    });
                }
                self.session
                    .uid_fetch(format!("1:{}", before_uid - 1), "(UID ENVELOPE FLAGS)")
                    .map_err(|e| e.to_string())?
            }
            None => {
                let start = total.saturating_sub(limit).saturating_add(1);
                self.session
                    .fetch(format!("{start}:{total}"), "(UID ENVELOPE FLAGS)")
                    .map_err(|e| e.to_string())?
            }
        };

        let mut messages = extract_summaries(&fetches);
        messages.sort_by(|a, b| b.uid.cmp(&a.uid));
        messages.truncate(limit as usize);

        Ok(ListMessagesResponse {
            messages,
            total_in_mailbox: total,
        })
    }

    pub fn get_message(&mut self, req: GetMessageRequest) -> Result<GetMessageResponse, String> {
        self.session
            .examine(&req.mailbox_name)
            .map_err(|e| e.to_string())?;

        let fetches = self
            .session
            .uid_fetch(req.uid.to_string(), "(UID ENVELOPE FLAGS BODY.PEEK[])")
            .map_err(|e| e.to_string())?;

        let fetch = fetches
            .iter()
            .next()
            .ok_or_else(|| format!("Message with UID {} not found", req.uid))?;

        let envelope = fetch
            .envelope()
            .ok_or("Message has no envelope")?;

        let subject = envelope.subject.map(decode_rfc2047);
        let date = envelope
            .date
            .map(|d| String::from_utf8_lossy(d).into_owned());

        let from: Vec<String> = envelope
            .from
            .as_ref()
            .map(|addrs| {
                addrs
                    .iter()
                    .map(|a| format_address_parts(a.name, a.mailbox, a.host))
                    .collect()
            })
            .unwrap_or_default();
        let to: Vec<String> = envelope
            .to
            .as_ref()
            .map(|addrs| {
                addrs
                    .iter()
                    .map(|a| format_address_parts(a.name, a.mailbox, a.host))
                    .collect()
            })
            .unwrap_or_default();
        let cc: Vec<String> = envelope
            .cc
            .as_ref()
            .map(|addrs| {
                addrs
                    .iter()
                    .map(|a| format_address_parts(a.name, a.mailbox, a.host))
                    .collect()
            })
            .unwrap_or_default();
        let flags = fetch.flags().iter().map(MessageFlag::from).collect();

        let (mut body_text, body_html, attachments) = match fetch.body() {
            Some(raw) => {
                let parsed = mailparse::parse_mail(raw).map_err(|e| e.to_string())?;
                let (text, html) = extract_body_parts(&parsed);
                let mut atts = Vec::new();
                collect_attachments(&parsed, &mut atts);
                (text, html, atts)
            }
            None => (None, None, Vec::new()),
        };

        if body_text.is_none()
            && let Some(ref html) = body_html
        {
            body_text = html2text::from_read(html.as_bytes(), 80).ok();
        }

        let include_html = req.include_html.unwrap_or(false);

        Ok(GetMessageResponse {
            uid: req.uid,
            subject,
            from,
            to,
            cc,
            date,
            flags,
            body_text,
            body_html: if include_html { body_html } else { None },
            attachments,
        })
    }

    pub fn search_messages(
        &mut self,
        req: SearchMessagesRequest,
    ) -> Result<SearchMessagesResponse, String> {
        let limit = req.limit.unwrap_or(25).min(100);
        self.session
            .examine(&req.mailbox_name)
            .map_err(|e| e.to_string())?;

        let query = build_search_query(&req);
        let uids = self
            .session
            .uid_search(&query)
            .map_err(|e| e.to_string())?;
        let total_matches = uids.len() as u32;

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

        let fetches = self
            .session
            .uid_fetch(uid_set, "(UID ENVELOPE FLAGS)")
            .map_err(|e| e.to_string())?;

        let mut messages = extract_summaries(&fetches);
        messages.sort_by(|a, b| b.uid.cmp(&a.uid));

        Ok(SearchMessagesResponse {
            messages,
            total_matches,
        })
    }

    pub fn get_mailbox_status(
        &mut self,
        req: GetMailboxStatusRequest,
    ) -> Result<GetMailboxStatusResponse, String> {
        let mailbox = self
            .session
            .examine(&req.mailbox_name)
            .map_err(|e| e.to_string())?;

        let unseen = self
            .session
            .uid_search("UNSEEN")
            .map(|uids| uids.len() as u32)
            .ok();

        Ok(GetMailboxStatusResponse {
            mailbox_name: req.mailbox_name,
            messages: mailbox.exists,
            recent: mailbox.recent,
            unseen,
            uid_next: mailbox.uid_next,
            uid_validity: mailbox.uid_validity,
        })
    }

    pub fn get_attachment(
        &mut self,
        req: GetAttachmentRequest,
    ) -> Result<GetAttachmentResponse, String> {
        self.session
            .examine(&req.mailbox_name)
            .map_err(|e| e.to_string())?;

        let fetches = self
            .session
            .uid_fetch(req.uid.to_string(), "BODY.PEEK[]")
            .map_err(|e| e.to_string())?;

        let fetch = fetches
            .iter()
            .next()
            .ok_or_else(|| format!("Message with UID {} not found", req.uid))?;

        let raw = fetch
            .body()
            .ok_or("Message has no body")?;

        let (filename, mime_type, body) = extract_attachment_by_index(raw, req.attachment_index)?;
        let size_bytes = body.len();
        let content_base64 = BASE64.encode(&body);

        Ok(GetAttachmentResponse {
            uid: req.uid,
            attachment_index: req.attachment_index,
            filename,
            mime_type,
            size_bytes,
            content_base64,
        })
    }

    pub fn archive_message(
        &mut self,
        req: ArchiveMessageRequest,
    ) -> Result<ArchiveMessageResponse, String> {
        self.session
            .select(&req.mailbox_name)
            .map_err(|e| e.to_string())?;

        self.session
            .uid_mv(req.uid.to_string(), "Archive")
            .map_err(|e| e.to_string())?;

        Ok(ArchiveMessageResponse {
            uid: req.uid,
            source_mailbox: req.mailbox_name,
            destination_mailbox: "Archive".into(),
        })
    }

    pub fn move_message(
        &mut self,
        req: MoveMessageRequest,
    ) -> Result<MoveMessageResponse, String> {
        if req.destination_mailbox.eq_ignore_ascii_case("trash") {
            return Err("Moving messages to Trash is not allowed via this tool. Use your email client to delete messages.".into());
        }

        self.session
            .select(&req.source_mailbox)
            .map_err(|e| e.to_string())?;

        self.session
            .uid_mv(req.uid.to_string(), &req.destination_mailbox)
            .map_err(|e| e.to_string())?;

        Ok(MoveMessageResponse {
            uid: req.uid,
            source_mailbox: req.source_mailbox,
            destination_mailbox: req.destination_mailbox,
        })
    }

    pub fn list_messages_by_thread(
        &mut self,
        req: ListMessagesByThreadRequest,
    ) -> Result<ListMessagesByThreadResponse, String> {
        if req.message_id.is_none() && req.subject.is_none() {
            return Err("At least one of message_id or subject must be provided".into());
        }

        self.session
            .examine(&req.mailbox_name)
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
                if let Ok(uids) = self.session.uid_search(query) {
                    all_uids.extend(uids);
                }
            }
        }

        if let Some(ref subj) = req.subject {
            let query = format!("SUBJECT \"{}\"", escape_search_string(subj));
            if let Ok(uids) = self.session.uid_search(&query) {
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

        let fetches = self
            .session
            .uid_fetch(uid_set, "(UID ENVELOPE FLAGS BODY.PEEK[HEADER])")
            .map_err(|e| e.to_string())?;

        let mut messages: Vec<ThreadMessage> = fetches
            .iter()
            .filter_map(|fetch| {
                let uid = fetch.uid?;
                let envelope = fetch.envelope();

                let subject = envelope.and_then(|e| e.subject).map(decode_rfc2047);
                let date = envelope
                    .and_then(|e| e.date)
                    .map(|d| String::from_utf8_lossy(d).into_owned());
                let from = envelope
                    .and_then(|e| e.from.as_ref())
                    .map(|addrs| {
                        addrs
                            .iter()
                            .map(|a| format_address_parts(a.name, a.mailbox, a.host))
                            .collect()
                    })
                    .unwrap_or_default();
                let flags = fetch.flags().iter().map(MessageFlag::from).collect();

                let (message_id, in_reply_to, references) = fetch
                    .header()
                    .and_then(|raw| mailparse::parse_headers(raw).ok())
                    .map(|(headers, _)| {
                        (
                            headers.get_first_value("Message-ID"),
                            headers.get_first_value("In-Reply-To"),
                            headers.get_first_value("References"),
                        )
                    })
                    .unwrap_or((None, None, None));

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

        Ok(ListMessagesByThreadResponse {
            messages,
            total_in_thread,
        })
    }
}
