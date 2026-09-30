use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rmcp::model::{CallToolResult, ContentBlock, Resource};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::email_client::{
    AttachmentData, AttachmentDeliveryKind, AttachmentDeliveryPreference, EmailClient,
    GetAttachmentRequest, GetAttachmentResponse,
};

pub const DEFAULT_INLINE_MAX_BYTES: usize = 16 * 1024;
pub const DEFAULT_MAX_TEXT_CHARS: usize = 40_000;
const URL_TTL_SECS: u64 = 3600;
const AUDIENCE: &str = "attachment";

/// Decides how `get_attachment` hands content back to the caller.
pub struct DeliveryConfig {
    pub inline_max_bytes: usize,
    pub link: LinkDelivery,
}

/// What a non-inline delivery looks like on this transport.
pub enum LinkDelivery {
    /// HTTP mode: a signed, time-limited URL served by [`router`].
    SignedUrl(Arc<AttachmentSigner>),
    /// stdio mode: the bytes are written under this directory.
    File { dir: PathBuf },
}

impl DeliveryConfig {
    pub async fn deliver(
        &self,
        req: &GetAttachmentRequest,
        data: AttachmentData,
    ) -> Result<GetAttachmentResponse, String> {
        let inline = match req.delivery {
            AttachmentDeliveryPreference::Inline => true,
            AttachmentDeliveryPreference::Link | AttachmentDeliveryPreference::Text => false,
            AttachmentDeliveryPreference::Auto => data.content.len() <= self.inline_max_bytes,
        };
        let mut response = GetAttachmentResponse {
            uid: req.uid,
            attachment_index: req.attachment_index,
            filename: data.filename.clone(),
            mime_type: data.mime_type.clone(),
            size_bytes: data.content.len(),
            delivery: AttachmentDeliveryKind::Inline,
            content_base64: None,
            download_url: None,
            file_path: None,
            expires_at: None,
            text: None,
            text_total_chars: None,
            text_next_offset: None,
        };
        if req.delivery == AttachmentDeliveryPreference::Text {
            let text = extract_text(data).await?;
            let page = page_text(
                &text,
                req.text_offset.unwrap_or(0),
                req.max_text_chars.unwrap_or(DEFAULT_MAX_TEXT_CHARS),
            );
            response.delivery = AttachmentDeliveryKind::Text;
            response.text = Some(page.text);
            response.text_total_chars = Some(page.total_chars);
            response.text_next_offset = page.next_offset;
            return Ok(response);
        }
        if inline {
            response.content_base64 = Some(BASE64.encode(&data.content));
            return Ok(response);
        }
        match &self.link {
            LinkDelivery::SignedUrl(signer) => {
                let (url, expires_at) = signer.sign(&AttachmentRef {
                    mailbox: req.mailbox_name.clone(),
                    uid: req.uid,
                    uid_validity: data.uid_validity,
                    index: req.attachment_index,
                });
                response.delivery = AttachmentDeliveryKind::Url;
                response.download_url = Some(url);
                response.expires_at = Some(expires_at);
            }
            LinkDelivery::File { dir } => {
                let path = write_attachment_file(dir, req, &data).await?;
                response.delivery = AttachmentDeliveryKind::File;
                response.file_path = Some(path.to_string_lossy().into_owned());
            }
        }
        Ok(response)
    }
}

struct TextPage {
    text: String,
    total_chars: usize,
    next_offset: Option<usize>,
}

fn page_text(text: &str, offset: usize, max_chars: usize) -> TextPage {
    let total_chars = text.chars().count();
    let page: String = text.chars().skip(offset).take(max_chars).collect();
    let end = offset + page.chars().count();
    TextPage {
        text: page,
        total_chars,
        next_offset: (end < total_chars).then_some(end),
    }
}

/// Text formats we can extract, chosen from the MIME type with the filename
/// extension as a fallback (mail clients often send `application/octet-stream`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextFormat {
    Pdf,
    Html,
    Plain,
}

fn text_format(mime_type: &str, filename: Option<&str>) -> Option<TextFormat> {
    let mime = mime_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let from_mime = match mime.as_str() {
        "application/pdf" | "application/x-pdf" => Some(TextFormat::Pdf),
        "text/html" | "application/xhtml+xml" => Some(TextFormat::Html),
        "application/json" | "application/xml" | "application/x-ndjson" => Some(TextFormat::Plain),
        m if m.starts_with("text/") => Some(TextFormat::Plain),
        _ => None,
    };
    from_mime.or_else(|| {
        let extension = filename?.rsplit_once('.')?.1.to_ascii_lowercase();
        match extension.as_str() {
            "pdf" => Some(TextFormat::Pdf),
            "html" | "htm" | "xhtml" => Some(TextFormat::Html),
            "txt" | "text" | "md" | "markdown" | "csv" | "tsv" | "json" | "xml" | "ics" | "log"
            | "yaml" | "yml" | "toml" | "eml" => Some(TextFormat::Plain),
            _ => None,
        }
    })
}

async fn extract_text(data: AttachmentData) -> Result<String, String> {
    let Some(format) = text_format(&data.mime_type, data.filename.as_deref()) else {
        return Err(format!(
            "text extraction is not supported for {} attachments; supported: PDF, HTML, and plain-text formats (txt, csv, json, ...). Use delivery=\"link\" to get the file itself.",
            data.mime_type
        ));
    };
    let text = match format {
        TextFormat::Plain => String::from_utf8_lossy(&data.content).into_owned(),
        TextFormat::Html => html2text::from_read(data.content.as_slice(), 80)
            .map_err(|e| format!("cannot render HTML attachment as text: {e}"))?,
        TextFormat::Pdf => extract_pdf_text(data.content).await?,
    };
    Ok(normalize_blank_lines(&text))
}

async fn extract_pdf_text(bytes: Vec<u8>) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem(&bytes))
            .map_err(|_| "PDF parser failed on this file".to_string())?
            .map_err(|e| format!("cannot extract text from PDF: {e}"))
    })
    .await
    .map_err(|e| format!("PDF extraction task failed: {e}"))?
}

fn normalize_blank_lines(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let mut out = Vec::with_capacity(lines.len());
    let mut blank_run = 0;
    lines.iter().for_each(|line| {
        if line.is_empty() {
            blank_run += 1;
        } else {
            blank_run = 0;
        }
        if blank_run <= 1 {
            out.push(*line);
        }
    });
    out.join("\n").trim().to_string()
}

async fn write_attachment_file(
    dir: &Path,
    req: &GetAttachmentRequest,
    data: &AttachmentData,
) -> Result<PathBuf, String> {
    let name = data.filename.as_deref().unwrap_or("attachment");
    let file_name = format!(
        "{}-{}-{}-{}",
        sanitize_file_name(&req.mailbox_name),
        req.uid,
        req.attachment_index,
        sanitize_file_name(name)
    );
    let dir = std::path::absolute(dir).map_err(|e| e.to_string())?;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let path = dir.join(file_name);
    tokio::fs::write(&path, &data.content)
        .await
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    info!(path = %path.display(), size_bytes = data.content.len(), "wrote attachment to disk");
    Ok(path)
}

fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => c,
            _ => '_',
        })
        .collect();
    let trimmed = cleaned.trim_matches('.');
    if trimmed.is_empty() {
        "attachment".into()
    } else {
        trimmed.to_string()
    }
}

/// Builds the MCP tool result: structured JSON plus, for link deliveries, a
/// `resource_link` content block pointing at the URL or file.
pub fn tool_result(response: GetAttachmentResponse) -> Result<CallToolResult, String> {
    let link = resource_link(&response);
    let value = serde_json::to_value(&response).map_err(|e| e.to_string())?;
    let mut result = CallToolResult::structured(value);
    if let Some(link) = link {
        result.content.push(ContentBlock::resource_link(link));
    }
    Ok(result)
}

fn resource_link(response: &GetAttachmentResponse) -> Option<Resource> {
    let uri = match response.delivery {
        AttachmentDeliveryKind::Inline | AttachmentDeliveryKind::Text => return None,
        AttachmentDeliveryKind::Url => response.download_url.clone()?,
        AttachmentDeliveryKind::File => file_uri(response.file_path.as_deref()?),
    };
    let name = response
        .filename
        .clone()
        .unwrap_or_else(|| format!("attachment-{}-{}", response.uid, response.attachment_index));
    let description = match response.delivery {
        AttachmentDeliveryKind::Url => "Time-limited download link for the attachment",
        _ => "Attachment saved on the machine running the MCP server",
    };
    Some(
        Resource::new(uri, name)
            .with_description(description)
            .with_mime_type(response.mime_type.clone())
            .with_size(response.size_bytes as u64),
    )
}

fn file_uri(path: &str) -> String {
    let encoded: String = path
        .bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'/' | b'.' | b'-' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("file://{encoded}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentRef {
    pub mailbox: String,
    pub uid: u32,
    pub uid_validity: Option<u32>,
    pub index: usize,
}

#[derive(Serialize, Deserialize)]
struct AttachmentClaims {
    aud: String,
    exp: u64,
    iat: u64,
    mailbox: String,
    uid: u32,
    uid_validity: Option<u32>,
    index: usize,
}

/// Mints and verifies the HS256 tokens embedded in attachment download URLs.
pub struct AttachmentSigner {
    base_url: String,
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
}

impl AttachmentSigner {
    /// `secret` is domain-separated from any other use (e.g. the OAuth signing
    /// key) so attachment tokens can never be confused with access tokens.
    pub fn new(base_url: &str, secret: &[u8]) -> Self {
        let key = Sha256::new()
            .chain_update(secret)
            .chain_update(b"proton-mail-mcp attachment links")
            .finalize();
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            encoding_key: EncodingKey::from_secret(&key),
            decoding_key: DecodingKey::from_secret(&key),
        }
    }

    /// A signer whose links stop working when this process exits.
    pub fn ephemeral(base_url: &str) -> Self {
        let secret: [u8; 32] = rand::random();
        Self::new(base_url, &secret)
    }

    pub fn sign(&self, attachment: &AttachmentRef) -> (String, u64) {
        let now = unix_now();
        let exp = now + URL_TTL_SECS;
        let claims = AttachmentClaims {
            aud: AUDIENCE.into(),
            exp,
            iat: now,
            mailbox: attachment.mailbox.clone(),
            uid: attachment.uid,
            uid_validity: attachment.uid_validity,
            index: attachment.index,
        };
        let token = jsonwebtoken::encode(&Header::default(), &claims, &self.encoding_key)
            .expect("HS256 signing is infallible");
        (format!("{}/attachments/{token}", self.base_url), exp)
    }

    pub fn verify(&self, token: &str) -> Option<AttachmentRef> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_audience(&[AUDIENCE]);
        validation.set_required_spec_claims(&["exp", "aud"]);
        let claims =
            jsonwebtoken::decode::<AttachmentClaims>(token, &self.decoding_key, &validation)
                .ok()?
                .claims;
        Some(AttachmentRef {
            mailbox: claims.mailbox,
            uid: claims.uid,
            uid_validity: claims.uid_validity,
            index: claims.index,
        })
    }
}

#[derive(Clone)]
struct DownloadState {
    signer: Arc<AttachmentSigner>,
    client: Arc<Mutex<EmailClient>>,
}

/// `GET /attachments/{token}` — serves raw attachment bytes for a signed link.
/// Mount it outside the bearer-token middleware; the token in the path is the
/// only credential a browser following the link will have.
pub fn router(signer: Arc<AttachmentSigner>, client: Arc<Mutex<EmailClient>>) -> Router {
    Router::new()
        .route("/attachments/{token}", get(download))
        .with_state(DownloadState { signer, client })
}

async fn download(State(state): State<DownloadState>, UrlPath(token): UrlPath<String>) -> Response {
    let Some(attachment) = state.signer.verify(&token) else {
        return (
            StatusCode::NOT_FOUND,
            "attachment link is invalid or has expired",
        )
            .into_response();
    };
    info!(mailbox = %attachment.mailbox, uid = attachment.uid, index = attachment.index, "attachment download");
    let fetched = state
        .client
        .lock()
        .await
        .fetch_attachment(&attachment.mailbox, attachment.uid, attachment.index)
        .await;
    match fetched {
        Ok(data) if data.uid_validity == attachment.uid_validity => file_response(data),
        Ok(_) => (
            StatusCode::GONE,
            "mailbox was reset since this link was issued; request a new link",
        )
            .into_response(),
        Err(e) => {
            warn!(error = %e, "attachment download failed");
            (StatusCode::NOT_FOUND, e).into_response()
        }
    }
}

fn file_response(data: AttachmentData) -> Response {
    let content_type = HeaderValue::from_str(&data.mime_type)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    let disposition = content_disposition(data.filename.as_deref());
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CONTENT_DISPOSITION, disposition),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, no-store"),
            ),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
        ],
        data.content,
    )
        .into_response()
}

fn content_disposition(filename: Option<&str>) -> HeaderValue {
    let ascii: String = filename
        .unwrap_or("attachment")
        .chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .filter(|c| !matches!(c, '"' | '\\' | ';'))
        .collect();
    let ascii = if ascii.trim().is_empty() {
        "attachment".to_string()
    } else {
        ascii
    };
    let utf8: String = filename
        .unwrap_or("attachment")
        .bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-' | b'_' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    HeaderValue::from_str(&format!(
        "attachment; filename=\"{ascii}\"; filename*=UTF-8''{utf8}"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static("attachment"))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(len: usize) -> AttachmentData {
        AttachmentData {
            filename: Some("report.pdf".into()),
            mime_type: "application/pdf".into(),
            uid_validity: Some(7),
            content: vec![0xAB; len],
        }
    }

    fn request(delivery: AttachmentDeliveryPreference) -> GetAttachmentRequest {
        GetAttachmentRequest {
            mailbox_name: "INBOX".into(),
            uid: 42,
            attachment_index: 1,
            delivery,
            text_offset: None,
            max_text_chars: None,
        }
    }

    fn text_request(offset: Option<usize>, max_chars: Option<usize>) -> GetAttachmentRequest {
        GetAttachmentRequest {
            text_offset: offset,
            max_text_chars: max_chars,
            ..request(AttachmentDeliveryPreference::Text)
        }
    }

    fn text_data(filename: &str, mime_type: &str, content: &[u8]) -> AttachmentData {
        AttachmentData {
            filename: Some(filename.into()),
            mime_type: mime_type.into(),
            uid_validity: None,
            content: content.to_vec(),
        }
    }

    /// A minimal single-page PDF with the given Helvetica text, with a valid xref table.
    fn tiny_pdf(text: &str) -> Vec<u8> {
        let stream = format!("BT /F1 18 Tf 72 720 Td ({text}) Tj ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        let mut pdf = String::from("%PDF-1.4\n");
        let offsets: Vec<usize> = objects
            .iter()
            .enumerate()
            .map(|(i, body)| {
                let offset = pdf.len();
                pdf.push_str(&format!("{} 0 obj\n{body}\nendobj\n", i + 1));
                offset
            })
            .collect();
        let xref = pdf.len();
        pdf.push_str(&format!(
            "xref\n0 {}\n0000000000 65535 f \n",
            objects.len() + 1
        ));
        offsets
            .iter()
            .for_each(|o| pdf.push_str(&format!("{o:010} 00000 n \n")));
        pdf.push_str(&format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        ));
        pdf.into_bytes()
    }

    fn url_config(limit: usize) -> DeliveryConfig {
        DeliveryConfig {
            inline_max_bytes: limit,
            link: LinkDelivery::SignedUrl(Arc::new(AttachmentSigner::new(
                "https://mail.example.com/",
                b"test-secret",
            ))),
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let nonce: u64 = rand::random();
        std::env::temp_dir().join(format!("proton-mail-mcp-test-{tag}-{nonce}"))
    }

    #[tokio::test]
    async fn auto_delivery_inlines_small_attachments() {
        let response = url_config(100)
            .deliver(&request(AttachmentDeliveryPreference::Auto), data(100))
            .await
            .unwrap();
        assert_eq!(response.delivery, AttachmentDeliveryKind::Inline);
        assert_eq!(response.size_bytes, 100);
        assert_eq!(
            BASE64.decode(response.content_base64.unwrap()).unwrap(),
            vec![0xAB; 100]
        );
        assert!(response.download_url.is_none());
        assert!(response.file_path.is_none());
    }

    #[tokio::test]
    async fn auto_delivery_links_large_attachments() {
        let config = url_config(100);
        let response = config
            .deliver(&request(AttachmentDeliveryPreference::Auto), data(101))
            .await
            .unwrap();
        assert_eq!(response.delivery, AttachmentDeliveryKind::Url);
        assert!(response.content_base64.is_none());
        let url = response.download_url.unwrap();
        assert!(
            url.starts_with("https://mail.example.com/attachments/"),
            "{url}"
        );
        assert!(response.expires_at.unwrap() > unix_now());

        let LinkDelivery::SignedUrl(signer) = &config.link else {
            unreachable!()
        };
        let token = url.rsplit('/').next().unwrap();
        assert_eq!(
            signer.verify(token).unwrap(),
            AttachmentRef {
                mailbox: "INBOX".into(),
                uid: 42,
                uid_validity: Some(7),
                index: 1,
            }
        );
    }

    #[tokio::test]
    async fn explicit_preferences_override_the_size_limit() {
        let config = url_config(100);
        let forced_inline = config
            .deliver(&request(AttachmentDeliveryPreference::Inline), data(5000))
            .await
            .unwrap();
        assert_eq!(forced_inline.delivery, AttachmentDeliveryKind::Inline);
        let forced_link = config
            .deliver(&request(AttachmentDeliveryPreference::Link), data(1))
            .await
            .unwrap();
        assert_eq!(forced_link.delivery, AttachmentDeliveryKind::Url);
    }

    #[tokio::test]
    async fn file_delivery_writes_under_the_configured_directory() {
        let dir = temp_dir("file");
        let config = DeliveryConfig {
            inline_max_bytes: 0,
            link: LinkDelivery::File { dir: dir.clone() },
        };
        let mut req = request(AttachmentDeliveryPreference::Auto);
        req.mailbox_name = "Folders/Tax 2025".into();
        let mut payload = data(10);
        payload.filename = Some("my report (final).pdf".into());
        let response = config.deliver(&req, payload).await.unwrap();
        assert_eq!(response.delivery, AttachmentDeliveryKind::File);
        let path = PathBuf::from(response.file_path.unwrap());
        assert!(path.is_absolute());
        assert_eq!(path.parent().unwrap(), std::path::absolute(&dir).unwrap());
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            "Folders_Tax_2025-42-1-my_report__final_.pdf"
        );
        assert_eq!(std::fs::read(&path).unwrap(), vec![0xAB; 10]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn text_delivery_extracts_plain_text_regardless_of_size() {
        let config = url_config(0);
        let body = "line one\nline two\n\n\n\nline three   \n";
        let response = config
            .deliver(
                &text_request(None, None),
                text_data("notes.txt", "text/plain; charset=utf-8", body.as_bytes()),
            )
            .await
            .unwrap();
        assert_eq!(response.delivery, AttachmentDeliveryKind::Text);
        assert_eq!(
            response.text.as_deref(),
            Some("line one\nline two\n\nline three")
        );
        assert_eq!(response.text_total_chars, Some(29));
        assert_eq!(response.text_next_offset, None);
        assert!(response.content_base64.is_none() && response.download_url.is_none());
    }

    #[tokio::test]
    async fn text_delivery_renders_html_and_sniffs_octet_stream_by_extension() {
        let html = b"<html><body><h1>Title</h1><p>Hello <b>world</b></p></body></html>";
        let response = url_config(0)
            .deliver(
                &text_request(None, None),
                text_data("page.html", "application/octet-stream", html),
            )
            .await
            .unwrap();
        let text = response.text.unwrap();
        assert!(
            text.contains("Title") && text.contains("Hello") && text.contains("world"),
            "{text}"
        );
        assert!(!text.contains("<b>"), "{text}");
    }

    #[tokio::test]
    async fn text_delivery_extracts_pdf_text() {
        let pdf = tiny_pdf("Hello from a PDF");
        let response = url_config(0)
            .deliver(
                &text_request(None, None),
                text_data("doc.pdf", "application/pdf", &pdf),
            )
            .await
            .unwrap();
        assert_eq!(response.delivery, AttachmentDeliveryKind::Text);
        let text = response.text.unwrap();
        assert!(text.contains("Hello from a PDF"), "{text}");
    }

    #[tokio::test]
    async fn text_delivery_rejects_unsupported_and_broken_files() {
        let config = url_config(0);
        let err = config
            .deliver(
                &text_request(None, None),
                text_data("photo.jpg", "image/jpeg", b"\xff\xd8"),
            )
            .await
            .unwrap_err();
        assert!(err.contains("not supported for image/jpeg"), "{err}");
        let err = config
            .deliver(
                &text_request(None, None),
                text_data("bad.pdf", "application/pdf", b"not a pdf"),
            )
            .await
            .unwrap_err();
        assert!(err.contains("PDF"), "{err}");
    }

    #[tokio::test]
    async fn text_delivery_pages_by_character_offset() {
        let config = url_config(0);
        let body = "αβγδε";
        let first = config
            .deliver(
                &text_request(None, Some(2)),
                text_data("g.txt", "text/plain", body.as_bytes()),
            )
            .await
            .unwrap();
        assert_eq!(first.text.as_deref(), Some("αβ"));
        assert_eq!(first.text_total_chars, Some(5));
        assert_eq!(first.text_next_offset, Some(2));
        let last = config
            .deliver(
                &text_request(Some(4), Some(2)),
                text_data("g.txt", "text/plain", body.as_bytes()),
            )
            .await
            .unwrap();
        assert_eq!(last.text.as_deref(), Some("ε"));
        assert_eq!(last.text_next_offset, None);
        let past_end = config
            .deliver(
                &text_request(Some(99), None),
                text_data("g.txt", "text/plain", body.as_bytes()),
            )
            .await
            .unwrap();
        assert_eq!(past_end.text.as_deref(), Some(""));
        assert_eq!(past_end.text_next_offset, None);
    }

    #[test]
    fn text_format_prefers_mime_then_extension() {
        assert_eq!(text_format("application/pdf", None), Some(TextFormat::Pdf));
        assert_eq!(
            text_format("TEXT/CSV; header=present", None),
            Some(TextFormat::Plain)
        );
        assert_eq!(
            text_format("application/octet-stream", Some("a.PDF")),
            Some(TextFormat::Pdf)
        );
        assert_eq!(
            text_format("application/octet-stream", Some("a.ics")),
            Some(TextFormat::Plain)
        );
        assert_eq!(text_format("application/octet-stream", Some("a.zip")), None);
        assert_eq!(text_format("application/octet-stream", None), None);
    }

    #[test]
    fn sanitize_file_name_never_yields_a_path_component_escape() {
        assert_eq!(sanitize_file_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitize_file_name(".."), "attachment");
        assert_eq!(sanitize_file_name(""), "attachment");
        assert_eq!(sanitize_file_name("ok-name_1.txt"), "ok-name_1.txt");
    }

    #[test]
    fn signer_rejects_tampered_foreign_and_expired_tokens() {
        let signer = AttachmentSigner::new("https://a.example", b"secret-a");
        let other = AttachmentSigner::new("https://a.example", b"secret-b");
        let attachment = AttachmentRef {
            mailbox: "INBOX".into(),
            uid: 1,
            uid_validity: None,
            index: 0,
        };
        let (url, _) = signer.sign(&attachment);
        let token = url.rsplit('/').next().unwrap().to_string();
        assert_eq!(signer.verify(&token), Some(attachment.clone()));
        assert!(other.verify(&token).is_none());
        let mut tampered = token.clone();
        tampered.replace_range(token.len() - 2.., "AA");
        assert!(signer.verify(&tampered).is_none());

        let expired = AttachmentClaims {
            aud: AUDIENCE.into(),
            exp: unix_now() - 120,
            iat: unix_now() - 3600,
            mailbox: "INBOX".into(),
            uid: 1,
            uid_validity: None,
            index: 0,
        };
        let expired =
            jsonwebtoken::encode(&Header::default(), &expired, &signer.encoding_key).unwrap();
        assert!(signer.verify(&expired).is_none());
    }

    #[test]
    fn signer_domain_separates_from_the_raw_secret() {
        let raw_key = EncodingKey::from_secret(b"shared");
        let claims = AttachmentClaims {
            aud: AUDIENCE.into(),
            exp: unix_now() + 60,
            iat: unix_now(),
            mailbox: "INBOX".into(),
            uid: 1,
            uid_validity: None,
            index: 0,
        };
        let token = jsonwebtoken::encode(&Header::default(), &claims, &raw_key).unwrap();
        assert!(
            AttachmentSigner::new("https://a.example", b"shared")
                .verify(&token)
                .is_none()
        );
    }

    #[test]
    fn tool_result_carries_structured_content_and_a_resource_link() {
        let response = GetAttachmentResponse {
            uid: 42,
            attachment_index: 0,
            filename: Some("photo.jpg".into()),
            mime_type: "image/jpeg".into(),
            size_bytes: 123456,
            delivery: AttachmentDeliveryKind::Url,
            content_base64: None,
            download_url: Some("https://mail.example.com/attachments/abc".into()),
            file_path: None,
            expires_at: Some(1),
            text: None,
            text_total_chars: None,
            text_next_offset: None,
        };
        let result = tool_result(response).unwrap();
        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.structured_content.unwrap()["delivery"], "url");
        assert_eq!(result.content.len(), 2);
        let link = result.content[1].as_resource_link().unwrap();
        assert_eq!(link.uri, "https://mail.example.com/attachments/abc");
        assert_eq!(link.name, "photo.jpg");
        assert_eq!(link.mime_type.as_deref(), Some("image/jpeg"));
        assert_eq!(link.size, Some(123456));
    }

    #[test]
    fn tool_result_for_file_delivery_uses_a_file_uri() {
        let response = GetAttachmentResponse {
            uid: 1,
            attachment_index: 0,
            filename: None,
            mime_type: "text/plain".into(),
            size_bytes: 3,
            delivery: AttachmentDeliveryKind::File,
            content_base64: None,
            download_url: None,
            file_path: Some("/tmp/my dir/INBOX-1-0-attachment".into()),
            expires_at: None,
            text: None,
            text_total_chars: None,
            text_next_offset: None,
        };
        let result = tool_result(response).unwrap();
        let link = result.content[1].as_resource_link().unwrap();
        assert_eq!(link.uri, "file:///tmp/my%20dir/INBOX-1-0-attachment");
        assert_eq!(link.name, "attachment-1-0");
    }

    #[test]
    fn inline_tool_result_has_no_resource_link() {
        let response = GetAttachmentResponse {
            uid: 1,
            attachment_index: 0,
            filename: None,
            mime_type: "text/plain".into(),
            size_bytes: 2,
            delivery: AttachmentDeliveryKind::Inline,
            content_base64: Some("aGk=".into()),
            download_url: None,
            file_path: None,
            expires_at: None,
            text: None,
            text_total_chars: None,
            text_next_offset: None,
        };
        let result = tool_result(response).unwrap();
        assert_eq!(result.content.len(), 1);
        assert!(result.content[0].as_text().is_some());
    }

    #[test]
    fn download_response_sets_type_and_disposition() {
        let response = file_response(AttachmentData {
            filename: Some("état \"final\".pdf".into()),
            mime_type: "application/pdf".into(),
            uid_validity: None,
            content: b"%PDF".to_vec(),
        });
        assert_eq!(response.status(), StatusCode::OK);
        let headers = response.headers();
        assert_eq!(headers[header::CONTENT_TYPE], "application/pdf");
        assert_eq!(
            headers[header::CONTENT_DISPOSITION],
            "attachment; filename=\"tat final.pdf\"; filename*=UTF-8''%C3%A9tat%20%22final%22.pdf"
        );
        assert_eq!(headers[header::CACHE_CONTROL], "private, no-store");
    }

    #[test]
    fn download_response_falls_back_on_bad_mime_types() {
        let response = file_response(AttachmentData {
            filename: None,
            mime_type: "bad\nvalue".into(),
            uid_validity: None,
            content: vec![],
        });
        let headers = response.headers();
        assert_eq!(headers[header::CONTENT_TYPE], "application/octet-stream");
        assert_eq!(
            headers[header::CONTENT_DISPOSITION],
            "attachment; filename=\"attachment\"; filename*=UTF-8''attachment"
        );
    }
}
