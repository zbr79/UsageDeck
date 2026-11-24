# Security

UsageDeck is a single-user, local-only Windows desktop tool written in pure Rust. This page lists
exactly what it reads, stores, and sends.

## Pure-Rust Win32 architecture — no webview, no local server

UsageDeck runs as a single native Windows process (`usagedeck.exe`) rendered directly with Win32,
Direct2D, and DirectWrite. There is no embedded browser, no WebView2, no JavaScript runtime, and no
Tauri layer.

- **No listening socket.** Nothing listens on a port: there is no HTTP server, no loopback API, and
  no shared secret to protect one. Other programs and web pages have nothing to connect to.
- **No script execution surface.** All UI rendering, storage, clipboard writes, Registry reads, and
  network calls happen directly in compiled Rust.
- **Single instance.** A second launch activates the running taskbar widget instead (or opens the
  native Direct2D Settings window when started with `--settings`).

## Hardened outbound HTTP client

Every provider call uses a shared `reqwest` client backed by `rustls`:

- **HTTPS only** (`.https_only(true)`): plaintext `http://` requests are rejected at the client
  layer.
- **Zero redirects** (`.redirect(Policy::none())`): bearer tokens and cookies can never be
  forwarded to a redirected host.
- **TLS 1.2 minimum** (`.min_tls_version(Version::TLS_1_2)`).
- **20-second request cap** (with 10-second per-provider timeouts) and URL-redacted error logging
  (`e.without_url()`).

## Memory and debug hygiene

- **Secret-redacting `Debug` implementations.** `Account`, `ClaudeToken`, Cursor `SignIn`,
  `GrokAccount`, and `AntigravityCredential` implement `fmt::Debug` manually and print `<redacted>`
  in place of tokens or API keys.
- **DPAPI hardening and memory zeroization.** `CryptUnprotectData` is called in process with
  `CRYPTPROTECT_UI_FORBIDDEN` so it never blocks on a Windows UI prompt. `Local State` keys require
  an explicit `"DPAPI"` prefix; Chromium App-Bound Encryption (`app_bound_encrypted_key` / `"APPB"`
  prefix) fails closed with a clear diagnostic message. Decrypted DPAPI `LocalAlloc` buffers are
  scrubbed in place with volatile writes and a `SeqCst` compiler fence (`volatile_zeroize`) before
  `LocalFree`.
- **Hashed in-memory profile keys.** Claude's in-memory `oauth/profile` cache is keyed by the
  SHA-256 `log_id` of the access token rather than the raw plaintext token.
- **Poison-safe locking.** Internal mutexes recover gracefully on poison rather than panicking.

## OpenCode key copy

Clicking an OpenCode account's icon (or **Copy key**) reads the key from Windows Credential Manager
in Rust and writes it directly to the Windows clipboard; no key is logged or written to disk.

## Other apps' credentials: read-only

UsageDeck **reads** sign-ins that other apps already keep on this machine. It **never writes,
refreshes, or deletes them** — refreshing third-party OAuth tokens would rotate their refresh tokens
and log those apps out. When a token has expired or is rejected, the card shows `re-auth` and the
last known numbers instead. No third-party token-refresh endpoint is ever called.

**Cursor (API Key path — recommended).** When `CURSOR_API_KEY` is set in the environment or saved
in Windows Credential Manager (`UsageDeck` / `cursor:api-key`), UsageDeck exchanges it via
`POST https://api2.cursor.sh/auth/exchange_user_api_key` for a short-lived runtime access token and
queries `DashboardService` directly with `x-cursor-client-type: cli`. On this path, UsageDeck does
**not** touch DPAPI or `state.vscdb` at all.

**Cursor (`state.vscdb` fallback).** When no Cursor API key is configured, UsageDeck reads only
`cursorAuth/accessToken` and the account name (`cursorAuth/cachedEmail`, else
`cursorAuth/cachedScopedProfile`) from `state.vscdb` (opened `SQLITE_OPEN_READ_ONLY`). It never
reads `cursorAuth/refreshToken` and never calls `https://api2.cursor.sh/oauth/token`. When Cursor
holds a lock on `state.vscdb`, UsageDeck copies the database (with `-wal`, `-shm`, `-journal`) into
a private per-user folder under `%LOCALAPPDATA%\com.usagedeck.app\tmp\cursor-state-<pid>-<nanos>\`,
reads it, and deletes the folder immediately; any orphaned `cursor-state-*` folders from a prior
crash are swept on startup.

The **Open Cursor** button starts `Cursor.exe` from `%LOCALAPPDATA%\Programs\cursor`,
`%ProgramFiles%\cursor`, or `HKCU`/`HKLM` Uninstall entries whose `Publisher` matches `Anysphere`;
UNC paths (`\\...`, `//...`) and `..` traversals are rejected.

Cursor 3.27 and later seal the access token with Electron safeStorage (`cursor-safe-storage:v1:`);
UsageDeck unseals it in memory using `%APPDATA%\Cursor\Local State`. When `api2.cursor.sh` rejects an
unexpired token or answers without usage, UsageDeck falls back to
`GET https://cursor.com/api/usage-summary` (and, when API spend > 0, up to 20 pages of
`POST https://cursor.com/api/dashboard/get-filtered-usage-events` at most every 5 minutes) with the
session cookie built in memory from the same access token.

**Grok (xAI Grok CLI and Grok Bot).** UsageDeck checks the official xAI Grok CLI session at
`~/.grok/auth.json` (or `GROK_HOME/auth.json` / `GROK_AUTH`, skipped when `XAI_API_KEY` is set) and
reads `%APPDATA%\Grok Bot\sand-secrets.json` + `Local State`. If `sand-secrets.json` exists on disk
but fails to parse or decrypt, UsageDeck **fails closed** and never silently falls back to Cursor's
`state.vscdb`.

**OpenCode (Credential Manager + read-only auto-discovery).** In addition to accounts configured in
Windows Credential Manager, UsageDeck performs read-only discovery from `OPENCODE_API_KEY`,
`%LOCALAPPDATA%\opencode\auth.json` / `~/.local/share/opencode/auth.json`, and read-only SQLite
`opencode.db` (`SQLITE_OPEN_READ_ONLY`). Discovered keys are deduplicated against Credential Manager
accounts in memory; `accounts.json` is never mutated.

**Google Antigravity.** When Antigravity Desktop IDE or `agy` is running locally, UsageDeck queries
its loopback Connect-RPC server on `127.0.0.1` (`RetrieveUserQuotaSummary` / `GetUserStatus` /
`GetCommandModelConfigs`) read-only using the process's `--csrf_token`. Otherwise, UsageDeck reads
Antigravity's OAuth access token read-only from `%APPDATA%\*Antigravity*\User\globalStorage\state.vscdb`
(`SQLITE_OPEN_READ_ONLY`, private temporary copy when locked), Windows Credential Manager
(`gemini:antigravity` and `*antigravity*`, reading raw UTF-8/UTF-16LE credential blobs), or
`%USERPROFILE%\.gemini\` (`jetski-standalone-oauth-token`, `antigravity-oauth-token`, `acp_token.json`)
and queries `v1internal:retrieveUserQuotaSummary` (falling back to `v1internal:fetchAvailableModels`)
read-only. UsageDeck never refreshes or modifies Antigravity's OAuth tokens.

| App                     | What is read (read-only)                                                                                                                                                                    |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Claude Code             | `~/.claude/.credentials.json` (`claudeAiOauth`), `~/.claude.json` (account email / plan)                                                                                                    |
| Claude Code transcripts | `~/.claude/projects/**/*.jsonl`, shared-read; only reply token counts, model, and timestamp are parsed; only per-minute token totals are stored                                             |
| Claude desktop          | `%LOCALAPPDATA%\Claude-Data\*`, Microsoft Store package roaming dir, or `%APPDATA%\Claude` (`config.json` token cache and `Local State` DPAPI key)                                          |
| Codex CLI               | `~/.codex/auth.json`                                                                                                                                                                        |
| Cursor                  | `CURSOR_API_KEY` env / Credential Manager `cursor:api-key`; fallback `%APPDATA%\Cursor\User\globalStorage\state.vscdb` (`SQLITE_OPEN_READ_ONLY`; private `%LOCALAPPDATA%` copy when busy)   |
| Grok CLI                | `~/.grok/auth.json` (or `GROK_HOME/auth.json` / `GROK_AUTH`, skipped if `XAI_API_KEY` is set)                                                                                               |
| Grok Bot                | `%APPDATA%\Grok Bot\sand-secrets.json` and `Local State` (fails closed on decryption error; falls back to Cursor only when absent)                                                          |
| OpenCode                | Windows Credential Manager (`UsageDeck`), `OPENCODE_API_KEY`, `auth.json`, and `opencode.db` (`SQLITE_OPEN_READ_ONLY`)                                                                      |
| Antigravity             | Local `language_server` / `agy` loopback RPC (`127.0.0.1`), `*Antigravity*\User\globalStorage\state.vscdb` (`SQLITE_OPEN_READ_ONLY`), Credential Manager (`*antigravity*`), and `~/.gemini/` |

## What the app stores

All in `%APPDATA%\com.usagedeck.app`, protected by your Windows account's file permissions:

| File                 | Contents                                    | Notes                                              |
| -------------------- | ------------------------------------------- | -------------------------------------------------- |
| `settings.json`      | Toggles, themes, nicknames, hidden accounts | Validated in Rust; unknown fields ignored          |
| `accounts.json`      | OpenCode account labels (emails)            | **No keys**                                        |
| `seen-accounts.json` | Emails of provider accounts already seen    | No tokens. Lets a switched-away account stay listed |
| `usage-cache.json`   | Last known usage, account emails / plans    | Shown as `cached` when providers are unreachable   |
| `window-pos.json`    | Widget position                             |                                                    |

**OpenCode and Cursor API keys** are stored in **Windows Credential Manager** (generic credentials,
service `UsageDeck`), never in a file. Settings shows only a masked hint (`oc_sk_…a1b2` / `…a1b2`);
deleting an account deletes its credential, and account updates write to Credential Manager only
when a key was added or changed.

Writes are atomic (temp file + `sync_all` + rename) and serialised per file.

**Logs.** `%LOCALAPPDATA%\com.usagedeck.app\logs\` (tray menu -> _Open log folder_). About 1 MB per
file, at most 5 files, oldest deleted first. They hold start-up steps, provider request results, and
errors (HTTP status, error kind) with account ids hashed and URLs/paths redacted. They never hold
API keys or tokens.

**Diagnostics.** Settings -> Cursor -> Diagnostics and **Copy diagnostics** keep the last Cursor
attempt and update check in memory only. Any run of 24+ token characters or JWT-shaped text is
replaced with `[redacted]` before recording and again before display/copy.

**Crash files.** On panic, UsageDeck writes `crash-<unix-seconds>.txt` to the log folder (panic
message, source location, backtrace, app version; newest 10 kept). Nothing is sent anywhere.

**Screen sampler.** With the widget theme on `auto`, every 3 s the app averages pixel brightness in
a 48 px band around the widget via GDI and keeps only that scalar in memory. No pixels are saved,
logged, or sent anywhere.

**Start at login & Uninstall.** `startAtLogin` toggles `UsageDeck` under
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`. Uninstalling removes program files, shortcuts,
and startup entries; ticking **Delete the application data** also removes
`%APPDATA%\com.usagedeck.app`, `%LOCALAPPDATA%\com.usagedeck.app`, and `UsageDeck` entries in
Windows Credential Manager.

## Network

**Inbound.** None. UsageDeck opens no listening socket.

**Outbound.** HTTPS (TLS 1.2+, `rustls`, zero redirects) to each provider's own usage endpoint, and
nothing else:

| Provider                  | Endpoint                                                                                                           | Credential sent                                         |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------- |
| OpenCode Go               | `GET https://opencode.ai/zen/go/v1/usage`                                                                          | OpenCode API key (`Authorization: Bearer`)              |
| Cursor (API key exchange) | `POST https://api2.cursor.sh/auth/exchange_user_api_key`                                                           | Configured `CURSOR_API_KEY` (`Authorization: Bearer`)   |
| Cursor (DashboardService) | `POST https://api2.cursor.sh/aiserver.v1.DashboardService/` `GetCurrentPeriodUsage`, `GetPlanInfo`, `GetHardLimit` | Cursor access token (`Authorization: Bearer`)           |
| Cursor (web fallback)     | `GET https://cursor.com/api/usage-summary`                                                                         | Same access token as `WorkosCursorSessionToken` cookie  |
| Cursor (API dollars)      | `POST https://cursor.com/api/dashboard/get-filtered-usage-events`                                                  | Same access token as `WorkosCursorSessionToken` cookie  |
| Grok CLI (xAI)            | `GET https://cli-chat-proxy.grok.com/v1/billing?format=credits`                                                    | xAI Grok CLI session token (`Authorization: Bearer`)    |
| Grok Bot                  | `POST https://api2.cursor.sh/aiserver.v1.DashboardService/GetSandUsageStatus`                                      | Grok Bot (or Cursor fallback) token                     |
| Codex                     | `GET https://chatgpt.com/backend-api/wham/usage`, `.../wham/rate-limit-reset-credits`                              | Codex CLI access token (`Authorization: Bearer`)        |
| Claude                    | `GET https://api.anthropic.com/api/oauth/usage`, `GET https://api.anthropic.com/api/oauth/profile`                 | Claude OAuth access token (`Authorization: Bearer`)     |
| Antigravity               | `POST https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary` (fallback `cloudcode-pa.googleapis.com`) | Antigravity Google OAuth token (`Authorization: Bearer`) |
| Updater                   | `GET https://api.github.com/repos/zbr79/UsageDeck/releases/latest` (+ SHA-256 verified release asset)              | None                                                    |

Each token goes only to its own provider. No telemetry, no analytics, no third-party token-refresh
endpoints.

## Known limitations

- **Undocumented endpoints.** Provider usage endpoints are not public APIs and may change or break
  without notice; some requests identify as the provider's own client.
- **Same-user trust.** Anything running as your Windows user can read the same sign-ins and
  Credential Manager entries UsageDeck can; UsageDeck does not try to defend against that.

## Reporting a problem

Please open a private security advisory on the GitHub repository rather than a public issue.
