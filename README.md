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
| **Claude**      | 5-hour and weekly usage, plan badge, reset countdowns; spend rate ($/h) and pace (%/h, projected limit time) | Claude desktop / Claude Code sign-in → Anthropic OAuth usage; dollars from the Claude Code logs on this PC (usage fields only) |

Other apps' sign-ins are **read, never written**: UsageDeck never refreshes or rewrites another
app's tokens (doing so rotates their refresh tokens and logs them out). If a sign-in has expired,
the card shows the last known numbers with a `re-auth` hint — open that app once to refresh it.
For Cursor the numbers turn gray and the hint is an **Open Cursor** button: it starts Cursor,
which renews its own sign-in, and the card shows live numbers again within about 10 seconds of that.
If the card stays gray while Cursor works, the log names the endpoint, the HTTP status and whether
the access token Cursor stored had expired (see Logs below).
The Cursor card turns red when both included buckets are at 100% and on-demand usage is off (or its
monthly limit is spent); with on-demand budget left it stays usable.
See [SECURITY.md](SECURITY.md) for exactly what is read, stored and sent.

---

## Install

Download and run the installer:
**<https://github.com/zbr79/UsageDeck/releases/latest/download/UsageDeck-Setup.exe>**

This link always serves the newest version. The installer is one small window: click **Install**,
watch the progress bar, then click **Launch UsageDeck**. If UsageDeck is already installed, the
button says **Update** or **Reinstall** instead, and your settings and keys stay. **Options** has
two switches, both on for a new install:

- **Create desktop shortcut**. The Start menu entry is always created.
- **Start UsageDeck with Windows**. This sets the same option as Settings, so you can change it
  later there.

The installer is per-user and needs no admin rights. It is not code-signed, so Windows SmartScreen
may show "Windows protected your PC": choose **More info → Run anyway**. On a PC without the
WebView2 runtime, the installer shows the classic setup wizard instead, which installs WebView2
first.

For scripted installs, `UsageDeck-Setup.exe` accepts the NSIS installer's own switches and then
shows no window: `/S` installs silently, `/D=<folder>` (last) picks the folder, and `/NS` skips
all shortcuts. Each release also has the plain NSIS installer, `UsageDeck_<version>_x64-setup.exe`.

### Updates

UsageDeck checks GitHub 15 seconds after it starts and then every hour. When a newer version is
out, it downloads the installer in the background and checks its size and SHA-256 against the
release. Then a banner at the top of the widget says "Update X is ready":

- **Restart** installs the update silently and starts the new version. No installer window
  opens; the widget disappears for a few seconds and comes back updated.
- **Later** hides the banner until a newer version comes out or UsageDeck restarts. The download
  is kept, so Restart later does not download again.

Background checks show nothing else: no "up to date" note and no errors. To check at once, use
**Check for updates** in any of these places. The result shows in the widget's banner.

- the widget's right-click menu;
- Settings → General → Updates, which also shows the installed version and when UsageDeck last
  checked, and offers **Restart to update** once an update is ready;
- the tray menu, which also brings up the widget. The tray menu's first line shows the installed
  version.

### Coming from UsageDeck 1.0.x

Versions 1.0.0 and 1.0.1 were a different app (Electron). Both apps appear as "UsageDeck" under
Settings → Apps; the old one has version 1.0.x. The old app does not update to this one, and it
keeps its own data. To move your accounts and settings across:

1. Install this version, then quit it (tray menu → **Quit**) and quit the old app.
2. In a Command Prompt, copy the old app's data:

   ```bat
   "%LOCALAPPDATA%\UsageDeck\usagedeck.exe" --import-old-data "%APPDATA%\UsageDeck\data"
   ```

   This reads the old `accounts.json`, `settings.json` and `.api-secret` and moves the OpenCode
   keys into Windows Credential Manager. It never prints a key.

3. Uninstall the 1.0.x "UsageDeck" from Settings → Apps, then start UsageDeck again.

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
- `settings-window.json` — the settings window's last position and size.
- `accounts.json` — OpenCode account labels (emails only, no keys).
- `seen-accounts.json` — emails of Cursor, Grok Bot, Codex and Claude accounts already seen, so
  switching accounts in those apps does not drop the earlier one. Tokens are not stored.
- `usage-cache.json` — last known usage per provider and account, so the widget has something to
  show after a restart or outage (shown like fresh usage: it is the latest the app has seen).
- `window-pos.json` — where the widget was last placed.

OpenCode API keys live in **Windows Credential Manager** (service `UsageDeck`), never in a file.
The settings window only ever shows a hint (`sk-…a1b2`). All file writes are atomic (temp file +
rename), so a crash can't corrupt settings.

The installed app starts at login by default; turn it off with **Start with Windows** in
Settings → General (applied at once). Copies run from a cargo `target` folder never register
autostart.

Logs (about 1 MB per file, 5 files kept) are in `%LOCALAPPDATA%\com.usagedeck.app\logs\`; the
tray menu's **Open log folder** opens it in Explorer. A crash leaves a `crash-<time>.txt` report
there (panic message, location, backtrace, version; newest 10 kept).
A Cursor call that fails is logged once per endpoint and status, for example
`cursor https://api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage: HTTP 401
(unauthenticated: …); the stored access token expired at 2026-11-10 08:00:00 UTC`. An expired token
means Cursor has not written a renewed one yet; a token still valid means the server refused the
request itself.

Uninstalling removes the start-at-login entry. Tick **Delete the application data** in the
uninstaller to also remove settings, logs and the stored OpenCode keys (details in SECURITY.md).

## Features

### Widget

- Compact panel with the active OpenCode account plus every enabled provider; right-click (or the
  tray menu) → _Expand_ / _Collapse_ for all OpenCode accounts, _Refresh_, _Settings_, _Check for
  updates_, _Quit_
- **Adaptive theme** — in `auto`, samples the screen around the widget and switches card style
  with hysteresis; `light` and `dark` are fixed (no sampling)
- **Change detection** — when an account's usage rises, a ▲ shows for 20 s and the most recently
  used account floats to the top
- **Reset countdowns** (`5d 7h 45m`) and **reset memory** — cached usage looks exactly like fresh
  usage, with no "cached" or "updated … ago" note; a window whose stored reset time has passed
  shows `--` until fresh data arrives
- **States, consistently on every card** — a card shows text only when you must act: loading,
  unavailable, `re-auth` (Cursor: an `Open Cursor` button, numbers in gray), `key rejected` (OpenCode 401 / 403), `not signed in`, exhausted (red).
  Passing failures (rate limit, network, service error, busy, changed reply format) show nothing
  on the card; they stay in the log and `--dump`
- **Hide until reset** — a used-up (red) card has a small eye-slash button next to its name. Click
  it to hide the card until it can be used again: the latest reset among its full windows (5-hour
  and weekly both full waits for the weekly reset). The card comes back on its own at that time,
  or sooner if usage drops. A hidden card leaves nothing on the widget; right-click → **Show
  hidden** brings it back early
- **Unavailable banner** — "Usage unavailable · updated 3m ago" with Retry
- **Update banner** — "Update X is ready" with Restart and Later, and the progress and result of a
  check you started (see [Updates](#updates))
- **Copy key** — hover the OpenCode icon on a card (a copy glyph appears) and click; the key is
  copied by the Rust side and never passes through the page
- Settings changes apply live; right-click or **Shift+F10** opens the menu

### Settings (right-click widget → Settings)

- A normal window with a title bar and a taskbar button (960x680, resizable). It reopens where you
  left it, kept on a visible monitor, and closes only when asked: the X, Alt+F4, Ctrl+W or Esc
- General: start with Windows, hide during full-screen apps, DeepSeek peak-hour warning, and
  Updates (installed version, last check, Check for updates, Restart to update)
- Appearance: settings theme, widget theme, show provider names
- Widget: provider order (drag the grip, or Move up / Move down) and a "Show on widget" switch per
  provider
- Accounts: one page per provider with the widget's own cards, 2 per row. Each card header has a
  "Show on widget" switch and a "⋯" menu (also right-click, Shift+F10 or the Menu key): Rename,
  Refresh now and, for OpenCode, Copy API key, Replace API key and Remove account. OpenCode
  accounts are added with an API key; Cursor, Grok Bot, Codex and Claude read that app's own
  sign-in and only the signed-in account shows on the widget
- Claude page: a "Spend & pace" section with the API-equivalent spend of Claude Code on this PC
  (last hour, 5 h, today, 7 days, $/h), a 24-hour bar chart, spend per model, each window's
  speed, even pace and projected limit time, and what 1% of the weekly limit is worth in dollars.
  The widget's Claude card shows the spend rate after its name (`$71/h`); its meter colours follow
  the percentage only (amber from 80%, red from 100%), and pace stays in the meter tooltip
- About: version, log folder and the repository
- Fully keyboard accessible: modal dialogs trap focus and Esc closes the topmost one first; menus
  use the arrow keys, Home and End; forced-colors (high contrast) is supported
- `settings.html?page=<general|appearance|widget|claude|codex|cursor|grok|opencode|about>` opens a
  page directly (browser preview: add `&panel=light` or `&panel=dark`)
- Browser preview of spend and pace: add `?pace=<lasts|limit|weekly|measuring|backfill|unpriced|fail>`
  to `widget.html` or `settings.html`
- Browser preview of the updater: add `?update=<idle|never|checking|downloading|upToDate|failed|ready>`
  for the starting state and `&updateResult=<ready|upToDate|failed>` for what Check for updates
  finds

### Ranking rules

1. Accounts at 100% on **monthly or weekly** usage sink to the bottom (red glow)
2. Among exhausted accounts, the one resetting soonest is ranked first
3. Otherwise, the most recently used account floats to the top (green glow)

## Command-line modes

Headless: each prints a short report and exits.

- `usagedeck.exe --import-old-data <dir>`: one-time copy of the old Next.js or Electron app's
  `data/` folder (the installed 1.0.x app kept it in `%APPDATA%\UsageDeck\data`).
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
`cargo clippy -D warnings`, `cargo test` and `cargo audit` for both crates (`src-tauri/` and
`installer/`), and `npm audit` on every push and pull request.

To see the installer window with a real installer inside, build the app's NSIS installer first,
then build the window around it:

```bash
npm run tauri build
USAGEDECK_NSIS_PATH="$PWD/src-tauri/target/release/bundle/nsis/UsageDeck_<version>_x64-setup.exe" \
  cargo build --release --manifest-path installer/Cargo.toml
```

A plain `cargo build` in `installer/` embeds nothing, and its Install button reports that. A
release build fails without `USAGEDECK_NSIS_PATH`, and also when the file is not that version's
installer.

## Releasing

1. Raise the version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`,
   `installer/Cargo.toml` and `package.json`, then run `npm install` and `cargo check` (in
   `src-tauri/` and in `installer/`) so the lock files follow. `tests/version.test.ts` fails if
   any of the eight disagree.
2. Merge to `main`.

The **Release** workflow (`.github/workflows/release.yml`) runs when `tauri.conf.json` changes on
`main`. If `v<version>` has no release yet, it builds the NSIS installer and publishes release
`v<version>` with it as `UsageDeck_<version>_x64-setup.exe`. It then builds the installer window
(`installer/`) with that NSIS installer inside and uploads it as `UsageDeck-Setup.exe`. The
permanent download link and the in-app updater both use `UsageDeck-Setup.exe`.

The updater in installed copies runs `UsageDeck-Setup.exe /S /R /UPDATE` (1.4.0 and later) or
`/P /R /UPDATE` (1.1.0 to 1.3.1). Started with any arguments, the installer window shows nothing
and runs the NSIS installer inside with the same arguments, with one change: when `/UPDATE` is
present, `/P` (passive mode, which shows a progress window) becomes `/S`, so updates from older
copies install silently too. Keep that behaviour (`installer/src/decide.rs`, `launch` and
`nsis_args`) for as long as those copies exist.

A version whose release already has `UsageDeck-Setup.exe` is skipped, so other edits to
`tauri.conf.json` publish nothing. A release missing it, from a run that failed after publishing,
is built again. The workflow can also be started by hand from the Actions tab.

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
installer/            # installer window (UsageDeck-Setup.exe) around the NSIS installer
tests/                # frontend unit tests (Vitest)
widget.html           # widget page entry
settings.html         # settings page entry
docs/plans/           # design and build notes
```
