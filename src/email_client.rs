use std::net::TcpStream;

use imap::{
    Session,
    types::{Name, NameAttribute},
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

#[allow(dead_code)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetEmailRequest {
    mailbox_name: String,
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
}
