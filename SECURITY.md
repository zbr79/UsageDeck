# Security

Provider Usage Monitor is a single-user, local-only desktop tool. This page lists exactly what it
reads, stores and sends, and how the local server is protected.

## Other apps' credentials: read-only

The monitor **reads** sign-ins that other apps already keep on this machine. It **never writes,
refreshes or deletes them** — refreshing would rotate their refresh tokens and log those apps out.
When a token has expired, the card shows `re-auth` and the last known numbers instead.

| App            | What is read (read-only)                                                                                                |
| -------------- | ----------------------------------------------------------------------------------------------------------------------- |
| Claude Code    | `~/.claude/.credentials.json` (`claudeAiOauth`), `~/.claude.json` (account email/plan)                                   |
| Claude desktop | `%APPDATA%\Claude\config.json` token cache and `Local State` key (or the Microsoft Store package equivalent)             |
| Codex CLI      | `~/.codex/auth.json`                                                                                                    |
| Cursor         | `%APPDATA%\Cursor\User\globalStorage\state.vscdb` (opened read-only via `node:sqlite`)                                  |
| Grok Bot       | `%APPDATA%\Grok Bot\sand-secrets.json` and `Local State`                                                                |

Claude desktop and Grok Bot keep their tokens in Chromium "safeStorage": the AES key in `Local State`
is protected with Windows DPAPI for the current user. The monitor unprotects that key via PowerShell
(`ProtectedData.Unprotect`, current user scope), caches it in memory only, and decrypts the token in
process. Nothing decrypted is written to disk.

A unit test (`tests/unit/credentials-readonly.test.ts`) fails if expired or rejected tokens cause
any write to these files or any call to a token endpoint.

## What the app stores

All in `data/` (gitignored), or `USAGE_DATA_DIR` if set:

| File               | Contents                                     | Notes                                                              |
| ------------------ | -------------------------------------------- | ------------------------------------------------------------------ |
| `accounts.json`    | OpenCode account labels and `sk-…` API keys  | Plain JSON, protected by your Windows account's file permissions   |
| `settings.json`      | Toggles, nicknames, hidden accounts          | Validated server-side; unknown fields ignored                      |
| `seen-accounts.json` | Emails of provider accounts already seen     | No tokens. Lets a switched-away account stay listed                |
| `usage-cache.json`   | Last known usage, account emails/plan names  | Shown as `cached` when providers are unreachable                   |
| `.api-secret`      | 256-bit random secret                        | Shared by the Electron shell and the server                        |

Writes are atomic (temp file + rename) and serialised per file.

## Network

**Inbound.** The server listens on `127.0.0.1:3100` only. `proxy.ts` runs before every request:

- the `Host` header must be `127.0.0.1`, `localhost` or `[::1]` (blocks DNS-rebinding attacks from
  web pages);
- every `/api/*` request must carry the `x-usage-secret` header (constant-time comparison). The
  Electron preload injects it; ordinary browsers and other local pages get `401`.

Responses carry a Content-Security-Policy (`default-src 'self'`, `frame-ancestors 'none'`),
`X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY` and `Referrer-Policy: no-referrer`.

**Outbound.** Only each provider's own usage endpoint: `opencode.ai`, `api2.cursor.sh`,
`chatgpt.com`, `api.anthropic.com`. No telemetry. No token-refresh endpoints are called.

## Electron shell

- Single instance; renderers are sandboxed with context isolation, no Node integration, no
  `<webview>`.
- Navigation, redirects and new windows are blocked unless they stay on the app origin.
- IPC handlers ignore messages from any other origin, so the API secret is never handed to foreign
  content.
- All permission requests are denied except clipboard writes from the app (copy key).
- Electron is kept on a supported major version (Dependabot watches `desktop/`).

## Known limitations

- **Undocumented endpoints.** Provider usage endpoints are not public APIs and may change or break
  without notice; some requests identify as the provider's own client.
- **OpenCode keys are stored as plain JSON** in `data/accounts.json`, relying on Windows file
  permissions. Encrypting them with DPAPI is a possible future improvement.
- **Dev-only advisory.** `braces` (via `eslint-config-next`'s file globbing) has an advisory with no
  patched release yet. It is lint tooling only and never runs in the app; CI audits production
  dependencies at `high`.

## Reporting a problem

Please open a private security advisory on the GitHub repository rather than a public issue.
