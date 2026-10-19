# Security

UsageDeck is a single-user, local-only Windows desktop tool. This page lists exactly what it reads,
stores and sends.

## No local server

UsageDeck is one process (`usagedeck.exe`, Tauri 2). The widget and settings pages are bundled
into the exe and talk to the Rust core through Tauri `invoke`, in process.

- Nothing listens on a port: there is no HTTP server, no loopback API and no shared secret to
  protect one. Other programs and web pages have nothing to connect to.
- The two windows (`widget`, `settings`) get only the permissions in
  `src-tauri/capabilities/default.json`: core window and event calls, autostart and log. No file
  system, shell or HTTP plugin is exposed to the frontend; all file and network access happens in
  Rust.
- Single instance: a second launch shows the running widget instead.

## Other apps' credentials: read-only

UsageDeck **reads** sign-ins that other apps already keep on this machine. It **never writes,
refreshes or deletes them** — refreshing would rotate their refresh tokens and log those apps out.
When a token has expired or is rejected, the card shows `re-auth` and the last known numbers
instead. No token-refresh endpoint is ever called.

| App            | What is read (read-only)                                                                                    |
| -------------- | ----------------------------------------------------------------------------------------------------------- |
| Claude Code    | `~/.claude/.credentials.json` (`claudeAiOauth`), `~/.claude.json` (account email / plan)                    |
| Claude desktop | `%APPDATA%\Claude\config.json` token cache and `Local State` key (or the Microsoft Store package equivalent) |
| Codex CLI      | `~/.codex/auth.json`                                                                                        |
| Cursor         | `%APPDATA%\Cursor\User\globalStorage\state.vscdb` (SQLite, opened with `SQLITE_OPEN_READ_ONLY`)             |
| Grok Bot       | `%APPDATA%\Grok Bot\sand-secrets.json` and `Local State`; falls back to Cursor's `state.vscdb`              |

Claude desktop and Grok Bot keep their tokens in Chromium "safeStorage": the AES key in
`Local State` is protected with Windows DPAPI for the current user. UsageDeck unprotects that key
**in process** with `CryptUnprotectData` (current-user scope; no PowerShell or child process),
keeps it in memory only, and decrypts the token with AES-GCM. Nothing decrypted is written to disk.

## What the app stores

All in `%APPDATA%\com.usagedeck.app`, protected by your Windows account's file permissions:

| File                 | Contents                                    | Notes                                              |
| -------------------- | ------------------------------------------- | -------------------------------------------------- |
| `settings.json`      | Toggles, themes, nicknames, hidden accounts | Validated in Rust; unknown fields ignored          |
| `accounts.json`      | OpenCode account labels (emails)            | **No keys**                                        |
| `seen-accounts.json` | Emails of provider accounts already seen    | No tokens. Lets a switched-away account stay listed |
| `usage-cache.json`   | Last known usage, account emails / plans    | Shown as `cached` when providers are unreachable   |
| `window-pos.json`    | Widget position                             |                                                    |

**OpenCode API keys** are stored in **Windows Credential Manager** (generic credentials, service
`UsageDeck`), never in a file. The settings window shows only a hint (`sk-…a1b2`); deleting an
account deletes its credential. `--import-old-data` moves keys from the old app's `accounts.json`
(plaintext or `enc:v1:`) into Credential Manager and never prints them; `--dump` prints a redacted
summary with labels cut to two characters.

Writes are atomic (temp file + rename) and serialised per file.

## Network

**Inbound.** None. UsageDeck opens no listening socket.

**Outbound.** HTTPS (rustls) to each provider's own usage endpoint, and nothing else:

| Provider    | Endpoint                                                                      | Credential sent             |
| ----------- | ----------------------------------------------------------------------------- | --------------------------- |
| OpenCode Go | `https://opencode.ai/zen/go/v1/usage`                                         | your OpenCode API key       |
| Cursor      | `https://api2.cursor.sh/aiserver.v1.DashboardService/…` (usage and plan)      | Cursor's access token       |
| Grok Bot    | `https://api2.cursor.sh/aiserver.v1.DashboardService/GetSandUsageStatus`      | Grok Bot's (or Cursor's) token |
| Codex       | `https://chatgpt.com/backend-api/wham/usage`                                  | Codex CLI's access token    |
| Claude      | `https://api.anthropic.com/api/oauth/usage`                                   | Claude's OAuth access token |

Each token goes only to its own provider. No telemetry, no analytics, no update check, no
token-refresh endpoints.

## Known limitations

- **Undocumented endpoints.** Provider usage endpoints are not public APIs and may change or break
  without notice; some requests identify as the provider's own client.
- **Same-user trust.** Anything running as your Windows user can read the same sign-ins and
  Credential Manager entries UsageDeck can; UsageDeck does not try to defend against that.
- **No Content-Security-Policy yet.** `src-tauri/tauri.conf.json` sets `"csp": null`. The webview
  loads only the bundled pages and no remote content, but a CSP would be a cheap extra layer.

## Reporting a problem

Please open a private security advisory on the GitHub repository rather than a public issue.
