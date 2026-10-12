# Provider Usage Monitor

An always-on-top floating desktop widget that tracks AI subscription usage across **five providers**:

| Provider | What's shown | Data source |
| --- | --- | --- |
| **OpenCode Go** | Rolling / Weekly / Monthly usage per account, reset countdowns, DeepSeek peak-hour timer | `opencode.ai/zen/go/v1/usage` (API key) |
| **Cursor** | First Party & API usage, plan badge, billing-cycle reset | Cursor's local auth token → `api2.cursor.sh` |
| **Grok Bot** | Weekly usage %, plan badge, reset countdown | Cursor sand usage endpoint (`grok-bot` client) |
| **Codex** | Usage %, plan badge (Free/Plus/Pro), reset countdown | Codex CLI auth → ChatGPT `wham/usage` |
| **Claude** | Subscription status badge (Free / plan) | Claude Code local credentials |

No accounts or passwords leave your machine except the API calls each provider's own app makes.

---

## Architecture

```
Next.js app (server)                 Electron widget (desktop/)
├── /widget          floating panel  ├── frameless, always-on-top
├── /settings        settings modal  ├── follows your theme (adaptive background)
├── /                → /widget       ├── draggable, expandable
└── /api/*           usage endpoints └── right-click menu (expand, refresh, settings, quit)
```

- The widget and settings modal read from the same local API routes.
- The server binds to **127.0.0.1:3100** — deliberately not 3000 so it can coexist with other local dev servers.
- Every provider integration lives in `lib/`: `opencode.ts`, `cursor.ts`, `codex.ts`, `claude.ts`, `settings.ts`.

## Requirements

- **Node.js 20+** (built and tested on Node 24)
- Windows (the widget uses Electron; the server runs anywhere)
- Optional: Cursor, Codex CLI, Claude Code installed locally — each provider is auto-detected

## Setup

```bash
# 1. Install dependencies
npm install

# 2. (Widget only) install Electron deps
cd desktop && npm install && cd ..

# 3. Build and run
npm run build
npm run start        # serves the app on http://127.0.0.1:3100
```

### Desktop shortcut

`desktop/widget-launcher.vbs` starts the floating widget from the `desktop/` folder (hidden). Create a shortcut to it (via `wscript.exe`) for one-click launch.

## Accounts & Secrets

Provider credentials **never live in this repo**:

- `data/accounts.json` — OpenCode API accounts (`email` + `sk-...` key). **Gitignored.**
- `data/settings.json` — toggles, display names, disabled accounts. **Gitignored.**
- `data/.api-secret` — local token for key/account writes. **Gitignored.** Electron injects it automatically.
- `data/accounts.example.json` — committed template showing the format

Cursor / Codex / Claude read their tokens directly from each app's local credential store at request time; nothing is copied into this project.

## Features

### Widget
- Compact panel showing the active account + all providers; click expand for the full list
- **Adaptive theme** — samples screen brightness every 3 seconds and fades between dark/light card styles
- **Change detection** — when an account's usage percent moves, a green ▲ appears for 20s and the most recently used account floats to the top
- **Reset countdowns** — each bar shows time until its window resets (`5d 7h 45m`); monthly resets always visible
- **Reset memory** — last known usage and reset times persist across restarts; cached cards are marked `cached`, and a window whose stored reset time has passed shows as unknown until fresh data arrives
- **Status badges** — `Go` (OpenCode), plan names for Cursor/Codex, `Free` for Claude
- **Copy key** — click the OpenCode icon on a card to copy that account's API key
- Right-click menu: expand/collapse, refresh, settings, quit

### Settings (right-click widget → Settings)
- Per-provider tabs with on/off toggles, nickname editing, and account details
- **Show provider names** master toggle — display `OpenCode / Cursor / Grok Bot / Codex / Claude` instead of nicknames on all cards
- **DeepSeek Peak Hour Warning** toggle — show or hide the peak-hour countdown on the active OpenCode card
- Add / remove / disable OpenCode accounts without touching files

### Ranking rules
1. Accounts at 100% on **monthly or weekly** usage sink to the bottom (red glow)
2. Among exhausted accounts, the one resetting soonest is ranked first
3. Otherwise, the most recently used account floats to the top (green glow)

## Scripts

| Command | Description |
| --- | --- |
| `npm run dev` | Development server on http://127.0.0.1:3100 |
| `npm run build` | Production build |
| `npm run start` | Serve the production build on http://127.0.0.1:3100 |

## Project layout

```
app/
├── api/            # usage endpoints (opencode, cursor, codex, claude, settings, accounts, key)
├── components/     # SettingsModal, toast system
├── page.tsx        # redirects / → /widget
├── settings/       # settings modal overlay
└── widget/         # floating panel UI
desktop/            # Electron wrapper (main.js, preload, launchers)
lib/                # provider integrations + settings store
data/               # local secrets (gitignored)
```
