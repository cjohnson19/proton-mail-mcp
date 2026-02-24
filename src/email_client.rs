use std::net::TcpStream;

use imap::{
    Session,
    types::{Fetch, Flag, Name, NameAttribute},
};
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

// --- Message types ---

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

// --- Request/Response types ---

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

// --- Helpers ---

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

// --- EmailClient ---

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

        let (body_text, body_html, attachments) = match fetch.body() {
            Some(raw) => {
                let parsed = mailparse::parse_mail(raw).map_err(|e| e.to_string())?;
                let (text, html) = extract_body_parts(&parsed);
                let mut atts = Vec::new();
                collect_attachments(&parsed, &mut atts);
                (text, html, atts)
            }
            None => (None, None, Vec::new()),
        };

        Ok(GetMessageResponse {
            uid: req.uid,
            subject,
            from,
            to,
            cc,
            date,
            flags,
            body_text,
            body_html,
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
}
