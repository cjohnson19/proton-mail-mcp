# proton-mail-mcp

MCP server for Proton Mail. Connects via IMAP through [Proton Mail Bridge](https://proton.me/mail/bridge) and exposes email operations as MCP tools.

## Prerequisites

- [Rust](https://rustup.rs/)
- [Proton Mail Bridge](https://proton.me/mail/bridge) running on localhost

## Setup

For local use the server runs over stdio; point your MCP client at the
binary with the Bridge credentials in its environment. Build it once with
`cargo build --release` (binary at `target/release/proton-mail-mcp`), or let
the client run it through `cargo run` as below. See [Claude setup](#claude-setup)
for where this configuration goes.

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

### Attachments

`get_attachment` embeds attachments up to `MCP_ATTACHMENT_INLINE_MAX_BYTES`
(default 16 KiB) as base64. Larger ones, or any request with
`delivery: "link"`, are handed back out of band so the size is not limited
by the MCP tool-result channel:

- **stdio:** the bytes are written under `MCP_ATTACHMENT_DIR` (default
  `$TMPDIR/proton-mail-mcp`) and the response carries `file_path`.
- **HTTP:** the response carries a signed `download_url`
  (`/attachments/{token}`, valid for one hour, no OAuth token needed) that
  streams the raw bytes with the right `Content-Type`. Links are signed with
  a key derived from `MCP_OAUTH_SIGNING_KEY`, or a per-process random key
  when OAuth is off.

Either way the result also includes a `resource_link` content block.

To read a document rather than fetch it, pass `delivery: "text"`: the server
extracts text from PDF, HTML, and plain-text attachments (txt, csv, json,
ics, ...) and returns it in `text`, paged with `text_offset` /
`max_text_chars` (default 40 000 characters) and `text_next_offset`.

## Remote endpoint (HTTP + OAuth)

Setting `MCP_BIND` (e.g. `0.0.0.0:8080`) serves streamable-HTTP at `/mcp`
instead of stdio. For anything reachable from the internet, also set the
OAuth trio — the server then acts as its own OAuth 2.1 authorization server
(dynamic client registration, PKCE, passphrase-gated approval page) and
requires a Bearer token on `/mcp`:

```sh
MCP_BIND=0.0.0.0:8080
MCP_PUBLIC_URL=https://your-domain.example      # issuer + token audience
MCP_OAUTH_PASSPHRASE='entered once when authorizing a client (12+ chars)'
MCP_OAUTH_SIGNING_KEY="$(openssl rand -hex 32)" # rotating revokes all tokens
```

Clients then connect to `https://your-domain.example/mcp`; see
[Claude setup](#claude-setup) and [ChatGPT setup](#chatgpt-setup).
`MCP_OAUTH_REDIRECT_ALLOWLIST` (comma-separated) admits additional OAuth
callbacks beyond Claude, ChatGPT's stable callback, and loopback.

A ready-made AWS deployment (one EC2 instance running Bridge, the server,
and Caddy for TLS, with secrets in SSM Parameter Store) lives in
[`deploy/aws`](deploy/aws/README.md).

## Claude setup

### Claude Desktop (local, stdio)

Put the JSON block from [Setup](#setup) into Claude Desktop's config file
and restart the app:

- macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`
- Windows: `%APPDATA%\Claude\claude_desktop_config.json`

Proton Mail Bridge must be running on the same machine. The tools appear
under the tools menu in a new conversation.

### Claude Code (local, stdio)

```sh
claude mcp add proton-mail --scope user \
  -e PROTON_USERNAME=your-bridge-username \
  -e PROTON_PASSWORD=your-bridge-password \
  -- /path/to/proton-mail-mcp/target/release/proton-mail-mcp
```

`--scope user` makes the server available in every project; drop it to
register it for the current project only. Run `/mcp` inside Claude Code to
confirm it connected. A checked-in `.mcp.json` works too, but keep the
credentials out of version control (this repo gitignores it).

### claude.ai and Claude Desktop (remote, HTTP + OAuth)

Requires the server to be reachable over HTTPS with the OAuth trio set, for
example via the [`deploy/aws`](deploy/aws/README.md) stack.

1. Open **Settings → Connectors → Add custom connector** (on claude.ai or in
   Claude Desktop; connectors are tied to your Claude account, so a connector
   added in one place is available in the web app, desktop app, and mobile
   apps).
2. Name it **Proton Mail** and enter `https://your-domain.example/mcp`.
   Leave the OAuth client ID and client secret blank; Claude registers itself
   via dynamic client registration.
3. Click **Connect**. The server's authorization page opens; enter
   `MCP_OAUTH_PASSPHRASE` (not your Proton or Bridge password). Tokens
   refresh automatically from then on.
4. In a conversation, enable the connector from the tools menu and try:
   "List my Proton Mail folders."

The server allows the claude.ai and claude.com OAuth callbacks out of the
box, so no `MCP_OAUTH_REDIRECT_ALLOWLIST` entry is needed. On Team and
Enterprise plans an admin can instead add the connector once under
**Admin settings → Connectors** for the whole organization.

### Claude Code (remote, HTTP + OAuth)

```sh
claude mcp add --transport http --scope user proton-mail https://your-domain.example/mcp
```

Then start Claude Code, run `/mcp`, select **proton-mail**, and choose
**Authenticate**. A browser opens on the authorization page; enter the
passphrase. Claude Code receives the token on a loopback callback
(`http://localhost:<port>/callback`), which the server allows by default.
Tokens are stored in your OS keychain and refreshed automatically; rotating
`MCP_OAUTH_SIGNING_KEY` on the server forces a re-authentication.

## ChatGPT setup

ChatGPT connects to the remote `/mcp` endpoint using streamable HTTP and
OAuth. Use the HTTP + OAuth configuration above behind a public HTTPS
reverse proxy, with Proton Mail Bridge running where the server can reach
it. The stdio JSON configuration is for local MCP clients.

The server uses MCP SDK 3.x to support ChatGPT's `2026-07-28`
`server/discover` flow, while retaining legacy `initialize` sessions.
HTTP host validation allows loopback and the authority from `MCP_PUBLIC_URL`.

Deploy this version of the server before creating the ChatGPT connection.
With the [`deploy/aws`](deploy/aws/README.md) stack, run `npx cdk diff`,
review the changes, then `npx cdk deploy`. An app-only deployment updates
the image without replacing the instance and is applied within 30 minutes
(or immediately via the runbook). Check the diff carefully: infrastructure
changes can replace the instance and require a new Bridge login.

1. In ChatGPT, open **Settings → Security and login → Developer mode**.
   Availability depends on your account and workspace policy.
2. Open [ChatGPT Plugins](https://chatgpt.com/plugins), select **+**, and name
   the connection **Proton Mail**.
3. Enter your public server URL including `/mcp`. Choose **OAuth** and
   **dynamic client registration (DCR)** if prompted. Leave client ID and
   client secret blank; the server supports public clients using PKCE.
4. On the server's authorization page, enter `MCP_OAUTH_PASSPHRASE` (the
   separate connector passphrase, not your Proton account or Bridge password).
5. Review the discovered tools, add the connection to a new conversation,
   and try: “List my Proton Mail folders.” Then try: “Find unread messages
   in my inbox.” Drafting is supported; sending mail is not.

The server allows the exact ChatGPT callback
`https://chatgpt.com/connector_platform_oauth_redirect`. It advertises
`authorization_response_iss_parameter_supported: true` and returns `iss`
in successful and error OAuth redirects so ChatGPT can use that stable
callback. If the connection's management page instead shows
`https://chatgpt.com/connector/oauth/{callback_id}`, add that **exact** URL
to `MCP_OAUTH_REDIRECT_ALLOWLIST` and restart the server. Do not allow a
wildcard domain or callback prefix.

Verify the deployed configuration (replace the example origin):

```sh
curl -fsS https://your-domain.example/healthz
curl -fsS https://your-domain.example/.well-known/oauth-authorization-server
curl -fsS https://your-domain.example/.well-known/oauth-protected-resource/mcp
curl -i https://your-domain.example/mcp
```

Expect `ok`, authorization metadata containing the issuer-identification
flag above, resource metadata identifying the `/mcp` URL, and an HTTP 401
with a `WWW-Authenticate` header for the unauthenticated MCP request.
If ChatGPT reports `invalid_redirect_uri`, compare its exact callback with
the allowlist. After a server update, refresh the connection's metadata in
ChatGPT and start a new conversation.

See the official [connection guide](https://developers.openai.com/plugins/deploy/connect-chatgpt)
and [authentication requirements](https://developers.openai.com/plugins/build/auth).

## Tools

All tools advertise an `outputSchema` generated from their response types and
return matching JSON in `structuredContent`, plus a serialized JSON text fallback.
`list_mailboxes` returns an array; all other tools return objects. Tool failures
return text with `isError: true` instead of a structured success result.

| Tool | Description |
|------|-------------|
| `list_mailboxes` | List all mailboxes |
| `resolve_special_mailbox` | Resolve a well-known folder kind to its IMAP mailbox name |
| `list_messages` | List messages in a mailbox with pagination |
| `get_message` | Fetch a full message (headers, body, attachments) |
| `search_messages` | Search with filters and cursor-based pagination |
| `get_mailbox_status` | Mailbox stats (count, recent, unseen) |
| `get_attachment` | Download an attachment (inline base64, a download URL / file path for large ones, or extracted text) |
| `list_messages_by_thread` | Find related messages by Message-ID or subject |
| `archive_message` | Move a message to Archive |
| `trash_message` | Move a message to Trash |
| `archive_messages` | Move several messages to Archive in one call |
| `trash_messages` | Move several messages to Trash in one call |
| `move_message` | Move a message to a different mailbox |
| `move_messages` | Move several messages to a different mailbox in one call |
| `mark_as_read` | Mark a message as read or unread |
| `set_flagged` | Star or unstar a message |
| `draft_email` | Save a draft to the Drafts mailbox, optionally as a threaded reply |

## Testing

Unit tests run without any external dependencies:

```sh
cargo test --lib
cargo test --test oauth_flow
cargo test --test http_transport
```

Integration tests require a running [GreenMail](https://greenmail-mail-test.github.io/greenmail/) IMAP server:

```sh
docker compose up -d
cargo test --test imap_integration
docker compose down
```

By default, tests connect to `127.0.0.1:3993` (IMAPS). Override with `TEST_HOST` and `TEST_IMAPS_PORT` if needed.
