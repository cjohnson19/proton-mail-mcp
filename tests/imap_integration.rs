mod helpers;

use helpers::{
    append_message, build_attachment_message, build_html_message, build_rfc2822_message,
    connect_client, create_mailbox, imap_session,
};
use proton_mail_mcp::email_client::*;

#[tokio::test]
async fn test_list_mailboxes() {
    let user = "listmbox@localhost";

    let mut client = connect_client(user).await;
    let resp = client.list_mailboxes().await.unwrap();

    let json: serde_json::Value = serde_json::to_value(&resp).unwrap();
    let mailboxes = json.as_array().unwrap();
    let names: Vec<&str> = mailboxes
        .iter()
        .filter_map(|m| m["name"].as_str())
        .collect();
    assert!(names.contains(&"INBOX"), "INBOX should be present, got: {names:?}");
}

#[tokio::test]
async fn test_list_messages() {
    let user = "listmsg@localhost";

    let mut raw = imap_session(user).await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message("sender@example.com", user, "Message 1", "Body 1", &[]),
    )
    .await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message("sender@example.com", user, "Message 2", "Body 2", &[]),
    )
    .await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message("sender@example.com", user, "Message 3", "Body 3", &[]),
    )
    .await;
    raw.logout().await.ok();

    let mut client = connect_client(user).await;
    let resp = client
        .list_messages(ListMessagesRequest {
            mailbox_name: "INBOX".into(),
            limit: 25,
            before_uid: None,
        })
        .await
        .unwrap();

    assert_eq!(resp.total_in_mailbox, 3);
    assert_eq!(resp.messages.len(), 3);
    assert!(resp.messages[0].uid > resp.messages[1].uid);
    assert!(resp.messages[1].uid > resp.messages[2].uid);
}

#[tokio::test]
async fn test_list_messages_pagination() {
    let user = "pagination@localhost";

    let mut raw = imap_session(user).await;
    for i in 1..=5 {
        append_message(
            &mut raw,
            "INBOX",
            &build_rfc2822_message(
                "sender@example.com",
                user,
                &format!("Page msg {i}"),
                &format!("Body {i}"),
                &[],
            ),
        )
        .await;
    }
    raw.logout().await.ok();

    let mut client = connect_client(user).await;

    let page1 = client
        .list_messages(ListMessagesRequest {
            mailbox_name: "INBOX".into(),
            limit: 3,
            before_uid: None,
        })
        .await
        .unwrap();
    assert_eq!(page1.messages.len(), 3);
    assert_eq!(page1.total_in_mailbox, 5);

    let lowest_uid = page1.messages.last().unwrap().uid;
    let page2 = client
        .list_messages(ListMessagesRequest {
            mailbox_name: "INBOX".into(),
            limit: 3,
            before_uid: Some(lowest_uid),
        })
        .await
        .unwrap();
    assert_eq!(page2.messages.len(), 2);

    let page1_uids: Vec<u32> = page1.messages.iter().map(|m| m.uid).collect();
    let page2_uids: Vec<u32> = page2.messages.iter().map(|m| m.uid).collect();
    assert!(
        page1_uids.iter().all(|uid| !page2_uids.contains(uid)),
        "pages should not overlap"
    );
}

#[tokio::test]
async fn test_get_message_text() {
    let user = "getmsg@localhost";

    let mut raw = imap_session(user).await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message(
            "alice@example.com",
            user,
            "Hello World",
            "This is the body text.",
            &[],
        ),
    )
    .await;
    raw.logout().await.ok();

    let mut client = connect_client(user).await;
    let list = client
        .list_messages(ListMessagesRequest {
            mailbox_name: "INBOX".into(),
            limit: 1,
            before_uid: None,
        })
        .await
        .unwrap();
    let uid = list.messages[0].uid;

    let resp = client
        .get_message(GetMessageRequest {
            mailbox_name: "INBOX".into(),
            uid,
            include_html: false,
        })
        .await
        .unwrap();

    assert_eq!(resp.uid, uid);
    assert_eq!(resp.subject.as_deref(), Some("Hello World"));
    assert!(
        resp.from.iter().any(|f| f.contains("alice@example.com")),
        "from should contain alice"
    );
    assert!(
        resp.body_text
            .as_deref()
            .unwrap()
            .contains("This is the body text."),
        "body_text should contain the message body"
    );
    assert!(resp.body_html.is_none());
}

#[tokio::test]
async fn test_get_message_html() {
    let user = "htmlmsg@localhost";

    let mut raw = imap_session(user).await;
    append_message(
        &mut raw,
        "INBOX",
        &build_html_message(
            "bob@example.com",
            user,
            "HTML Email",
            "<html><body><h1>Hello</h1><p>World</p></body></html>",
        ),
    )
    .await;
    raw.logout().await.ok();

    let mut client = connect_client(user).await;
    let list = client
        .list_messages(ListMessagesRequest {
            mailbox_name: "INBOX".into(),
            limit: 1,
            before_uid: None,
        })
        .await
        .unwrap();
    let uid = list.messages[0].uid;

    let resp = client
        .get_message(GetMessageRequest {
            mailbox_name: "INBOX".into(),
            uid,
            include_html: false,
        })
        .await
        .unwrap();

    assert!(resp.body_text.is_some(), "body_text should be auto-converted from HTML");
    let body = resp.body_text.unwrap();
    assert!(body.contains("Hello"), "converted text should contain 'Hello'");
    assert!(body.contains("World"), "converted text should contain 'World'");
}

#[tokio::test]
async fn test_search_by_subject() {
    let user = "searchsubj@localhost";

    let mut raw = imap_session(user).await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message("a@example.com", user, "Unique Needle Subject", "body", &[]),
    )
    .await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message("b@example.com", user, "Other Topic", "body", &[]),
    )
    .await;
    raw.logout().await.ok();

    let mut client = connect_client(user).await;
    let resp = client
        .search_messages(SearchMessagesRequest {
            mailbox_name: "INBOX".into(),
            from: None,
            to: None,
            subject: Some("Needle".into()),
            body: None,
            since: None,
            before: None,
            seen: None,
            flagged: None,
            limit: None,
        })
        .await
        .unwrap();

    assert_eq!(resp.total_matches, 1);
    assert_eq!(
        resp.messages[0].subject.as_deref(),
        Some("Unique Needle Subject")
    );
}

#[tokio::test]
async fn test_search_by_from() {
    let user = "searchfrom@localhost";

    let mut raw = imap_session(user).await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message("specific-sender@example.com", user, "From test", "body", &[]),
    )
    .await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message("other@example.com", user, "From test 2", "body", &[]),
    )
    .await;
    raw.logout().await.ok();

    let mut client = connect_client(user).await;
    let resp = client
        .search_messages(SearchMessagesRequest {
            mailbox_name: "INBOX".into(),
            from: Some("specific-sender@example.com".into()),
            to: None,
            subject: None,
            body: None,
            since: None,
            before: None,
            seen: None,
            flagged: None,
            limit: None,
        })
        .await
        .unwrap();

    assert_eq!(resp.total_matches, 1);
    assert!(resp.messages[0]
        .from
        .iter()
        .any(|f| f.contains("specific-sender")));
}

#[tokio::test]
async fn test_get_mailbox_status() {
    let user = "mboxstat@localhost";

    let mut raw = imap_session(user).await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message("a@example.com", user, "Status 1", "body", &[]),
    )
    .await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message("b@example.com", user, "Status 2", "body", &[]),
    )
    .await;
    raw.logout().await.ok();

    let mut client = connect_client(user).await;
    let resp = client
        .get_mailbox_status(GetMailboxStatusRequest {
            mailbox_name: "INBOX".into(),
        })
        .await
        .unwrap();

    assert_eq!(resp.messages, 2);
    assert_eq!(resp.unseen, Some(2));
}

#[tokio::test]
async fn test_get_attachment() {
    let user = "attach@localhost";

    let attachment_data = b"Hello attachment content!";
    let mut raw = imap_session(user).await;
    append_message(
        &mut raw,
        "INBOX",
        &build_attachment_message(
            "sender@example.com",
            user,
            "With attachment",
            "See attached",
            "test.txt",
            attachment_data,
        ),
    )
    .await;
    raw.logout().await.ok();

    let mut client = connect_client(user).await;
    let list = client
        .list_messages(ListMessagesRequest {
            mailbox_name: "INBOX".into(),
            limit: 1,
            before_uid: None,
        })
        .await
        .unwrap();
    let uid = list.messages[0].uid;

    let resp = client
        .get_attachment(GetAttachmentRequest {
            mailbox_name: "INBOX".into(),
            uid,
            attachment_index: 0,
        })
        .await
        .unwrap();

    assert_eq!(resp.filename.as_deref(), Some("test.txt"));
    let decoded = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &resp.content_base64,
    )
    .unwrap();
    assert_eq!(decoded, attachment_data);
}

#[tokio::test]
async fn test_list_messages_by_thread() {
    let user = "thread@localhost";

    let msg_id = "<original-123@test>";
    let mut raw = imap_session(user).await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message(
            "alice@example.com",
            user,
            "Thread subject",
            "Original message",
            &[("Message-ID", msg_id)],
        ),
    )
    .await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message(
            "bob@example.com",
            user,
            "Re: Thread subject",
            "Reply message",
            &[
                ("Message-ID", "<reply-456@test>"),
                ("In-Reply-To", msg_id),
                ("References", msg_id),
            ],
        ),
    )
    .await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message("other@example.com", user, "Unrelated", "no thread", &[]),
    )
    .await;
    raw.logout().await.ok();

    let mut client = connect_client(user).await;
    let resp = client
        .list_messages_by_thread(ListMessagesByThreadRequest {
            mailbox_name: "INBOX".into(),
            message_id: Some(msg_id.into()),
            subject: None,
        })
        .await
        .unwrap();

    assert_eq!(resp.total_in_thread, 2, "thread should have 2 messages");
    assert!(resp.messages.iter().any(|m| m
        .subject
        .as_deref()
        .is_some_and(|s| s.contains("Thread subject"))));
    assert!(resp
        .messages
        .iter()
        .any(|m| m.in_reply_to.as_deref() == Some(msg_id)));
}

#[tokio::test]
async fn test_archive_message() {
    let user = "archive@localhost";

    let mut raw = imap_session(user).await;
    create_mailbox(&mut raw, "Archive").await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message("a@example.com", user, "To archive", "body", &[]),
    )
    .await;
    raw.logout().await.ok();

    let mut client = connect_client(user).await;
    let list = client
        .list_messages(ListMessagesRequest {
            mailbox_name: "INBOX".into(),
            limit: 1,
            before_uid: None,
        })
        .await
        .unwrap();
    let uid = list.messages[0].uid;

    let resp = client
        .archive_message(ArchiveMessageRequest {
            mailbox_name: "INBOX".into(),
            uid,
            subject: None,
        })
        .await
        .unwrap();

    assert_eq!(resp.source_mailbox, "INBOX");
    assert_eq!(resp.destination_mailbox, "Archive");

    let status = client
        .get_mailbox_status(GetMailboxStatusRequest {
            mailbox_name: "INBOX".into(),
        })
        .await
        .unwrap();
    assert_eq!(status.messages, 0);

    let archive_list = client
        .list_messages(ListMessagesRequest {
            mailbox_name: "Archive".into(),
            limit: 10,
            before_uid: None,
        })
        .await
        .unwrap();
    assert_eq!(archive_list.messages.len(), 1);
}

#[tokio::test]
async fn test_trash_message() {
    let user = "trash@localhost";

    let mut raw = imap_session(user).await;
    create_mailbox(&mut raw, "Trash").await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message("a@example.com", user, "To trash", "body", &[]),
    )
    .await;
    raw.logout().await.ok();

    let mut client = connect_client(user).await;
    let list = client
        .list_messages(ListMessagesRequest {
            mailbox_name: "INBOX".into(),
            limit: 1,
            before_uid: None,
        })
        .await
        .unwrap();
    let uid = list.messages[0].uid;

    let resp = client
        .trash_message(TrashMessageRequest {
            source_mailbox: "INBOX".into(),
            uid,
            subject: None,
        })
        .await
        .unwrap();

    assert_eq!(resp.source_mailbox, "INBOX");

    let status = client
        .get_mailbox_status(GetMailboxStatusRequest {
            mailbox_name: "INBOX".into(),
        })
        .await
        .unwrap();
    assert_eq!(status.messages, 0);

    let trash_list = client
        .list_messages(ListMessagesRequest {
            mailbox_name: "Trash".into(),
            limit: 10,
            before_uid: None,
        })
        .await
        .unwrap();
    assert_eq!(trash_list.messages.len(), 1);
}

#[tokio::test]
async fn test_move_message() {
    let user = "movemsg@localhost";

    let mut raw = imap_session(user).await;
    create_mailbox(&mut raw, "CustomFolder").await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message("a@example.com", user, "To move", "body", &[]),
    )
    .await;
    raw.logout().await.ok();

    let mut client = connect_client(user).await;
    let list = client
        .list_messages(ListMessagesRequest {
            mailbox_name: "INBOX".into(),
            limit: 1,
            before_uid: None,
        })
        .await
        .unwrap();
    let uid = list.messages[0].uid;

    let resp = client
        .move_message(MoveMessageRequest {
            source_mailbox: "INBOX".into(),
            destination_mailbox: "CustomFolder".into(),
            uid,
            subject: None,
        })
        .await
        .unwrap();

    assert_eq!(resp.source_mailbox, "INBOX");
    assert_eq!(resp.destination_mailbox, "CustomFolder");

    let status = client
        .get_mailbox_status(GetMailboxStatusRequest {
            mailbox_name: "INBOX".into(),
        })
        .await
        .unwrap();
    assert_eq!(status.messages, 0);

    let dest_list = client
        .list_messages(ListMessagesRequest {
            mailbox_name: "CustomFolder".into(),
            limit: 10,
            before_uid: None,
        })
        .await
        .unwrap();
    assert_eq!(dest_list.messages.len(), 1);
}

#[tokio::test]
async fn test_move_to_trash_blocked() {
    let user = "moveblock@localhost";

    let mut raw = imap_session(user).await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message("a@example.com", user, "Blocked", "body", &[]),
    )
    .await;
    raw.logout().await.ok();

    let mut client = connect_client(user).await;
    let list = client
        .list_messages(ListMessagesRequest {
            mailbox_name: "INBOX".into(),
            limit: 1,
            before_uid: None,
        })
        .await
        .unwrap();
    let uid = list.messages[0].uid;

    let result = client
        .move_message(MoveMessageRequest {
            source_mailbox: "INBOX".into(),
            destination_mailbox: "Trash".into(),
            uid,
            subject: None,
        })
        .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not allowed"));
}

#[tokio::test]
async fn test_mark_as_read() {
    let user = "markread@localhost";

    let mut raw = imap_session(user).await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message("a@example.com", user, "Read test", "body", &[]),
    )
    .await;
    raw.logout().await.ok();

    let mut client = connect_client(user).await;
    let list = client
        .list_messages(ListMessagesRequest {
            mailbox_name: "INBOX".into(),
            limit: 1,
            before_uid: None,
        })
        .await
        .unwrap();
    let uid = list.messages[0].uid;

    let status = client
        .get_mailbox_status(GetMailboxStatusRequest {
            mailbox_name: "INBOX".into(),
        })
        .await
        .unwrap();
    assert_eq!(status.unseen, Some(1));

    let resp = client
        .mark_as_read(MarkAsReadRequest {
            mailbox_name: "INBOX".into(),
            uid,
            read: true,
        })
        .await
        .unwrap();
    assert!(resp.read);

    let status = client
        .get_mailbox_status(GetMailboxStatusRequest {
            mailbox_name: "INBOX".into(),
        })
        .await
        .unwrap();
    assert_eq!(status.unseen, Some(0));
}

#[tokio::test]
async fn test_mark_as_unread() {
    let user = "markunread@localhost";

    let mut raw = imap_session(user).await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message("a@example.com", user, "Unread test", "body", &[]),
    )
    .await;
    raw.logout().await.ok();

    let mut client = connect_client(user).await;
    let list = client
        .list_messages(ListMessagesRequest {
            mailbox_name: "INBOX".into(),
            limit: 1,
            before_uid: None,
        })
        .await
        .unwrap();
    let uid = list.messages[0].uid;

    client
        .mark_as_read(MarkAsReadRequest {
            mailbox_name: "INBOX".into(),
            uid,
            read: true,
        })
        .await
        .unwrap();

    let resp = client
        .mark_as_read(MarkAsReadRequest {
            mailbox_name: "INBOX".into(),
            uid,
            read: false,
        })
        .await
        .unwrap();
    assert!(!resp.read);

    let status = client
        .get_mailbox_status(GetMailboxStatusRequest {
            mailbox_name: "INBOX".into(),
        })
        .await
        .unwrap();
    assert_eq!(status.unseen, Some(1));
}
