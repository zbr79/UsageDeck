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

**Logs.** `%LOCALAPPDATA%\com.usagedeck.app\logs\` (tray menu → _Open log folder_). About 1 MB
per file, at most 5 files, oldest deleted first. They hold start-up steps, provider request
results and errors (HTTP status, error kind), with account ids hashed. They never hold API keys or
tokens.

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
- **Inline styles allowed.** The CSP keeps `style-src 'unsafe-inline'` because the UI sets styles
  at run time; scripts stay strictly `'self'`.

## Reporting a problem

Please open a private security advisory on the GitHub repository rather than a public issue.
