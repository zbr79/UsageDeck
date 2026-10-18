# Plan: one packaged desktop app

Status: **planning only, nothing built yet** (2026-10-18). Branch `claudefix`, based on `7d36020`.

Confidence tags: HIGH = measured or read in this repo, MED = public docs or general knowledge not
re-checked here, LOW = estimate.

## 1. The question

Is the "web way" (Next.js + Electron) the wrong shape for this tool? Should it move to Python, or
something else?

**Short answer:** the web *UI* is fine. The problem is the *local web server* inside the app. Keep
TypeScript and React. Remove the server, and turn the project into a single installed Electron app.
Do not move to Python.

## 2. What is wrong with the current shape

- **It is not really a web app.** It is a desktop app that carries a private web server inside it.
  Electron starts `cmd /c npm run start` (`desktop/server-manager.js:47-57`). To run at all it
  needs Node, npm, a source checkout and a fresh `npm run build`. HIGH
- **No product basics.** There is no installer, no auto-update, no tray icon and no
  start-at-login. A search of `desktop/main.js` finds no `Tray` and no `autoUpdater`. HIGH
- **Heavy for a widget.** The live app runs 9 processes using about **417 MB of RAM** (4 Electron,
  2 node, 2 cmd). The repo needs about 860 MB of `node_modules` (494 MB in the root, 368 MB for
  Electron). HIGH, measured 2026-10-18
- **The server causes its own bugs:**
  - Port 3100 is hard-coded in 3 places.
  - A shared secret file exists only because other local programs can reach HTTP.
  - The same secret also encrypts the OpenCode keys, so losing it loses the keys.
  - Restarts happen only when the server process exits, and the crash counter resets after each
    recovery.

  HIGH (`desktop/server-manager.js:70-106`, `desktop/main.js:17`, `desktop/main.js:113-115`,
  `lib/opencode.ts:23-80`)
- **Next.js adds nothing here.** There is no SEO, no server rendering benefit, and no remote
  users. The `app/api/*` routes exist only to cross the process boundary the project made for
  itself. HIGH
- **Failures are silent.** `lib/` and `app/` contain no logging, the provider error paths use empty
  `catch {}` blocks, and the replies from undocumented endpoints are never validated. This is the
  biggest source of the "wrong in many ways" feeling. Changing language would not fix it. HIGH

What is good and should be kept:

- The React UI, the design tokens (`app/_ui/palette.ts`) and the accessible components
  (`app/_ui/Dialog.tsx`, `app/_ui/Tabs.tsx`).
- The pure logic in `lib/` (format, ranking, settings schema) and the 135 unit tests.
- The rule of never writing other apps' sign-ins.

## 3. Options compared

| Option                                    | What carries over                 | What is rewritten                                  | Size / RAM (est.)                | Verdict          |
| ----------------------------------------- | --------------------------------- | -------------------------------------------------- | -------------------------------- | ---------------- |
| **A. Electron, one process, no Next.js**  | UI, lib/, tests, all TypeScript   | API routes become in-app calls; remove server code | ~90 MB installer, ~200-250 MB RAM | **Recommended**  |
| B. Tauri 2 (Rust core, same React UI)     | UI and design                     | All of `lib/` in Rust (~1,500 lines)               | ~5-10 MB installer, ~50-80 MB RAM | Later, if wanted |
| C. Python (PySide6 / Qt)                  | Nothing                           | Everything (~8,700 lines)                          | ~60-150 MB bundle                | No               |
| D. C# / .NET (WPF or WinUI)               | Nothing                           | Everything                                         | small, most "native" on Windows  | No               |
| E. Keep as is and patch                   | Everything                        | Nothing                                            | as today                         | No               |

Sizes and RAM for A-D are MED/LOW general figures, not measured here. Today's 417 MB is measured.

**Why not Python:**

- It means rewriting the whole UI and all the logic in a language this project does not use.
- Packaging (PyInstaller or Nuitka) is heavy and often triggers antivirus false alarms. MED
- Transparent, always-on-top windows are possible in Qt, but no better than today.
- No maintained GUI tool in this space uses Python. Python appears only in terminal monitors
  (Claude-Code-Usage-Monitor). MED, from the 2026-10-18 tool survey

It is all cost and no gain for this problem.

**Why not Tauri first:**

- Most of the reference tools use Tauri or Rust (Win-CodexBar, OpenQuota, CodeZeno), and the
  footprint is far smaller. MED
- But the fragility lives in failure handling, not in the language. A Rust rewrite would mean
  writing ~1,500 lines in a language the owner does not work in, before any reliability gain.
- Doing A first keeps the provider layer behind a clean interface. Moving to B later then means
  swapping only that layer, because the UI carries over unchanged.

**Also worth knowing:** CodeZeno's Claude-Code-Usage-Monitor (Windows, Rust, MIT) already covers
almost the same providers, including OpenCode Go, Cursor, Grok, Codex and Claude, with multiple
accounts. If maintaining this project stops being fun, using that tool or contributing to it is a
real option. MED, from https://github.com/CodeZeno/Claude-Code-Usage-Monitor

## 4. Target shape (option A)

```
UsageDeck.exe (one Electron app, installed per user, no admin)
├── main process
│   ├── store: settings, accounts, cache in %APPDATA%\UsageDeck (keys sealed with Windows DPAPI
│   │          through Electron safeStorage, no shared secret file)
│   ├── tray icon, start at login, single instance, global hotkey
│   └── typed IPC handlers (one contract file shared with the UI)
├── provider worker (Electron utilityProcess): a hung provider can't freeze the UI
│   └── one module per provider: ordered sources -> validated snapshot or a typed error
└── two windows (widget, settings): React + Tailwind built by Vite, loaded from files,
    sandboxed, no network access, no open port
```

- **Provider contract:** each provider returns either a snapshot validated against a schema, or
  one of these errors: `missing-credentials`, `auth-rejected`, `rate-limited`, `network`, `http`,
  `schema-changed` or `locked`. The card shows which one happened.
- **Last good value:** a good cached value is never overwritten by an empty or invalid reply.
- **Polling:** each provider has a floor, Claude at least 60 s because its usage endpoint
  rate-limits hard. Polling backs off on errors and pauses while the PC sleeps.
- **Logging:** a rotating log file with secrets redacted, plus a "Copy diagnostics" item in the
  tray menu.

## 5. Stages

Each stage ends with checks that must pass before the next stage starts.

**Stage 0: safety net (small, before moving anything)**

- Add `data/` as a whole to `.gitignore`. Today `data/logs/` and `data/seen-accounts.json` (which
  holds account emails) are not ignored. HIGH, `git status` on 2026-10-18
- Save one real reply per provider as a redacted test fixture, and write parser tests against those
  fixtures.
- Check: `npm run check` passes, and every provider has at least one fixture test.

**Stage 1: UI out of Next.js**

- Move `app/widget` and `app/settings` into a Vite + React renderer.
- Replace `fetch("/api/...")` with a typed bridge. While still on the old server, the bridge can
  call HTTP underneath.
- Check: the existing e2e screenshots and accessibility scans pass unchanged against the Vite build.

**Stage 2: server logic into the app**

- `lib/` moves almost unchanged into the provider worker and the main process.
- The `app/api/*` routes become IPC handlers.
- Delete `proxy.ts`, `desktop/server-manager.js`, `desktop/restart-policy.js`, the secret file and
  port 3100.
- Check:
  - one launch starts one app;
  - `netstat` shows no listening port;
  - all five providers show data;
  - killing the worker brings it back within 10 s.

**Stage 3: reliability**

- Schema validation, typed error states on the cards, the last-good-value rule and logging.
- Fix the defects in section 6, each with a failing test written first.
- Check: every defect has a test that failed before its fix, and a fake bad reply shows
  "schema changed" instead of blanks.

**Stage 4: packaging**

- A per-user installer, tray icon, start at login and auto-update.
- A one-time move of the old `data/` folder into `%APPDATA%\UsageDeck`. OpenCode keys are
  re-sealed with DPAPI.
- Check:
  - install on a fresh Windows user profile;
  - the app starts at login;
  - an upgrade keeps settings and keys;
  - uninstall leaves no running process.

**Stage 5: extras (optional, after the above)**

- Warning thresholds and notifications.
- Usage history with a pace line.
- Privacy mode (show "Account 1" instead of email addresses).
- A small CLI or JSON output.

## 6. Known defects carried into the plan

These come from the 2026-10-18 audit. Typecheck, lint and 135 unit tests all pass. Not fixed yet.

| #   | Where                                                              | Defect                                                                                                                                                   | Stage |
| --- | ------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------- | ----- |
| 1   | `lib/ranking.ts` `earliestReset`                                   | Uses the soonest reset. An account full on both weekly and monthly limits is blocked until the later one, so it ranks too high.                          | 3     |
| 2   | `lib/codex.ts:55-67`                                               | Reads only the 5-hour window. The weekly window is ignored, so a weekly-exhausted card shows the wrong percentage and reset time.                         | 3     |
| 3   | `lib/codex.ts:87`, `lib/cursor.ts:136`, `lib/provider-cache.ts:55` | Any HTTP 200 counts as success, so an unexpected reply overwrites good cached data with blanks.                                                           | 3     |
| 4   | `lib/opencode.ts:23-80`                                            | If the secret file changes, keys silently vanish from the UI, and the next save erases them from disk.                                                    | 2, 4  |
| 5   | `lib/claude.ts:120-133, 218-226`                                   | Usage can come from the desktop app's token but be filed under Claude Code's email.                                                                       | 3     |
| 6   | `desktop/server-manager.js`                                        | No health check, an adopted server is never restarted, and the crash cap never triggers.                                                                  | 2     |
| 7   | `app/widget/_lib/useWidgetData.ts:71`                              | A refresh asked for during a poll is dropped, so a settings change can take up to 60 s to show.                                                          | 3     |
| 8   | `lib/cursor.ts:39-69`                                              | A locked Cursor database is cached for 60 s as "no account", and the card disappears without a message.                                                   | 3     |
| 9   | `lib/opencode.ts:137-140`                                          | An OpenCode 401/403 is not shown as a rejected key. The old server log has three 403s.                                                                   | 3     |
| 10  | `lib/seen-accounts.ts:71`                                          | Keeps the oldest 50 entries, so new accounts are dropped.                                                                                                | 3     |

Items 1-3 were re-read by hand: HIGH. Items 4-10 come from the audit agent with file evidence: MED.

## 7. Risks

- **Undocumented endpoints.** Every provider except OpenCode is read from an endpoint the vendor
  does not document. Any of them can change without notice. Mitigation: fixtures, the
  `schema-changed` state, and the last good value.
- **Claude rate limits and policy.** `/api/oauth/usage` rate-limits hard and is unsupported. In
  February 2026 Anthropic said subscription OAuth tokens are for Claude Code and Claude.ai only.
  Read-only polling is not addressed. MED
  (https://winbuzzer.com/2026/02/19/anthropic-bans-claude-subscription-oauth-in-third-party-apps-xcxwbn/)
- **Unsigned installer.** Windows SmartScreen warns on the first run unless the app is code-signed,
  which costs money. That is normal for tools like this. MED
- **New dependencies need the owner's approval before stage 1:** `vite`, `@vitejs/plugin-react`,
  `electron-builder`, `electron-updater`, `electron-log`, `zod`. `next` is removed.

## 8. Choices left to the owner (defaults applied until told otherwise)

- **Main face:** keep the floating widget and **add** a tray icon (default), or go tray-first like
  most other tools.
- **Platform:** Windows only (default). The credential readers are Windows-specific today.
- **Releases:** build the installer locally only (default). Auto-update needs releases published to
  GitHub, which happens only when the owner asks.

## 9. Reference tools (design ideas, not code)

- Win-CodexBar, Tauri + Rust, Windows: https://github.com/nesszer/Win-CodexBar. Thresholds, hooks
  on low / out / reset, privacy mode, float bar.
- CodeZeno Claude-Code-Usage-Monitor, Rust, Windows taskbar:
  https://github.com/CodeZeno/Claude-Code-Usage-Monitor. Same providers, typed failure categories.
- OpenQuota, Tauri 2 + Svelte: https://github.com/deviffyy/OpenQuota. Tray popup or floating panel,
  keys in the OS store.
- CodexBar, Swift, macOS: https://github.com/steipete/CodexBar. Provider authoring guide, ordered
  data sources, stale-data dimming, JSON CLI.
- OpenUsage: https://github.com/robinebers/openusage. Pipeline of auth -> usage -> mapper ->
  snapshot; cached values first, then refresh.
- AIUsageMonitor, C# WPF, Windows: https://github.com/ansonliam/AIUsageMonitor. Widget, tray and
  taskbar strip, with a note that polling Claude about once a minute avoids 429s.
