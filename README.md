# Provider Usage Monitor

An always-on-top floating desktop widget that tracks AI subscription usage across **five providers**:

| Provider        | What's shown                                                                            | Data source                                                                  |
| --------------- | --------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| **OpenCode Go** | Rolling / Weekly / Monthly usage per account, reset countdowns, DeepSeek peak-hour timer | `opencode.ai/zen/go/v1/usage` (API key you add in Settings)                  |
| **Cursor**      | First Party & API usage, plan badge, billing-cycle reset                                | Cursor's local sign-in → `api2.cursor.sh`                                     |
| **Grok Bot**    | Weekly usage %, plan badge, reset countdown                                             | Grok Bot's own sign-in (falls back to Cursor's) → `api2.cursor.sh` sand usage |
| **Codex**       | Usage %, plan badge (Free/Plus/Pro), reset credits, reset countdown                     | Codex CLI sign-in → ChatGPT `wham/usage`                                     |
| **Claude**      | 5-hour and weekly usage, plan badge, reset countdowns                                   | Claude desktop / Claude Code sign-in → Anthropic OAuth usage                 |

Other apps' sign-ins are **read, never written**: the monitor never refreshes or rewrites another
app's tokens (doing so rotates their refresh tokens and logs them out). If a sign-in has expired, the
card shows the last known numbers with a `re-auth` hint — open that app once to refresh it. See
[SECURITY.md](SECURITY.md) for exactly what is read, stored and sent.

---

## Architecture

```
Next.js app (server, 127.0.0.1:3100)        Electron widget (desktop/)
├── proxy.ts   loopback-Host + secret gate   ├── frameless, always-on-top, single instance
├── /widget    floating panel                ├── sandboxed renderers, navigation locked to the app
├── /settings  settings overlay              ├── adaptive theme (samples the screen around it)
├── /          → /widget                     ├── draggable, expandable
└── /api/*     usage + settings endpoints    └── right-click menu (expand, refresh, settings, quit)
```

- Every `/api` request must carry a shared secret (`data/.api-secret`) that the Electron preload
  injects, and only loopback `Host` headers are served. Opening the pages in a normal browser shows
  them, but the API refuses to answer.
- Providers live in `lib/` and share one caching policy (`lib/provider-cache.ts`): short TTL,
  single in-flight request, and a persisted last-known value used when a provider is unreachable.
- Each widget card loads independently: one slow or failing provider never blanks the others.
- The server binds to **127.0.0.1:3100** — deliberately not 3000 so it can coexist with other dev
  servers.

## Requirements

- **Windows** (credential stores and the widget are Windows-specific)
- **Node.js 22.13+** (needs the built-in `node:sqlite` to read Cursor's sign-in; tested on Node 24)
- Optional: Cursor, Grok Bot, Codex CLI, Claude desktop / Claude Code — each is auto-detected

## Setup

```bash
npm install
```

```bash
cd desktop && npm install && cd ..
```

```bash
npm run build
```

Then start the widget with `desktop/widget-launcher.vbs` (create a shortcut to it via `wscript.exe`
for one-click launch). The widget starts the server itself if it isn't already running. If that
server exits, the window stays open and restarts it with backoff. If the server can't start within
60 seconds it shows an error telling you to run `npm run build`. The desktop log is
`%APPDATA%\provider-usage-widget\logs\main.log`.

## Your data

Everything the app stores lives in `data/` (gitignored; override with `USAGE_DATA_DIR`):

- `accounts.json` — OpenCode accounts. Keys are encrypted at rest (`enc:v1:`, AES-256-GCM) with
  `data/.api-secret`. A plaintext file is encrypted the next time it is read. The settings panel
  only ever shows a hint (`sk-…a1b2`); the full key is only sent to the widget's copy-key button.
- `settings.json` — toggles, nicknames, hidden accounts.
- `seen-accounts.json` — emails of Cursor, Grok Bot, Codex, and Claude accounts already seen, so
  switching accounts in those apps does not drop the earlier one. Tokens are not stored.
- `usage-cache.json` — last known usage per provider and account, so the widget has something to
  show after a restart or outage (marked `cached`).
- `.api-secret` — the shared secret between the widget and the server.
- `accounts.example.json` — committed template showing the format.

All writes are atomic (temp file + rename) and serialised, so a crash or two quick saves can't
corrupt or lose settings.

## Features

### Widget

- Compact panel with the active OpenCode account plus every enabled provider; right-click →
  _Expand_ or _Collapse_ for all OpenCode accounts
- **Adaptive theme** — in `auto`, samples the screen around the widget every 3 s and switches card
  style with hysteresis; `light` and `dark` are fixed (no sampling)
- **Change detection** — when an account's usage rises, a ▲ shows for 20 s and the most recently
  used account floats to the top
- **Reset countdowns** (`5d 7h 45m`) and **reset memory** — cached cards are marked `cached`; a
  window whose stored reset time has passed shows `--` until fresh data arrives
- **States, consistently on every card** — loading, unavailable, `cached`, `re-auth`, exhausted (red)
- **Offline banner** — if the local server can't be reached: "Server unreachable · updated 3m ago"
  with Retry
- **Copy key** — hover the OpenCode icon on a card (a copy glyph appears) and click
- Settings changes apply live; right-click menu or **Shift+F10** opens the menu

### Settings (right-click widget → Settings)

- General: panel theme, widget theme, show provider names, DeepSeek peak-hour warning
- Accounts: one tab per provider. Drag a provider tab sideways to change its order on the widget
- Every provider can be hidden as a whole. Each account has its own nickname and can be hidden on its own. OpenCode accounts are added with an API key and can all appear on the widget. Cursor, Grok Bot, Codex, and Claude only show the account that app is signed into; other sign-ins stay in Settings so you can nickname or hide them before switching
- Fully keyboard accessible: real dialogs (focus trapped, Escape closes the topmost), arrow-key tabs,
  switches and radio groups announced to screen readers

### Ranking rules

1. Accounts at 100% on **monthly or weekly** usage sink to the bottom (red glow)
2. Among exhausted accounts, the one resetting soonest is ranked first
3. Otherwise, the most recently used account floats to the top (green glow)

## Development

| Command                | Description                                                  |
| ---------------------- | ------------------------------------------------------------ |
| `npm run dev`          | Development server on http://127.0.0.1:3100                  |
| `npm run build`        | Production build (what the widget serves)                    |
| `npm run start`        | Serve the production build on http://127.0.0.1:3100          |
| `npm run check`        | Type-check, lint, format check and unit tests                |
| `npm test`             | Unit tests (Vitest)                                          |
| `npm run test:e2e`     | Browser tests (Playwright + Edge, axe accessibility scans)   |
| `npm run format`       | Format with Prettier                                         |

The e2e suite builds into `.next-e2e` and serves it on port 3199, so it never disturbs a running
widget. It mocks the API, stubs the Electron bridge and freezes the clock; screenshot baselines live
next to the specs (`*-snapshots/`) — update them with `npx playwright test --update-snapshots=all`
after intentional UI changes.

CI (GitHub Actions, Windows) runs all of the above plus `npm audit` on every push and pull request.

## Project layout

```
app/
├── _ui/            # design tokens (palette.ts) + accessible primitives (Dialog, Tabs, Switch…)
├── api/            # usage, settings, accounts and key endpoints
├── components/     # toast system
├── settings/       # settings overlay (_components/, _lib/hooks.ts)
└── widget/         # floating panel (_components/ cards, _lib/ data + theme)
desktop/            # Electron shell (main.js, preloads, launcher)
lib/                # providers, caching, settings rules, formatting/ranking (pure, unit-tested)
proxy.ts            # loopback Host + secret gate for every request
tests/              # unit/ (Vitest) and e2e/ (Playwright)
data/               # your local data (gitignored)
```
