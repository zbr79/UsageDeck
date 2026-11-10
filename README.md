# Provider Usage Monitor

A local desktop widget that shows how much of an AI subscription is left. It covers OpenCode Go, Cursor, Grok, Codex, and Claude.

Keys and logins stay on this machine. OpenCode uses an API key you add in settings. The other providers are read from the apps already signed in on the computer.

## What it shows

- OpenCode Go: rolling, weekly, and monthly usage, and the time until each window resets
- Cursor: first-party and API usage, the plan name, and the billing-cycle reset
- Grok: weekly usage and the time until reset
- Codex: usage, plan, and the time until reset
- Claude: signed-in state and plan

The panel stays compact until you expand it. Disabled accounts are hidden. An account that has used up its weekly or monthly allowance drops to the bottom, soonest reset first. Otherwise the account that changed most recently stays on top.

Right-click the window, or press Shift+F10, to expand, refresh, open settings, check for updates, or quit. Settings can turn providers on or off, rename accounts, show or hide the DeepSeek peak-hour warning, and add or remove OpenCode accounts.

Keyboard shortcuts on the widget:

- `E` expands or collapses the panel
- `R` refreshes usage
- `Ctrl+,` opens settings
- `Ctrl+Q` quits
- `Shift+F10` opens the menu

## How it runs

A Next.js server and a small Electron window. The server listens on `127.0.0.1:3100`, so it can run next to other local apps. The window is frameless, always on top, and draggable. Both the widget and the settings screen use the same local API routes. Provider clients live in `lib/`.

## Requirements

- Node.js 20 or newer
- Windows for the Electron window. The server can run on its own anywhere.
- Cursor, the Codex CLI, or Claude Code if you want those rows. A missing app is skipped.

## Install

1. Download `UsageDeck-Setup-x.y.z.exe` from the [latest release](https://github.com/zbr79/UsageDeck/releases/latest) and run it.
2. Launch UsageDeck from the desktop shortcut or Start menu.

The installed app checks for updates 10 seconds after launch and every 6 hours, downloads them in the background, and asks before restarting to install. To check on demand, right-click the widget and choose **Check for Updates…**.

The installer is unsigned, so Windows SmartScreen shows a warning. Choose **More info**, then **Run anyway**.

Installed data lives in `%APPDATA%/UsageDeck/data`. If you are migrating from a source checkout, copy your old `data/` files (`accounts.json`, `settings.json`, `.api-secret`) there.

## Develop from source

```bash
npm install
cd desktop && npm install && cd ..
npm run build
npm run start
```

The site is at http://127.0.0.1:3100. From `desktop/`, `npm start` opens the widget. If port 3100 is already taken by another program, the window stays open and says so instead of exiting. A desktop shortcut can point at `desktop/widget-launcher.vbs`. From source, data stays in the repo's `data/` folder.

To build the installer locally, run `npm run bundle` at the root, then `npm run dist` in `desktop/`. The installer lands in `desktop/dist/`.

## Releasing

Bump `version` in `desktop/package.json` and merge the change to `main`.

The `Release` workflow then builds the installer on Windows and publishes it to GitHub Releases as `vX.Y.Z`, where installed apps pick it up. A push that leaves the version unchanged, or names a version that is already released, builds nothing. To rerun a release by hand, use **Run workflow** on the workflow's Actions page.

## Local data

The installed app keeps its data in `%APPDATA%/UsageDeck/data`. A source checkout uses the repo's `data/` folder, and the server reads `USAGE_DATA_DIR` to point somewhere else. Below, `data/` means that folder.

In the repo, nothing under `data/` is committed except the example file.

- `data/accounts.json` stores OpenCode emails and API keys. Keys are encrypted with AES-256-GCM. The key comes from `data/.api-secret`. An older plaintext file is encrypted the next time it is read. See `data/accounts.example.json` for the shape before encryption.
- `data/settings.json` stores toggles, names, and disabled accounts.
- `data/.api-secret` is a local token for account and settings writes, and for encrypting stored API keys. The desktop app sends it for you.
- Crash logs: the Electron window writes `logs\main.log` under its app data folder (`%APPDATA%\UsageDeck` installed, `%APPDATA%\provider-usage-widget` from source). The server writes `data/logs/server.log`. The widget menu has **Open log folder**.

## Scripts

- `npm run dev` starts the development server on http://127.0.0.1:3100
- `npm run build` builds for production
- `npm run start` serves that build
- `npm run bundle` builds for production, then assembles the server bundle in `desktop/server/` for the installer
- `npm test` runs the unit tests
- `npm run lint` runs ESLint
- `npm run typecheck` checks TypeScript

The tests cover ranking, reset text, settings defaults, the local write token, encrypted account keys, provider response parsers, and account validation. They do not call provider APIs.

## Layout

```
app/        pages and API routes
desktop/    Electron window
lib/        provider clients, ranking, and settings
data/       local accounts and settings, not committed
```
