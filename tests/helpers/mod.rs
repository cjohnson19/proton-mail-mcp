use std::sync::Arc;

use async_imap::Session;
use proton_mail_mcp::email_client::EmailClient;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio_native_tls::TlsStream;

fn test_host() -> String {
    std::env::var("TEST_HOST").unwrap_or_else(|_| "127.0.0.1".into())
}

fn test_imaps_port() -> u16 {
    std::env::var("TEST_IMAPS_PORT")
        .unwrap_or_else(|_| "3993".into())
        .parse()
        .unwrap_or(3993)
}

/// TCP proxy in front of the IMAP server whose established connections can be
/// severed mid-test to simulate the server dropping the connection.
pub struct TcpProxy {
    pub port: u16,
    conns: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl TcpProxy {
    pub async fn start_to_imaps() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("proxy bind failed");
        let port = listener.local_addr().expect("proxy local_addr").port();
        let conns: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>> = Arc::default();

        let accept_conns = conns.clone();
        tokio::spawn(async move {
            while let Ok((mut inbound, _)) = listener.accept().await {
                let target = (test_host(), test_imaps_port());
                let handle = tokio::spawn(async move {
                    let Ok(mut outbound) = TcpStream::connect(target).await else {
                        return;
                    };
                    let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                });
                accept_conns.lock().await.push(handle);
            }
        });

        Self { port, conns }
    }

    /// Kill all established connections; new connections are still accepted.
    /// Returns how many connections were severed.
    pub async fn sever(&self) -> usize {
        let handles: Vec<_> = self.conns.lock().await.drain(..).collect();
        let count = handles.len();
        for handle in handles {
            handle.abort();
            let _ = handle.await;
        }
        count
    }
}

pub async fn connect_client(user: &str) -> EmailClient {
    EmailClient::new_direct_tls(&test_host(), test_imaps_port(), user, user)
        .await
        .expect("failed to connect EmailClient")
}

pub async fn imap_session(user: &str) -> Session<TlsStream<TcpStream>> {
    let host = test_host();
    let port = test_imaps_port();

    let tcp = TcpStream::connect((&*host, port))
        .await
        .expect("TCP connect failed");

    let tls_connector = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .expect("TLS connector build failed");
    let tls_connector = tokio_native_tls::TlsConnector::from(tls_connector);
    let tls_stream = tls_connector
        .connect(&host, tcp)
        .await
        .expect("TLS upgrade failed");

    let client = async_imap::Client::new(tls_stream);
    client
        .login(user, user)
        .await
        .map_err(|e| e.0)
        .expect("IMAP login failed")
}

pub async fn append_message(
    session: &mut Session<TlsStream<TcpStream>>,
    mailbox: &str,
    raw_rfc2822: &[u8],
) {
    session
        .append(mailbox, None, None, raw_rfc2822)
        .await
        .expect("IMAP APPEND failed");
}

pub async fn create_mailbox(session: &mut Session<TlsStream<TcpStream>>, name: &str) {
    session.create(name).await.expect("IMAP CREATE failed");
}

pub async fn ensure_mailbox(session: &mut Session<TlsStream<TcpStream>>, name: &str) {
    if let Err(e) = session.create(name).await {
        let msg = e.to_string();
        if !msg.to_lowercase().contains("already exists") {
            panic!("IMAP CREATE {name} failed: {msg}");
        }
    }
}

pub fn build_rfc2822_message(
    from: &str,
    to: &str,
    subject: &str,
    body: &str,
    extras: &[(&str, &str)],
) -> Vec<u8> {
    let mut headers = format!(
        "From: {from}\r\n\
         To: {to}\r\n\
         Subject: {subject}\r\n"
    );
    for (key, value) in extras {
        headers.push_str(&format!("{key}: {value}\r\n"));
    }
    if !extras
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("Content-Type"))
    {
        headers.push_str("Content-Type: text/plain; charset=utf-8\r\n");
    }
    headers.push_str(&format!(
        "Date: Mon, 01 Jan 2024 12:00:00 +0000\r\n\r\n{body}"
    ));
    headers.into_bytes()
}

pub fn build_html_message(from: &str, to: &str, subject: &str, html_body: &str) -> Vec<u8> {
    format!(
        "From: {from}\r\n\
         To: {to}\r\n\
         Subject: {subject}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Date: Mon, 01 Jan 2024 12:00:00 +0000\r\n\
         \r\n\
         {html_body}"
    )
    .into_bytes()
}

pub fn build_attachment_message(
    from: &str,
    to: &str,
    subject: &str,
    body_text: &str,
    attachment_filename: &str,
    attachment_content: &[u8],
) -> Vec<u8> {
    let boundary = "BOUNDARY_TEST_12345";
    let b64_content = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        attachment_content,
    );
    format!(
        "From: {from}\r\n\
         To: {to}\r\n\
         Subject: {subject}\r\n\
         Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\
         Date: Mon, 01 Jan 2024 12:00:00 +0000\r\n\
         \r\n\
         --{boundary}\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         \r\n\
         {body_text}\r\n\
         --{boundary}\r\n\
         Content-Type: application/octet-stream\r\n\
         Content-Disposition: attachment; filename=\"{attachment_filename}\"\r\n\
         Content-Transfer-Encoding: base64\r\n\
         \r\n\
         {b64_content}\r\n\
         --{boundary}--\r\n"
    )
    .into_bytes()
}
