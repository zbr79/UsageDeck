# Security

UsageDeck is a single-user, local-only Windows desktop tool. This page lists exactly what it reads,
stores and sends.

## No local server

UsageDeck is one process (`usagedeck.exe`, Tauri 2). The widget and settings pages are bundled
into the exe and talk to the Rust core through Tauri `invoke`, in process.

- Nothing listens on a port: there is no HTTP server, no loopback API and no shared secret to
  protect one. Other programs and web pages have nothing to connect to.
- The two windows (`widget`, `settings`) get only the permissions in
  `src-tauri/capabilities/default.json`: core window and event calls, and log. No file system,
  shell, HTTP, clipboard or autostart plugin is exposed to the frontend; all file, network,
  clipboard and Run-key access happens in Rust.
- The settings window is not created at startup. It is built when you open Settings and destroyed
  when you close it; the capability matches it by its label (`settings`).
- Single instance: a second launch shows the running widget instead (or opens Settings when it
  was started with `--settings`).

## Content-Security-Policy

`src-tauri/tauri.conf.json` sets this CSP on both pages:

```
default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:;
connect-src ipc: http://ipc.localhost; object-src 'none'; base-uri 'none'; frame-ancestors 'none'
```

Scripts load only from the bundled app; no inline or remote script, no `eval`. Inline styles are
allowed because the UI sets element styles (sizes, positions, colours) at run time. `connect-src`
allows only Tauri's IPC channel, so page script cannot reach the network.

## OpenCode key copy

**Copy key** on the widget calls the Rust command `copy_key(email)`. Rust reads the key from
Credential Manager and writes it to the Windows clipboard itself; the full key never reaches the
page. The old `GET /api/key` route that returned the key to the page is gone (it now answers 404).

## Other apps' credentials: read-only

UsageDeck **reads** sign-ins that other apps already keep on this machine. It **never writes,
refreshes or deletes them** — refreshing would rotate their refresh tokens and log those apps out.
When a token has expired or is rejected, the card shows `re-auth` and the last known numbers
instead. No token-refresh endpoint is ever called.

**Cursor.** UsageDeck reads only `cursorAuth/accessToken` and the account name
(`cursorAuth/cachedEmail`, else `cursorAuth/cachedScopedProfile`) from `state.vscdb`. It never reads
`cursorAuth/refreshToken` and never calls `https://api2.cursor.sh/oauth/token`: Cursor stores each
renewed access token as its refresh token too, and whether a refresh retires the token it was given
is not known, so a refresh from UsageDeck could log Cursor out. When Cursor's API rejects the access
token (401 or 403), the card shows its last numbers in gray with an **Open Cursor** button. The button
starts `Cursor.exe`, which renews its own token. UsageDeck looks for it in
`%LOCALAPPDATA%\Programs\cursor`, then `%ProgramFiles%\cursor`, then in the `InstallLocation` or
`DisplayIcon` of a "Cursor" entry under the `HKCU` / `HKLM` `…\CurrentVersion\Uninstall` registry
keys (read-only). After any failed fetch, every poll reads `state.vscdb` again (read-only),
so the renewed token is used as soon as Cursor writes it. The token is kept in memory only.

Cursor 3.27 and later store the access token sealed with Electron safeStorage, as
`cursor-safe-storage:v1:` followed by base64. UsageDeck opens it with the key in
`%APPDATA%\Cursor\Local State`, exactly as described below for Claude desktop and Grok Bot. A
token it cannot open is reported as `can't read sign-in`; it is never sent anywhere.

When Cursor has `state.vscdb` busy, UsageDeck copies it, with its `-wal`, `-shm` and `-journal`
files, to a new folder under `%TEMP%`, reads the copy and deletes the folder at once. The copy holds
Cursor's sign-in for those few milliseconds; Cursor's own files are never opened for writing.

**cursor.com fallback.** When `api2.cursor.sh` answers 401 or 403 for a token whose expiry is still
ahead, answers with another error, or answers without the usage, UsageDeck sends one
`GET https://cursor.com/api/usage-summary` with the headers `Accept: application/json` and
`Cookie: WorkosCursorSessionToken=<user id>%3A%3A<access token>`. The user id is the part of the
token's `sub` claim after the last `|`. This is the session cookie Cursor's own web dashboard uses,
built from the same access token; nothing new is stored and no cookie is kept between requests.

| App            | What is read (read-only)                                                                                    |
| -------------- | ----------------------------------------------------------------------------------------------------------- |
| Claude Code    | `~/.claude/.credentials.json` (`claudeAiOauth`), `~/.claude.json` (account email / plan)                    |
| Claude Code transcripts | `~/.claude/projects/**/*.jsonl`, shared-read; only reply token counts, model and time are parsed; only per-minute token totals are stored (`spend-buckets.json`, `claude-logs-index.json`, `usage-history.jsonl` holds hashed account ids and percentages) |
| Claude desktop | `%APPDATA%\Claude\config.json` token cache and `Local State` key (or the Microsoft Store package equivalent) |
| Codex CLI      | `~/.codex/auth.json`                                                                                        |
| Cursor         | `%APPDATA%\Cursor\User\globalStorage\state.vscdb` (SQLite, opened with `SQLITE_OPEN_READ_ONLY`; a temporary copy when busy) and the `Local State` key |
| Grok Bot       | `%APPDATA%\Grok Bot\sand-secrets.json` and `Local State`; falls back to Cursor's `state.vscdb`              |

Claude desktop, Grok Bot and Cursor (3.27 and later) keep their tokens in Chromium "safeStorage": the AES key in
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

**Logs.** `%LOCALAPPDATA%\com.usagedeck.app\logs\` (tray menu → _Open log folder_). About 1 MB
per file, at most 5 files, oldest deleted first. They hold start-up steps, provider request
results and errors (HTTP status, error kind), with account ids hashed. They never hold API keys or
tokens.

**Diagnostics.** Settings → Cursor → Diagnostics and **Copy diagnostics** show what the last Cursor
attempt and the last update check did. The record is kept in memory only and is gone when
UsageDeck exits. It contains: the time; whether `state.vscdb` was found and how it was read; how
the token is stored and whether it decrypted; the first 8 hex characters of the token's SHA-256 and
its expiry time; for each request the host, path, HTTP status and either the server's Connect
error or the first 200 characters of its reply; which source gave the usage; the UsageDeck and
Windows versions; and the updater's last check. It never contains a token, a cookie, an API key or
an account name: any run of 24 or more token characters, and anything shaped like a JWT, is
replaced with `[redacted]` before it is recorded and again before the report is shown or copied.
The file path is shown as `state.vscdb`, without your user folder. Copying puts the report on the
clipboard from the app's own process; nothing is sent anywhere.

**Crash files.** If the app panics it writes `crash-<unix-seconds>.txt` to the same log folder:
the panic message, its source location, a backtrace (function names and addresses) and the app
version. The newest 10 are kept. Nothing else is collected and nothing is sent anywhere. The panic
message is code-written text; the code never formats account emails, keys or tokens into one.

**Screen sampler.** With the widget theme on `auto`, every 3 s the app reads the screen pixels in
a 48 px band around the widget (GDI), averages their brightness in memory and keeps only that one
number to pick the card style. No pixels are saved, logged or sent anywhere. With `light` or
`dark` it does not sample at all.

**Start at login.** The `startAtLogin` setting (Settings → General → _Start with Windows_, on by
default) adds or removes `UsageDeck` in
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`. Only the Rust side changes it, at start-up
and when you flip the setting; copies run from a cargo `target` folder never touch it.

**Uninstall.** Every uninstall (not an in-place update) removes the program files, the shortcuts,
the `UsageDeck` value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` and the
matching Task Manager entry under `...\Explorer\StartupApproved\Run`. With the uninstaller's
**Delete the application data** box ticked it also removes `%APPDATA%\com.usagedeck.app` and
`%LOCALAPPDATA%\com.usagedeck.app` (settings, usage cache, logs, crash files) and every
`<account>.UsageDeck` generic credential in Credential Manager (the OpenCode keys). Unticked, the
data and the keys stay, so a reinstall or a manual upgrade picks them up again. A `Run` value under
`HKLM` exists only if the app was once run elevated; the per-user uninstaller cannot remove it.

## Network

**Inbound.** None. UsageDeck opens no listening socket.

**Outbound.** HTTPS (rustls) to each provider's own usage endpoint, and nothing else:

| Provider    | Endpoint                                                                      | Credential sent             |
| ----------- | ----------------------------------------------------------------------------- | --------------------------- |
| OpenCode Go | `https://opencode.ai/zen/go/v1/usage`                                         | your OpenCode API key       |
| Cursor      | `https://api2.cursor.sh/aiserver.v1.DashboardService/` `GetCurrentPeriodUsage`, `GetPlanInfo`, `GetHardLimit` | Cursor's access token |
| Cursor (fallback) | `GET https://cursor.com/api/usage-summary`                              | the same token, as the `WorkosCursorSessionToken` cookie |
| Grok Bot    | `https://api2.cursor.sh/aiserver.v1.DashboardService/GetSandUsageStatus`      | Grok Bot's (or Cursor's) token |
| Codex       | `https://chatgpt.com/backend-api/wham/usage`                                  | Codex CLI's access token    |
| Claude      | `https://api.anthropic.com/api/oauth/usage`                                   | Claude's OAuth access token |

Each token goes only to its own provider. No telemetry, no analytics, no token-refresh endpoints.

Every DashboardService call (Cursor and Grok Bot) is a `POST` with the body `{}` and these headers:
`Authorization: Bearer <token>`, `Content-Type: application/json`, `Connect-Protocol-Version: 1`,
and the `Accept: */*`, `Host` and `Content-Length` that the HTTP client adds. No `User-Agent` and no
cookie are sent. The cursor.com fallback above is the only request that carries a cookie. A failed
Cursor attempt is logged with the same one-line reason the card shows: the host, the HTTP status,
the error code from the reply and the expiry time from the token's `exp` claim; the token itself
is never logged.

The updater (src-tauri/src/updater.rs) sends no token and no personal data. About once an hour it
reads `https://api.github.com/repos/zbr79/UsageDeck/releases/latest`; when a newer version exists
it downloads that release's `UsageDeck-Setup.exe` from GitHub and checks its size and SHA-256
against the digest GitHub reports before offering to install it.

## Known limitations

- **Undocumented endpoints.** Provider usage endpoints are not public APIs and may change or break
  without notice; some requests identify as the provider's own client.
- **Same-user trust.** Anything running as your Windows user can read the same sign-ins and
  Credential Manager entries UsageDeck can; UsageDeck does not try to defend against that.
- **Inline styles allowed.** The CSP keeps `style-src 'unsafe-inline'` because the UI sets styles
  at run time; scripts stay strictly `'self'`.

## Reporting a problem

Please open a private security advisory on the GitHub repository rather than a public issue.
