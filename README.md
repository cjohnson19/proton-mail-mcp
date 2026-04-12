# proton-mail-mcp

MCP server for Proton Mail. Connects via IMAP through [Proton Mail Bridge](https://proton.me/mail/bridge) and exposes email operations as MCP tools.

## Prerequisites

- [Rust](https://rustup.rs/)
- [Proton Mail Bridge](https://proton.me/mail/bridge) running on localhost

## Setup

```json
{
  "mcpServers": {
    "proton-mail": {
      "command": "cargo",
      "args": ["run", "--manifest-path", "/path/to/proton-mail-mcp/Cargo.toml"],
      "env": {
        "PROTON_USERNAME": "your-bridge-username",
        "PROTON_PASSWORD": "your-bridge-password"
      }
    }
  }
}
```

`PROTON_HOSTNAME` (default `127.0.0.1`) and `PROTON_PORT` (default `1143`) can also be set if needed.

## Tools

| Tool | Description |
|------|-------------|
| `list_mailboxes` | List all mailboxes |
| `list_messages` | List messages in a mailbox with pagination |
| `get_message` | Fetch a full message (headers, body, attachments) |
| `search_messages` | Search with filters (from, to, subject, body, date range, seen/flagged) |
| `get_mailbox_status` | Mailbox stats (count, recent, unseen) |
| `get_attachment` | Download an attachment (base64-encoded) |
| `list_messages_by_thread` | Find related messages by Message-ID or subject |
| `archive_message` | Move a message to Archive |
| `trash_message` | Move a message to Trash |
| `move_message` | Move a message to a different mailbox |
| `mark_as_read` | Mark a message as read or unread |
| `draft_email` | Save a draft to the Drafts mailbox, optionally as a threaded reply |

## Testing

Unit tests run without any external dependencies:

```sh
cargo test --lib
```

Integration tests require a running [GreenMail](https://greenmail-mail-test.github.io/greenmail/) IMAP server:

```sh
docker compose up -d
cargo test --test imap_integration
docker compose down
```

By default, tests connect to `127.0.0.1:3993` (IMAPS). Override with `TEST_HOST` and `TEST_IMAPS_PORT` if needed.
