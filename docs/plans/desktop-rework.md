# Plan: rebuild UsageDeck as one installed desktop app (Tauri)

Status: **planning only, nothing built yet** (2026-10-18). Branch `claudefix`, base `7d36020`.

Confidence tags:

- HIGH: measured or read in this repo, or read in official docs.
- MED: public sources or general knowledge, not re-checked here.
- LOW: an estimate.

## 1. Decision

- **Stack: Tauri 2.** The engine (data fetching, storage, scheduling) is written in Rust. The
  windows stay React + TypeScript + Tailwind, built with Vite.
- Default applied 2026-10-18; the owner can still overrule it.
- **Dropped:**
  - Next.js and the local server on port 3100.
  - Electron.
  - The shared secret file.
  - Python: not considered further (see section 3).
- **Kept:** the look and the components of the widget and settings windows, the rules in `lib/`
  (ported, not thrown away), and the rule of never writing other apps' sign-ins.

## 2. Why the current shape is fragile

- **It is a desktop app with a hidden web server inside.** Electron runs `cmd /c npm run start`
  (`desktop/server-manager.js:47-57`). It needs Node, npm, the source checkout and a fresh build,
  and it has no installer. HIGH
- **Heavy.** The running app uses 9 processes and about 417 MB of RAM. The repo needs about 860 MB
  of `node_modules`. HIGH, measured 2026-10-18
- **Plumbing bugs:**
  - A fixed port.
  - A secret file that both locks the API and encrypts the OpenCode keys.
  - A server restart only when the process exits, never when it hangs.

  HIGH (`desktop/server-manager.js:70-106`, `lib/opencode.ts:23-80`)
- **Silent failures.**
  - Nothing is logged.
  - Error paths use empty `catch {}` blocks.
  - Provider replies are never checked, so any HTTP 200 counts as good data.

  This is the main cause of the wrong numbers, and a new language does not fix it by itself. The
  new design does (section 4). HIGH

## 3. Options weighed

| Option                                     | Good                                                                                                      | Bad                                                                                                    |
| ------------------------------------------ | --------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| **Tauri 2: Rust engine + React windows**   | Small installer (a few MB) and low RAM. Same shape as the best Windows competitors. Rust's strict types catch mistakes in reply parsing. The UI survives. | The engine (~1,500 lines today) is rewritten in Rust. The owner reads Rust less easily. New toolchain. |
| Electron, one process, no server           | Keeps all the TypeScript. The quickest route to stable.                                                   | Still ~90 MB to install and ~200 MB of RAM (LOW). Still feels like a browser.                          |
| Python (PySide6 / Qt)                      | An easy language.                                                                                         | Rewrites everything. Packaging an .exe is heavy and trips antivirus (MED). No reliability gain.        |
| C# / .NET (WPF / WinUI)                    | The most native Windows feel.                                                                              | Rewrites everything in a new language and a new UI framework.                                          |

Tauri was chosen because the owner wants a real native-feeling app, not a web app, and AI agents do
most of the coding. The weak spot is reviewing the Rust code. Section 4 reduces that risk:
recorded-reply tests define the behaviour, so the reviewer checks tests, not Rust.

## 4. Stack

**Windows (frontend)**

- Vite + React 19 + TypeScript + Tailwind 4. Components move from `app/_ui`, `app/widget` and
  `app/settings`.
- `src/bridge.ts` is the only file that talks to the engine (Tauri `invoke` and events). Its types
  are generated from the Rust types, so the two sides cannot drift apart: `ts-rs` by default, or
  `tauri-specta` if it is stable at the time. MED
- Tests: Vitest for logic. Playwright against the Vite build with a fake bridge, which keeps the
  current e2e screenshots and accessibility scans.

**Engine (Rust, `src-tauri/`)**

| Job                                                                                         | Crate                                                                                   |
| ------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| App shell, tray icon, transparent always-on-top frameless window                           | `tauri` 2                                                                               |
| Start at login, one instance only, hotkey, notifications, rotating log file                | official plugins `autostart`, `single-instance`, `global-shortcut`, `notification`, `log` (HIGH: plugin docs) |
| Auto-update (later)                                                                         | official plugin `updater`                                                               |
| HTTP                                                                                        | `reqwest` (rustls) on `tokio`                                                           |
| Reply checking                                                                              | `serde` structs per provider: a reply that doesn't match the type becomes `schema-changed` |
| Typed errors                                                                                | `thiserror`                                                                             |
| Cursor's sign-in database (read-only)                                                       | `rusqlite` (bundled). Replaces the still-experimental `node:sqlite`                     |
| Claude desktop / Grok Bot sealed tokens                                                     | `windows` crate: direct DPAPI call, plus `aes-gcm`. Replaces the PowerShell call in `lib/os-crypt.ts:11-30`, which can take up to 10 s |
| OpenCode API keys                                                                           | `keyring`: Windows Credential Manager, tied to the Windows user. Replaces the secret file |
| Adaptive theme                                                                              | `windows` crate GDI: sample a small strip around the widget instead of a full-screen capture every 3 s (`desktop/main.js:249`) |
| Times                                                                                       | `chrono`                                                                                |

**Engine design**

- **One `Provider` trait.** Each provider:
  - lists its sources in order (for example, Claude desktop token, then Claude Code token);
  - returns either a typed snapshot, or one of these errors: `missing-credentials`,
    `auth-rejected`, `rate-limited`, `network`, `http`, `schema-changed` or `locked`.
- **Scheduler.**
  - Each provider has a minimum poll interval; Claude's is 60 s or more because its endpoint
    rate-limits hard.
  - Polling backs off on errors and pauses while the PC sleeps.
  - A refresh requested during a poll is queued, not dropped.
- **Store.** Settings, nicknames and the cache live in `%APPDATA%\UsageDeck`, written atomically.
  A good cached value is never replaced by an empty or invalid reply.
- **Cards** show the error type in plain words: "sign in to Codex again", "Cursor is busy",
  "reply format changed".

**Installer and build**

- NSIS installer in per-user mode, so no admin is needed. That is Tauri's default (HIGH: tauri.app
  config reference).
- The installer uses the WebView2 that Windows already has: version 147 is installed on this PC
  (HIGH, registry).
- Not code-signed at first, so SmartScreen warns on the first run.
- CI on GitHub Actions (Windows) runs:
  - `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`;
  - the npm checks;
  - `tauri build`.

**Repo layout during the move**

```
native/                 the new app (own package.json, so the old app keeps working)
├── src/                React windows: widget/, settings/, ui/, bridge.ts
└── src-tauri/
    ├── src/lib.rs      tray, windows, plugins, commands
    ├── src/store/      settings, cache, keys
    ├── src/scheduler.rs
    ├── src/providers/  mod.rs (trait + errors), opencode.rs, codex.rs, claude.rs, cursor.rs, grok.rs
    ├── src/win/        dpapi.rs, os_crypt.rs, screen.rs
    └── tests/fixtures/ one folder per provider: recorded replies, redacted
app/ lib/ desktop/ ...  the old app, unchanged, used daily until step 8
```

## 5. Rewrite process

Work happens on `claudefix` in small commits. Each step ends with checks that must pass before the
next one starts. The old widget stays in daily use the whole time.

1. **Owner setup (only the owner can do this).** Approve installing the Rust toolchain with
   `rustup` (stable, MSVC). Visual Studio Build Tools 2022 is already installed (HIGH, vswhere),
   and Rust is not (HIGH, `cargo` not found). Also approve the new packages listed in section 7.
2. **Freeze the current behaviour as a spec.**
   - Add `data/` as a whole to `.gitignore`. `data/logs/` and `data/seen-accounts.json` (which holds
     account emails) are not ignored today (HIGH, `git status`).
   - Record one or more real replies per provider, with tokens and emails redacted, into
     `native/src-tauri/tests/fixtures/`. Include an error reply where possible.
   - Write `docs/plans/provider-spec.md`. For each provider it lists the fields read, the windows,
     the reset rules, the account identity, what each error should show, and the fixes from
     section 6.
   - Check: every provider has at least one fixture, and the spec covers every row of section 6.
3. **Empty shell.**
   - Create the Tauri app in `native/`: tray icon, one transparent always-on-top window, single
     instance, start at login, log file.
   - Check:
     - the installer builds and installs without admin;
     - `netstat` shows no listening port;
     - a second launch focuses the first.
4. **Windows moved over (no real data yet).**
   - Move the widget and settings components into `native/src/` and talk only through `bridge.ts`.
   - A fake bridge serves the old e2e fixture data.
   - Check: the existing screenshot tests and accessibility scans pass against the new build.
5. **Engine core.**
   - Build the store, scheduler, `Provider` trait and typed errors.
   - Import the old `data/settings.json` and nicknames, and move the OpenCode keys into Credential
     Manager (decrypted once with the old secret).
   - Check: Rust unit tests for the store and scheduler (queued refresh, backoff, last good value),
     and the import is tested on a copy of the real data.
6. **Providers, one at a time, easiest first: OpenCode, then Codex, Claude, Cursor, Grok.** For
   each one:
   - write the parser test from its fixtures first and see it fail, then make it pass;
   - include that provider's rows from section 6 as test cases;
   - switch that card from the fake bridge to the live engine.

   Check: `cargo test` passes for that provider, a deliberately broken fixture shows "reply format
   changed", and the live card matches the old widget for the same account.
7. **Feature parity.** Port the remaining features from `README.md`:
   - adaptive theme;
   - the ▲ usage-rise marker and float-to-top;
   - ranking;
   - copy key;
   - drag to reorder provider tabs;
   - hidden accounts;
   - DeepSeek peak-hour warning;
   - expand/collapse;
   - re-auth hint.

   Check: a written parity checklist is all ticked, and both apps run side by side for 3 days with
   matching numbers. Any difference is logged and explained.
8. **Retire the old app.**
   - Remove `app/`, `lib/`, `desktop/`, `proxy.ts` and the Next config.
   - Move `native/` up to the repo root and update the README, SECURITY.md and CI.
   - Check: a fresh install on a clean Windows user profile, a start at login, and an upgrade that
     keeps settings.
9. **Extras (optional):**
   - warning thresholds and notifications;
   - usage history with a pace line;
   - privacy mode;
   - auto-update through GitHub Releases, published only when the owner asks.

## 6. Known defects the rewrite must not repeat

These come from the 2026-10-18 audit (typecheck, lint and 135 unit tests pass). Each row becomes a
test in step 6.

| #   | Old location                                                       | Defect                                                                                                                                                    |
| --- | ------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `lib/ranking.ts` `earliestReset`                                   | Uses the soonest reset. An account full on both weekly and monthly limits is blocked until the later one.                                                 |
| 2   | `lib/codex.ts:55-67`                                               | Reads only the 5-hour window. The weekly window is ignored.                                                                                               |
| 3   | `lib/codex.ts:87`, `lib/cursor.ts:136`, `lib/provider-cache.ts:55` | Any HTTP 200 counts as success, so a bad reply overwrites good cached data.                                                                               |
| 4   | `lib/opencode.ts:23-80`                                            | A changed secret silently hides the keys, and the next save erases them.                                                                                  |
| 5   | `lib/claude.ts:120-133, 218-226`                                   | Usage from the desktop app's token can be filed under Claude Code's email.                                                                                |
| 6   | `desktop/server-manager.js`                                        | No health check, an adopted server is never restarted, and the crash cap never triggers.                                                                  |
| 7   | `app/widget/_lib/useWidgetData.ts:71`                              | A refresh asked for during a poll is dropped.                                                                                                             |
| 8   | `lib/cursor.ts:39-69`                                              | A locked database is cached as "no account", and the card vanishes.                                                                                       |
| 9   | `lib/opencode.ts:137-140`                                          | A 401/403 is not shown as a rejected key.                                                                                                                 |
| 10  | `lib/seen-accounts.ts:71`                                          | Keeps the oldest 50 entries, so new accounts are dropped.                                                                                                 |

Rows 1-3 were re-read by hand: HIGH. The rest come from the audit agent with file evidence: MED.
Row 6 disappears with the server.

## 7. Risks and what the owner must approve

- **New tools and packages (owner approval needed):**
  - the Rust toolchain;
  - npm: `vite`, `@vitejs/plugin-react`, `@tauri-apps/cli`, `@tauri-apps/api`, and the plugin JS
    packages;
  - Rust crates: those listed in section 4.
- **Undocumented endpoints.** Every provider except OpenCode can change without notice.
  Mitigations: fixtures, the `schema-changed` state, and the last good value.
- **Claude.**
  - `/api/oauth/usage` rate-limits hard and is unsupported.
  - In February 2026 Anthropic said subscription OAuth tokens are for Claude Code and Claude.ai
    only. Read-only polling is not addressed. MED
    (https://winbuzzer.com/2026/02/19/anthropic-bans-claude-subscription-oauth-in-third-party-apps-xcxwbn/)
- **Reviewing Rust.** The owner checks behaviour through the fixture tests and the side-by-side run
  in step 7, not by reading every line.
- **Scale of work.** Steps 2-8 are a full rewrite of the engine (~1,500 lines of TypeScript become
  ~2,000-2,500 lines of Rust, LOW estimate). The UI port is mostly mechanical.

## 8. Choices left to the owner (defaults applied)

- Keep the floating widget as the main window **and** add a tray icon (default), or go tray-first.
- Windows only (default).
- Build the installer locally only. Publish releases only when the owner asks.

## 9. Reference tools (design ideas, not code)

- Win-CodexBar, Tauri + Rust, Windows: https://github.com/nesszer/Win-CodexBar. Thresholds, hooks
  on low / out / reset, privacy mode, float bar.
- CodeZeno Claude-Code-Usage-Monitor, Rust, Windows taskbar:
  https://github.com/CodeZeno/Claude-Code-Usage-Monitor. Same providers, typed failure categories.
- OpenQuota, Tauri 2 + Svelte: https://github.com/deviffyy/OpenQuota. Tray popup or floating panel,
  keys in the OS store.
- CodexBar, Swift, macOS: https://github.com/steipete/CodexBar. Provider authoring guide, ordered
  data sources, stale-data dimming.
- OpenUsage: https://github.com/robinebers/openusage. Pipeline of auth -> usage -> mapper ->
  snapshot; cached values first, then refresh.

## 10. Build status (native rebuild, wave 3)

**Done** (branch `wave3-finish`, on top of the wave 2 merge):

- Rust shell, store, API dispatcher, five providers, DPAPI cookie decrypt and theme sampler
  (waves 1-2). `cargo test --lib`: 34 passed, 5 ignored; `npm test`: 85 passed; `npm run build` clean.
- The OpenCode card shows a "key rejected" hint when OpenCode refuses the key (`authExpired`).
- `usagedeck.exe --import-old-data <dir>`: settings through the patch rules, seen-accounts and usage
  cache when the shape matches, OpenCode keys (plaintext or `enc:v1:`) into Credential Manager.
  Run once on the owner's PC: 6 of 6 keys moved, settings / seen list / cache imported.
- `usagedeck.exe --dump`: redacted JSON of every provider route.
- Autostart: release builds only, and never when the exe runs from a cargo target folder.
- `npm run tauri build` produces the per-user NSIS installer (about 4 MB).
- Comparison with the old widget on the same PC: every provider's plan and window percentages
  match within 1 point (OpenCode 6 accounts, Cursor, Grok, Codex, Claude). Both apps see the
  Codex sign-in as expired and OpenCode answering HTTP 403.
- Visual smoke: the release widget renders the Claude and OpenCode cards with their bars.

**What differs from the old app:**

- No local server, no `x-usage-secret`, no port 3100; the frontend calls Rust through `invoke`.
- OpenCode keys live in Windows Credential Manager, not AES-encrypted in `accounts.json`.
- A rejected OpenCode key (401 / 403) is flagged as `authExpired` and shown as "key rejected";
  the old app only showed the row as stale.
- Data lives in `%APPDATA%\com.usagedeck.app`, not in the repo's `data/` folder.

**Left:**

- The owner installs the NSIS build, checks it for a day, then retires the old Electron widget
  (remove its shortcut / process) and later `app/`, `lib/`, `desktop/`.
- The OpenCode card hides its title when both "key rejected" and the peak hint show (width).
- The identifier ends in `.app` (Tauri warns; harmless on Windows, kept to match the contract).

**2026-10-18:** the old Next.js + Electron app (`app/`, `lib/`, `desktop/`, its tests and config)
was removed and `native/` moved to the repo root; paths in this file before this line are as built.

### Review map (wave 3)

- `native/src-tauri/src/cli.rs:28-57` `try_run`: picks a headless mode before any window exists.
- `cli.rs:66-78` `attach_console`: GUI-subsystem exe attaches to the parent console when stdout
  is not inherited.
- `cli.rs:81-114` `aes_key`, `b64url`, `decrypt_key`: port of `lib/key-crypto.ts` decrypt.
- `cli.rs:117-226` `seen_compatible`, `cache_compatible`, `import_old_data`: the import; keys go
  through `Store::update_accounts`, never printed.
- `cli.rs:229-325` `mask`, `reset_for`, `windows_of`, `plan_of`, `redact_account`, `dump`: the
  redacted summary.
- `cli.rs:325-369` tests: `enc:v1` round trip with a value encrypted in the test; redaction.
- `native/src-tauri/src/lib.rs:215-226` `running_from_cargo_target`; `lib.rs:229-233` CLI hook in
  `run`; `lib.rs:257-264` autostart guarded by it.
- `native/src-tauri/Cargo.toml`: `sha2` (already locked via tauri), `Win32_System_Console`.
- `native/src/lib/types.ts:78-79` `authExpired?`; `native/src/widget/_components/cards.tsx:135-144`
  "key rejected" hint on the OpenCode card.
