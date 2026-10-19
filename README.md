# UsageDeck

An always-on-top floating Windows widget that tracks AI subscription usage across **five
providers**. One `usagedeck.exe` (Tauri 2: Rust core, React frontend in WebView2): no local server,
no Electron, no Node at runtime.

| Provider        | What's shown                                                                             | Data source                                                                   |
| --------------- | ---------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| **OpenCode Go** | Rolling / Weekly / Monthly usage per account, reset countdowns, DeepSeek peak-hour timer | `opencode.ai/zen/go/v1/usage` (API key you add in Settings)                   |
| **Cursor**      | First Party & API usage, plan badge, billing-cycle reset                                 | Cursor's local sign-in → `api2.cursor.sh`                                      |
| **Grok Bot**    | Weekly usage %, plan badge, reset countdown                                              | Grok Bot's own sign-in (falls back to Cursor's) → `api2.cursor.sh` sand usage |
| **Codex**       | Usage %, plan badge (Free/Plus/Pro), reset credits, reset countdown                      | Codex CLI sign-in → ChatGPT `wham/usage`                                      |
| **Claude**      | 5-hour and weekly usage, plan badge, reset countdowns                                    | Claude desktop / Claude Code sign-in → Anthropic OAuth usage                  |

Other apps' sign-ins are **read, never written**: UsageDeck never refreshes or rewrites another
app's tokens (doing so rotates their refresh tokens and logs them out). If a sign-in has expired,
the card shows the last known numbers with a `re-auth` hint — open that app once to refresh it.
See [SECURITY.md](SECURITY.md) for exactly what is read, stored and sent.

---

## Architecture

```
Rust core (src-tauri/)                           React frontend (src/)
├── providers/  one module per provider          ├── widget/    floating panel (widget.html)
├── store.rs    settings, accounts, caches       ├── settings/  settings window (settings.html)
├── api.rs      commands the frontend invokes    ├── ui/        palette + accessible primitives
├── win/        DPAPI, screen sampling           └── lib/       ranking, formatting, settings rules
├── cli.rs      --import-old-data, --dump
└── lib.rs      windows, tray, autostart, log
```

- The frontend calls Rust through Tauri `invoke`; nothing listens on a port.
- Providers share one caching policy: short TTL, a single in-flight request, and a persisted
  last-known value used when a provider is unreachable.
- Each widget card loads independently: one slow or failing provider never blanks the others.

## Requirements

- **Windows 10/11** with WebView2 (preinstalled on Windows 11)
- To build: Node.js 22.13+, the Rust stable toolchain (rustup), and the Tauri 2 Windows
  prerequisites (MSVC build tools)
- Optional: Cursor, Grok Bot, Codex CLI, Claude desktop / Claude Code — each is auto-detected

## Your data

Everything UsageDeck stores lives in `%APPDATA%\com.usagedeck.app`:

- `settings.json` — toggles, themes, nicknames, hidden accounts, provider order.
- `accounts.json` — OpenCode account labels (emails only, no keys).
- `seen-accounts.json` — emails of Cursor, Grok Bot, Codex and Claude accounts already seen, so
  switching accounts in those apps does not drop the earlier one. Tokens are not stored.
- `usage-cache.json` — last known usage per provider and account, so the widget has something to
  show after a restart or outage (marked `cached`).
- `window-pos.json` — where the widget was last placed.

OpenCode API keys live in **Windows Credential Manager** (service `UsageDeck`), never in a file.
The settings window only ever shows a hint (`sk-…a1b2`). All file writes are atomic (temp file +
rename), so a crash can't corrupt settings.

The installed app starts at login. Copies run from a cargo `target` folder never register
autostart.

## Features

### Widget

- Compact panel with the active OpenCode account plus every enabled provider; right-click (or the
  tray menu) → _Expand_ / _Collapse_ for all OpenCode accounts, _Refresh_, _Settings_, _Quit_
- **Adaptive theme** — in `auto`, samples the screen around the widget and switches card style
  with hysteresis; `light` and `dark` are fixed (no sampling)
- **Change detection** — when an account's usage rises, a ▲ shows for 20 s and the most recently
  used account floats to the top
- **Reset countdowns** (`5d 7h 45m`) and **reset memory** — cached cards are marked `cached`; a
  window whose stored reset time has passed shows `--` until fresh data arrives
- **States, consistently on every card** — loading, unavailable, `cached`, `re-auth`, `key
  rejected` (OpenCode 401 / 403), exhausted (red)
- **Unavailable banner** — "Usage unavailable · updated 3m ago" with Retry
- **Copy key** — hover the OpenCode icon on a card (a copy glyph appears) and click
- Settings changes apply live; right-click or **Shift+F10** opens the menu

### Settings (right-click widget → Settings)

- General: panel theme, widget theme, show provider names, DeepSeek peak-hour warning
- Accounts: one tab per provider. Drag a provider tab to change its order on the widget
- Every provider can be hidden as a whole. Each account has its own nickname and can be hidden on
  its own. OpenCode accounts are added with an API key and can all appear on the widget. Cursor,
  Grok Bot, Codex and Claude only show the account that app is signed into; other sign-ins stay in
  Settings so you can nickname or hide them before switching
- Fully keyboard accessible: real dialogs (focus trapped, Escape closes the topmost), arrow-key
  tabs, switches and radio groups announced to screen readers

### Ranking rules

1. Accounts at 100% on **monthly or weekly** usage sink to the bottom (red glow)
2. Among exhausted accounts, the one resetting soonest is ranked first
3. Otherwise, the most recently used account floats to the top (green glow)

## Command-line modes

Headless: each prints a short report and exits.

- `usagedeck.exe --import-old-data <dir>`: one-time copy of the old Next.js app's `data/` folder.
  Settings go through the usual validation; the seen-accounts list and usage cache are copied when
  their shape matches; OpenCode keys (plaintext or `enc:v1:`) move into Windows Credential Manager.
  Keys are never printed.
- `usagedeck.exe --dump`: fetch every provider once and print a redacted JSON summary (label cut to
  two characters, plan, each window's percent and reset, stale / authExpired / error).

## Development

| Command                | Description                                                                      |
| ---------------------- | -------------------------------------------------------------------------------- |
| `npm install`          | Install the frontend and the Tauri CLI                                           |
| `npm run tauri dev`    | Run the app with hot reload (Vite on port 1420)                                  |
| `npm run tauri build`  | Release `usagedeck.exe` plus the per-user NSIS installer                         |
| `npm run build`        | Type-check and build the frontend into `dist/`                                   |
| `npm test`             | Frontend unit tests (Vitest)                                                     |
| `npm run format`       | Format with Prettier                                                             |
| `npm run check`        | Type-check, format check, frontend tests and `cargo test`                        |

CI (GitHub Actions, Windows) runs the frontend build, tests and format check, `cargo fmt --check`,
`cargo clippy -D warnings`, `cargo test`, and `npm audit` on every push and pull request.

## History

UsageDeck started as a Next.js server plus an Electron widget. That app was retired and removed;
this Tauri app keeps its providers, cards, ranking and settings. `--import-old-data` carries a
user's old data across. The design notes are in [`docs/plans/`](docs/plans/).

## Project layout

```
src/                  # React frontend
├── widget/           # floating panel (_components/ cards, _lib/ data + theme)
├── settings/         # settings window (_components/, _lib/hooks.ts)
├── components/toast/ # toast system
├── ui/               # design tokens (palette.ts) + accessible primitives
├── lib/              # bridge to Rust, ranking, formatting, settings rules (unit-tested)
└── public/           # provider icons
src-tauri/            # Rust core (providers/, win/, store, cli, tray), tauri.conf.json, icons
tests/                # frontend unit tests (Vitest)
widget.html           # widget page entry
settings.html         # settings page entry
docs/plans/           # design and build notes
```
