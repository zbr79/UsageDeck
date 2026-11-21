# UsageDeck

A **100% pure-Rust** Windows 11/10 taskbar monitor that tracks AI subscription usage across **five
providers**. It renders a compact right-side taskbar strip, an anchored hover/click flyout, and a
native Windows Settings dialog directly with Win32 + Direct2D/DirectWrite — **zero WebView2, zero
Node.js, zero Tauri, zero Electron**.

| Provider        | What's shown                                                                                                  | Data source                                                                                                                                    |
| --------------- | ------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| **OpenCode Go** | Rolling / Weekly / Monthly usage per account, reset countdowns, DeepSeek peak-hour timer                      | `opencode.ai/zen/go/v1/usage` (API key added in Settings)                                                                                      |
| **Cursor**      | First Party & API usage, plan badge, billing-cycle reset, API dollars and top models                          | Cursor local sign-in → `api2.cursor.sh` (fallback `cursor.com/api/usage-summary`); spend from `cursor.com/api/dashboard/get-filtered-usage-events` |
| **Grok Bot**    | Weekly usage %, plan badge, reset countdown                                                                   | Grok Bot local sign-in (falls back to Cursor's) → `api2.cursor.sh`                                                                             |
| **Codex**       | 5-hour & weekly usage %, plan badge (Free/Plus/Pro), reset credits, reset countdown                           | Codex CLI sign-in → ChatGPT `wham/usage`                                                                                                       |
| **Claude**      | 5-hour, weekly, and model-specific weekly (Fable) limits, banked resets (`↺ N`), spend rate ($/h) and pace    | Claude desktop / Claude Code sign-in → Anthropic OAuth usage; dollars from local Claude Code logs (usage fields only)                          |

Other apps' sign-ins are **read, never written**: UsageDeck never refreshes or rotates another
app's tokens. When a sign-in expires, the provider shows its last known numbers with a `re-auth`
hint (or an **Open Cursor** action for Cursor). See [SECURITY.md](SECURITY.md) for everything
UsageDeck reads, stores, and sends.

---

## Install

Download and run the per-user installer:
**<https://github.com/zbr79/UsageDeck/releases/latest/download/UsageDeck-Setup.exe>**

- One small native Windows window: click **Install** (or **Update** / **Reinstall** on an existing
  install — your settings, accounts, and keys are preserved).
- Installs per-user under `%LOCALAPPDATA%\UsageDeck\usagedeck.exe` with no admin rights required.
- Not code-signed, so Windows SmartScreen may show "Windows protected your PC" — choose
  **More info → Run anyway**.
- Each release also attaches the standalone portable executable (`usagedeck-v<version>-x64.exe`).

### Scripted & silent installer flags

| Flags                  | Behaviour                                                                 |
| ---------------------- | ------------------------------------------------------------------------- |
| `/S`                   | Silent install to `%LOCALAPPDATA%\UsageDeck` (no window)                  |
| `/P /UPDATE`, `/S /UPDATE` | Silent in-place upgrade (also compatible with UsageDeck v1.x updaters) |
| `/UNINSTALL`           | Interactive uninstaller                                                   |
| `/UNINSTALL /S`        | Silent uninstall (removes `usagedeck.exe`, shortcuts, and registry entry) |

### Automatic updates

UsageDeck checks GitHub Releases shortly after startup and periodically in the background. When a
newer version is published, it downloads `UsageDeck-Setup.exe`, verifies its SHA-256 digest against
the release metadata, and runs a silent in-place update automatically. Existing UsageDeck v1.x
installations also upgrade seamlessly to v2.0.0 through the same `UsageDeck-Setup.exe` release URL.

---

## UI & Features

- **Right-side taskbar strip** — sits cleanly beside the Windows 11/10 system tray, adapts to the
  Windows taskbar light/dark theme, supports per-monitor DPI scaling, and hides automatically during
  full-screen apps.
- **Anchored flyout** — hover or click the strip to inspect per-provider bars, reset countdowns,
  Claude Code spend/pace metrics, Cursor API dollar breakdowns, and quick actions (Refresh, Open
  Cursor, Copy OpenCode key, Hide until reset).
- **Native Settings dialog** — right-click the strip or tray icon → **Settings** to configure
  Start with Windows, provider visibility & order, per-bar toggles, OpenCode API keys, nicknames,
  hidden cards, theme mode, and diagnostics.
- **Change detection & ranking** — active accounts float to the top when usage rises, while
  exhausted accounts (100% weekly/monthly) sink to the bottom sorted by soonest reset.

---

## Data & Security

UsageDeck v2.0.0 keeps full compatibility with v1.x data directories:

- **Settings & caches**: `%APPDATA%\com.usagedeck.app` (`settings.json`, `accounts.json`,
  `seen-accounts.json`, `usage-cache.json`) — all writes are atomic (temp file + rename).
- **OpenCode API keys**: stored exclusively in **Windows Credential Manager** (service
  `UsageDeck`), never on disk.
- **Logs & crash reports**: `%LOCALAPPDATA%\com.usagedeck.app\logs\`.

---

## Command-line modes

```bat
usagedeck.exe --demo
usagedeck.exe --dump
usagedeck.exe --import-old-data "%APPDATA%\UsageDeck\data"
```

- `--demo` — launches the taskbar strip and flyout with realistic sample data across providers.
- `--dump` — polls all enabled providers once and prints a redacted JSON snapshot to stdout, then
  exits.
- `--import-old-data <dir>` — one-time migration from UsageDeck 1.0.x (`%APPDATA%\UsageDeck\data`),
  moving OpenCode keys into Windows Credential Manager without printing secrets.

---

## Architecture

```
crates/
├── usagedeck-core/       # Providers, store, pricing, Claude Code logs, history, diagnostics, updater
├── usagedeck-taskbar/    # usagedeck.exe: Win32 + Direct2D/DirectWrite taskbar strip, flyout, Settings, tray, CLI
└── usagedeck-installer/  # UsageDeck-Setup.exe: pure-Rust Win32 installer & uninstaller (embeds usagedeck.exe)
```

---

## Development

### Requirements

- **Windows 10/11** (x64)
- **Rust stable toolchain** (`rustup`) with the MSVC build tools — no Node.js, npm, or WebView2 SDK
  required.

### Commands

| Command                                                     | Description                                              |
| ----------------------------------------------------------- | -------------------------------------------------------- |
| `cargo run --bin usagedeck -- --demo`                       | Run the taskbar strip & flyout with demo data            |
| `cargo run --bin usagedeck -- --dump`                       | Poll providers once and print redacted JSON              |
| `cargo fmt --all -- --check`                                | Check Rust formatting across the workspace               |
| `cargo clippy --workspace --all-targets -- -D warnings`     | Lint all crates with warnings as errors                  |
| `cargo test --workspace`                                    | Run workspace unit and integration tests                 |
| `cargo build --release --bin usagedeck`                     | Build release `target/release/usagedeck.exe`             |

To build `UsageDeck-Setup.exe` with the release `usagedeck.exe` embedded (PowerShell):

```powershell
cargo build --release --bin usagedeck
$env:USAGEDECK_EXE_PATH = (Resolve-Path 'target\release\usagedeck.exe').Path
cargo build --release --bin UsageDeck-Setup
```

---

## Releasing

1. Bump `version` under `[workspace.package]` in root `Cargo.toml`.
2. Merge to `main`.

The **Release** workflow (`.github/workflows/release.yml`) triggers when `Cargo.toml` changes on
`main`. If tag `v<version>` does not yet have `UsageDeck-Setup.exe`, it builds release
`usagedeck.exe`, embeds it into `UsageDeck-Setup.exe`, and publishes GitHub release `v<version>`
with both `UsageDeck-Setup.exe` and `usagedeck-v<version>-x64.exe`.
