mod helpers;

use helpers::{
    append_message, build_attachment_message, build_html_message, build_rfc2822_message,
    connect_client, create_mailbox, ensure_mailbox, imap_session,
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
    assert!(
        names.contains(&"INBOX"),
        "INBOX should be present, got: {names:?}"
    );
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
            include_snippet: false,
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
            include_snippet: false,
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
            include_snippet: false,
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
            include_snippet: false,
        })
        .await
        .unwrap();
    let uid = list.messages[0].uid;

    let resp = client
        .get_message(GetMessageRequest {
            mailbox_name: "INBOX".into(),
            uid,
            include_html: false,
            max_body_chars: None,
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
            include_snippet: false,
        })
        .await
        .unwrap();
    let uid = list.messages[0].uid;

    let resp = client
        .get_message(GetMessageRequest {
            mailbox_name: "INBOX".into(),
            uid,
            include_html: false,
            max_body_chars: None,
        })
        .await
        .unwrap();

    assert!(
        resp.body_text.is_some(),
        "body_text should be auto-converted from HTML"
    );
    let body = resp.body_text.unwrap();
    assert!(
        body.contains("Hello"),
        "converted text should contain 'Hello'"
    );
    assert!(
        body.contains("World"),
        "converted text should contain 'World'"
    );
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
            before_uid: None,
            limit: None,
            include_snippet: false,
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
        &build_rfc2822_message(
            "specific-sender@example.com",
            user,
            "From test",
            "body",
            &[],
        ),
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
            before_uid: None,
            limit: None,
            include_snippet: false,
        })
        .await
        .unwrap();

    assert_eq!(resp.total_matches, 1);
    assert!(
        resp.messages[0]
            .from
            .iter()
            .any(|f| f.contains("specific-sender"))
    );
}

#[tokio::test]
async fn test_search_messages_pagination_preserves_filters() {
    let user = "searchpagination@localhost";

    let mut raw = imap_session(user).await;
    for i in 1..=4 {
        append_message(
            &mut raw,
            "INBOX",
            &build_rfc2822_message(
                "sender@example.com",
                user,
                &format!("Matching topic {i}"),
                "body",
                &[],
            ),
        )
        .await;
    }
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message("sender@example.com", user, "Unrelated", "body", &[]),
    )
    .await;
    raw.logout().await.ok();

    let request = |before_uid| SearchMessagesRequest {
        mailbox_name: "INBOX".into(),
        from: None,
        to: None,
        subject: Some("Matching topic".into()),
        body: None,
        since: None,
        before: None,
        seen: None,
        flagged: None,
        before_uid,
        limit: Some(2),
        include_snippet: false,
    };
    let mut client = connect_client(user).await;

    let page1 = client.search_messages(request(None)).await.unwrap();
    let page2 = client
        .search_messages(request(page1.next_cursor))
        .await
        .unwrap();

    assert_eq!(page1.total_matches, 4);
    assert_eq!(page2.total_matches, 4);
    assert_eq!(page1.messages.len(), 2);
    assert_eq!(page2.messages.len(), 2);
    assert!(page1.next_cursor.is_some());
    assert_eq!(page2.next_cursor, None);
    assert!(page1.messages.iter().chain(&page2.messages).all(|message| {
        message
            .subject
            .as_deref()
            .is_some_and(|subject| subject.contains("Matching topic"))
    }));
    assert!(
        page1
            .messages
            .iter()
            .all(|first| { page2.messages.iter().all(|second| first.uid != second.uid) })
    );
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
            include_snippet: false,
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
            uid: None,
            message_id: Some(msg_id.into()),
            subject: None,
        })
        .await
        .unwrap();

    assert_eq!(resp.total_in_thread, 2, "thread should have 2 messages");
    assert!(resp.messages.iter().any(|m| {
        m.subject
            .as_deref()
            .is_some_and(|s| s.contains("Thread subject"))
    }));
    assert!(
        resp.messages
            .iter()
            .any(|m| m.in_reply_to.as_deref() == Some(msg_id))
    );

    let reply_uid = resp
        .messages
        .iter()
        .find(|message| message.in_reply_to.as_deref() == Some(msg_id))
        .unwrap()
        .uid;
    let by_uid = client
        .list_messages_by_thread(ListMessagesByThreadRequest {
            mailbox_name: "INBOX".into(),
            uid: Some(reply_uid),
            message_id: None,
            subject: None,
        })
        .await
        .unwrap();
    assert_eq!(by_uid.total_in_thread, 2);
    assert_eq!(
        by_uid
            .messages
            .iter()
            .map(|message| message.uid)
            .collect::<Vec<_>>(),
        resp.messages
            .iter()
            .map(|message| message.uid)
            .collect::<Vec<_>>()
    );
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
            include_snippet: false,
        })
        .await
        .unwrap();
    let uid = list.messages[0].uid;

    let resp = client
        .archive_message(ArchiveMessageRequest {
            source_mailbox: "INBOX".into(),
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
            include_snippet: false,
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
            include_snippet: false,
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
            include_snippet: false,
        })
        .await
        .unwrap();
    assert_eq!(trash_list.messages.len(), 1);
}

async fn seed_inbox(user: &str, folder: &str, subjects: &[&str]) {
    let mut raw = imap_session(user).await;
    create_mailbox(&mut raw, folder).await;
    for subject in subjects {
        append_message(
            &mut raw,
            "INBOX",
            &build_rfc2822_message("a@example.com", user, subject, "body", &[]),
        )
        .await;
    }
    raw.logout().await.ok();
}

async fn inbox_uids(client: &mut EmailClient) -> Vec<u32> {
    client
        .list_messages(ListMessagesRequest {
            mailbox_name: "INBOX".into(),
            limit: 10,
            before_uid: None,
            include_snippet: false,
        })
        .await
        .unwrap()
        .messages
        .iter()
        .map(|m| m.uid)
        .collect()
}

async fn mailbox_count(client: &mut EmailClient, mailbox: &str) -> u32 {
    client
        .get_mailbox_status(GetMailboxStatusRequest {
            mailbox_name: mailbox.into(),
        })
        .await
        .unwrap()
        .messages
}

fn batch_request(uids: &[u32]) -> MessageBatchRequest {
    MessageBatchRequest {
        source_mailbox: "INBOX".into(),
        messages: uids
            .iter()
            .map(|uid| MessageRef {
                uid: *uid,
                subject: None,
            })
            .collect(),
    }
}

#[tokio::test]
async fn test_archive_messages_batch() {
    let user = "archivebatch@localhost";
    seed_inbox(user, "Archive", &["First", "Second", "Third"]).await;

    let mut client = connect_client(user).await;
    let uids = inbox_uids(&mut client).await;
    assert_eq!(uids.len(), 3);
    let keep = *uids.iter().min().unwrap();
    let mut to_archive: Vec<u32> = uids.into_iter().filter(|uid| *uid != keep).collect();
    to_archive.sort_unstable();

    let resp = client
        .archive_messages(batch_request(&[to_archive[1], to_archive[0], 9999]))
        .await
        .unwrap();

    assert_eq!(resp.source_mailbox, "INBOX");
    assert_eq!(resp.destination_mailbox, "Archive");
    assert_eq!(resp.moved, to_archive);
    assert_eq!(resp.not_found, vec![9999]);

    assert_eq!(mailbox_count(&mut client, "INBOX").await, 1);
    assert_eq!(mailbox_count(&mut client, "Archive").await, 2);
    assert_eq!(inbox_uids(&mut client).await, vec![keep]);
}

#[tokio::test]
async fn test_trash_messages_batch() {
    let user = "trashbatch@localhost";
    seed_inbox(user, "Trash", &["One", "Two"]).await;

    let mut client = connect_client(user).await;
    let mut uids = inbox_uids(&mut client).await;
    uids.sort_unstable();
    assert_eq!(uids.len(), 2);

    let resp = client.trash_messages(batch_request(&uids)).await.unwrap();

    assert_eq!(resp.destination_mailbox, "Trash");
    assert_eq!(resp.moved, uids);
    assert!(resp.not_found.is_empty());
    assert_eq!(mailbox_count(&mut client, "INBOX").await, 0);
    assert_eq!(mailbox_count(&mut client, "Trash").await, 2);

    let again = client.trash_messages(batch_request(&uids)).await.unwrap();
    assert!(again.moved.is_empty());
    assert_eq!(again.not_found, uids);
}

#[tokio::test]
async fn test_trash_messages_rejects_empty() {
    let mut client = connect_client("trashbatchempty@localhost").await;
    let err = client.trash_messages(batch_request(&[])).await.unwrap_err();
    assert!(err.contains("at least one"), "unexpected error: {err}");
}

fn move_batch_request(destination: &str, uids: &[u32]) -> MoveMessagesRequest {
    MoveMessagesRequest {
        source_mailbox: "INBOX".into(),
        destination_mailbox: destination.into(),
        messages: batch_request(uids).messages,
    }
}

#[tokio::test]
async fn test_move_messages_batch() {
    let user = "movebatch@localhost";
    seed_inbox(user, "Receipts", &["Invoice 1", "Invoice 2", "Unrelated"]).await;

    let mut client = connect_client(user).await;
    let mut uids = inbox_uids(&mut client).await;
    uids.sort_unstable();
    assert_eq!(uids.len(), 3);
    let (keep, to_move) = (uids[2], &uids[..2]);

    let resp = client
        .move_messages(move_batch_request(
            "Receipts",
            &[to_move[0], to_move[1], 4242],
        ))
        .await
        .unwrap();

    assert_eq!(resp.source_mailbox, "INBOX");
    assert_eq!(resp.destination_mailbox, "Receipts");
    assert_eq!(resp.moved, to_move);
    assert_eq!(resp.not_found, vec![4242]);
    assert_eq!(mailbox_count(&mut client, "INBOX").await, 1);
    assert_eq!(mailbox_count(&mut client, "Receipts").await, 2);
    assert_eq!(inbox_uids(&mut client).await, vec![keep]);
}

#[tokio::test]
async fn test_move_messages_to_trash_blocked() {
    let user = "movebatchblock@localhost";
    seed_inbox(user, "Trash", &["Blocked"]).await;

    let mut client = connect_client(user).await;
    let uids = inbox_uids(&mut client).await;

    let err = client
        .move_messages(move_batch_request("trash", &uids))
        .await
        .unwrap_err();

    assert!(err.contains("not allowed"), "unexpected error: {err}");
    assert_eq!(mailbox_count(&mut client, "INBOX").await, 1);
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
            include_snippet: false,
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
            include_snippet: false,
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
            include_snippet: false,
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
            include_snippet: false,
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
            source_mailbox: "INBOX".into(),
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
            include_snippet: false,
        })
        .await
        .unwrap();
    let uid = list.messages[0].uid;

    client
        .mark_as_read(MarkAsReadRequest {
            source_mailbox: "INBOX".into(),
            uid,
            read: true,
        })
        .await
        .unwrap();

    let resp = client
        .mark_as_read(MarkAsReadRequest {
            source_mailbox: "INBOX".into(),
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

#[tokio::test]
async fn test_set_flagged_and_unflagged() {
    let user = "setflagged@localhost";

    let mut raw = imap_session(user).await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message("a@example.com", user, "Flag test", "body", &[]),
    )
    .await;
    raw.logout().await.ok();

    let mut client = connect_client(user).await;
    let list = client
        .list_messages(ListMessagesRequest {
            mailbox_name: "INBOX".into(),
            limit: 1,
            before_uid: None,
            include_snippet: false,
        })
        .await
        .unwrap();
    let uid = list.messages[0].uid;

    let flagged = client
        .set_flagged(SetFlaggedRequest {
            source_mailbox: "INBOX".into(),
            uid,
            flagged: true,
        })
        .await
        .unwrap();
    assert!(flagged.flagged);

    let list = client
        .list_messages(ListMessagesRequest {
            mailbox_name: "INBOX".into(),
            limit: 1,
            before_uid: None,
            include_snippet: false,
        })
        .await
        .unwrap();
    assert!(list.messages[0].flags.contains(&MessageFlag::Flagged));

    let unflagged = client
        .set_flagged(SetFlaggedRequest {
            source_mailbox: "INBOX".into(),
            uid,
            flagged: false,
        })
        .await
        .unwrap();
    assert!(!unflagged.flagged);

    let list = client
        .list_messages(ListMessagesRequest {
            mailbox_name: "INBOX".into(),
            limit: 1,
            before_uid: None,
            include_snippet: false,
        })
        .await
        .unwrap();
    assert!(!list.messages[0].flags.contains(&MessageFlag::Flagged));
}

fn basic_draft_request(to: &str, subject: &str, body: &str) -> DraftEmailRequest {
    DraftEmailRequest {
        from: None,
        to: vec![to.into()],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: subject.into(),
        body_text: body.into(),
        in_reply_to: None,
        drafts_mailbox: "Drafts".into(),
    }
}

#[tokio::test]
async fn test_draft_email_basic() {
    let user = "drafts@localhost";

    let mut raw = imap_session(user).await;
    ensure_mailbox(&mut raw, "Drafts").await;
    raw.logout().await.ok();

    let mut client = connect_client(user).await;
    let resp = client
        .draft_email(basic_draft_request(
            "recipient@example.com",
            "Draft subject",
            "Hello, this is a draft.",
        ))
        .await
        .unwrap();

    assert_eq!(resp.drafts_mailbox, "Drafts");
    assert!(resp.message_id.starts_with('<') && resp.message_id.ends_with('>'));
    assert!(!resp.threaded);

    let list = client
        .list_messages(ListMessagesRequest {
            mailbox_name: "Drafts".into(),
            limit: 10,
            before_uid: None,
            include_snippet: false,
        })
        .await
        .unwrap();
    assert_eq!(list.messages.len(), 1);
    let summary = &list.messages[0];
    assert_eq!(summary.subject.as_deref(), Some("Draft subject"));
    assert!(
        summary.flags.contains(&MessageFlag::Draft),
        "expected \\Draft flag, got: {:?}",
        summary.flags
    );

    let full = client
        .get_message(GetMessageRequest {
            mailbox_name: "Drafts".into(),
            uid: summary.uid,
            include_html: false,
            max_body_chars: None,
        })
        .await
        .unwrap();
    assert_eq!(full.subject.as_deref(), Some("Draft subject"));
    assert!(
        full.from.iter().any(|f| f.contains(user)),
        "From should default to the login user, got: {:?}",
        full.from
    );
    assert!(
        full.to.iter().any(|t| t.contains("recipient@example.com")),
        "To should contain recipient, got: {:?}",
        full.to
    );
    assert!(
        full.body_text
            .as_deref()
            .is_some_and(|b| b.contains("Hello, this is a draft.")),
        "body_text missing expected content: {:?}",
        full.body_text
    );
}

#[tokio::test]
async fn test_draft_email_reply_threading() {
    let user = "draftreply@localhost";
    let original_msg_id = "<orig-12345@test.example>";

    let mut raw = imap_session(user).await;
    ensure_mailbox(&mut raw, "Drafts").await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message(
            "sender@example.com",
            user,
            "Original subject",
            "original body",
            &[("Message-ID", original_msg_id)],
        ),
    )
    .await;
    raw.logout().await.ok();

    let mut client = connect_client(user).await;
    let inbox_list = client
        .list_messages(ListMessagesRequest {
            mailbox_name: "INBOX".into(),
            limit: 1,
            before_uid: None,
            include_snippet: false,
        })
        .await
        .unwrap();
    let orig_uid = inbox_list.messages[0].uid;

    let resp = client
        .draft_email(DraftEmailRequest {
            from: None,
            to: vec!["sender@example.com".into()],
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: "Re: Original subject".into(),
            body_text: "My reply.".into(),
            in_reply_to: Some(InReplyTo {
                mailbox_name: "INBOX".into(),
                uid: orig_uid,
            }),
            drafts_mailbox: "Drafts".into(),
        })
        .await
        .unwrap();
    assert!(resp.threaded, "draft should be threaded");

    let thread = client
        .list_messages_by_thread(ListMessagesByThreadRequest {
            mailbox_name: "Drafts".into(),
            uid: None,
            message_id: Some(original_msg_id.into()),
            subject: None,
        })
        .await
        .unwrap();
    assert_eq!(
        thread.messages.len(),
        1,
        "expected the draft to match thread search, got: {:?}",
        thread.messages
    );
    let draft = &thread.messages[0];
    assert_eq!(
        draft.in_reply_to.as_deref(),
        Some(original_msg_id),
        "In-Reply-To should be the original Message-ID"
    );
    assert!(
        draft
            .references
            .as_deref()
            .is_some_and(|r| r.contains(original_msg_id)),
        "References should contain the original Message-ID, got: {:?}",
        draft.references
    );
}

#[tokio::test]
async fn test_draft_email_from_override() {
    let user = "draftfrom@localhost";

    let mut raw = imap_session(user).await;
    ensure_mailbox(&mut raw, "Drafts").await;
    raw.logout().await.ok();

    let mut client = connect_client(user).await;
    client
        .draft_email(DraftEmailRequest {
            from: Some("alias@example.com".into()),
            to: vec!["recipient@example.com".into()],
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: "From override".into(),
            body_text: "body".into(),
            in_reply_to: None,
            drafts_mailbox: "Drafts".into(),
        })
        .await
        .unwrap();

    let list = client
        .list_messages(ListMessagesRequest {
            mailbox_name: "Drafts".into(),
            limit: 1,
            before_uid: None,
            include_snippet: false,
        })
        .await
        .unwrap();
    let uid = list.messages[0].uid;

    let full = client
        .get_message(GetMessageRequest {
            mailbox_name: "Drafts".into(),
            uid,
            include_html: false,
            max_body_chars: None,
        })
        .await
        .unwrap();
    assert!(
        full.from.iter().any(|f| f.contains("alias@example.com")),
        "From should be the override, got: {:?}",
        full.from
    );
    assert!(
        !full.from.iter().any(|f| f.contains(user)),
        "From should NOT contain the default login user, got: {:?}",
        full.from
    );
}

#[tokio::test]
async fn test_reconnects_after_connection_loss() {
    let user = "reconnect@localhost";

    let mut raw = imap_session(user).await;
    append_message(
        &mut raw,
        "INBOX",
        &build_rfc2822_message("sender@example.com", user, "Survivor", "still here", &[]),
    )
    .await;
    raw.logout().await.ok();

    let proxy = helpers::TcpProxy::start_to_imaps().await;
    let mut client = EmailClient::new_direct_tls("127.0.0.1", proxy.port, user, user)
        .await
        .expect("initial connect through proxy failed");

    let live = client
        .get_mailbox_status(GetMailboxStatusRequest {
            mailbox_name: "INBOX".into(),
        })
        .await
        .expect("status should succeed on a live connection");
    assert_eq!(live.messages, 1);

    let severed = proxy.sever().await;
    assert!(
        severed >= 1,
        "proxy should have severed the live connection"
    );

    // A dead session must not surface as a successful-but-empty response
    // (async-imap treats EOF as an empty result) — it must reconnect and
    // return real data.
    let resp = client
        .get_mailbox_status(GetMailboxStatusRequest {
            mailbox_name: "INBOX".into(),
        })
        .await
        .expect("client should transparently reconnect after the connection dropped");
    assert_eq!(resp.mailbox_name, "INBOX");
    assert_eq!(resp.messages, 1, "reconnected session should see real data");
    assert!(resp.uid_validity.is_some());
}
