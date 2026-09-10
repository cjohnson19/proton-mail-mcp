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

Add `https://your-domain.example/mcp` as a claude.ai custom connector with
the OAuth fields blank; Claude registers itself and opens the passphrase
page. `MCP_OAUTH_REDIRECT_ALLOWLIST` (comma-separated) admits additional
OAuth callbacks beyond Claude, ChatGPT's stable callback, and loopback.

## ChatGPT setup

ChatGPT connects to the remote `/mcp` endpoint using streamable HTTP and
OAuth. Use the HTTP + OAuth configuration above behind a public HTTPS
reverse proxy, with Proton Mail Bridge running where the server can reach
it. The stdio JSON configuration is for local MCP clients.

The server uses MCP SDK 3.x to support ChatGPT's `2026-07-28`
`server/discover` flow, while retaining legacy `initialize` sessions.
HTTP host validation allows loopback and the authority from `MCP_PUBLIC_URL`.

Deploy this version of the server before creating the ChatGPT connection.
For the companion AWS deployment, run `npx cdk diff` from
`../proton-mail-mcp-infra`, review the changes, then run `npx cdk deploy`.
An app-only deployment updates the image without replacing the instance;
the existing refresh job applies it within 30 minutes. Follow the infra
runbook to apply it immediately. Check the diff carefully: infrastructure
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
| `get_attachment` | Download an attachment (base64-encoded) |
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
