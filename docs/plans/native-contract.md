# Native app contract (Tauri rebuild)

This is the binding agreement between the builders. Base commit: `f1679d6` on `claudefix`. The old
app (`app/`, `lib/`, `desktop/`) stays untouched until the final step.

## Ownership

| Path                                                                         | Owner                                       |
| ---------------------------------------------------------------------------- | ------------------------------------------- |
| `native/package.json`, `native/vite.config.ts`, `native/tsconfig*.json`      | Windows builder (B)                         |
| `native/widget.html`, `native/settings.html`, `native/src/**`                | Windows builder (B)                         |
| `native/.gitignore` (`node_modules`, `dist`)                                 | Windows builder (B)                         |
| `native/src-tauri/**`, including its `.gitignore` (`/target/`, `/gen/schemas`) | Shell builder (A); later the provider builders |
| Root `.gitignore`: only the line that ignores all of `data/`                 | Shell builder (A)                           |

Nobody else edits these paths.

## Frontend

- Vite + React 19 + TypeScript + Tailwind 4 (`@tailwindcss/vite`).
- Two pages:
  - `native/widget.html`, served from `src/widget/main.tsx`;
  - `native/settings.html`, served from `src/settings/main.tsx`.
- The dev server runs on port **1420** (`strictPort`). The build goes to `native/dist`.
- Scripts in `native/package.json`: `dev`, `build` (`tsc -b && vite build`), `test` (`vitest run`),
  `tauri`.
- `@tauri-apps/cli` and `@tauri-apps/api` are listed there.

## Windows (`native/src-tauri/tauri.conf.json`)

- **Identifier:** `com.usagedeck.app`. Product name: `UsageDeck`.
- **`widget` window:**
  - url `widget.html`;
  - width 238, height 150;
  - `decorations: false`, `transparent: true`, `alwaysOnTop: true`, `skipTaskbar: true`,
    `resizable: false`, `shadow: false`, visible.
  - Its position is saved to `window-pos.json` (debounced) and restored at start, kept on a visible
    monitor.
- **`settings` window:**
  - url `settings.html`, created hidden;
  - `decorations: false`, `transparent: true`, `alwaysOnTop: true`, `skipTaskbar: true`.
  - `open_settings` sizes it to the work area of the widget's monitor, then shows and focuses it.
- **Tray icon:** menu items Expand/Collapse, Refresh, Settings, Quit. Left click shows the widget.
- **Plugins:** `single-instance` (a second launch focuses the widget), `autostart` (enabled), and
  `log` (writes to the app log directory).

## Commands (Rust `#[tauri::command]`, JS `invoke`)

**`api(method, path, body) -> { status: number, body: any }`**

- `method` is `"GET" | "POST" | "DELETE"`. `body` is JSON or null.
- The paths and JSON bodies are **identical** to the old Next.js routes:

  | Path                         | Old route                     |
  | ---------------------------- | ----------------------------- |
  | `GET /api/usage`             | `app/api/usage/route.ts`      |
  | `GET /api/cursor`            | `app/api/cursor/route.ts`     |
  | `GET /api/grok`              | `app/api/grok/route.ts`       |
  | `GET /api/codex`             | `app/api/codex/route.ts`      |
  | `GET /api/claude`            | `app/api/claude/route.ts`     |
  | `GET /api/settings`, `POST /api/settings` (partial patch) | `app/api/settings/route.ts` |
  | `GET /api/accounts`, `POST /api/accounts`, `DELETE /api/accounts` | `app/api/accounts/route.ts` |
  | `GET /api/key?email=`        | `app/api/key/route.ts`        |

- Errors return the old status and `{ "error": "..." }`. An unknown path returns 404.

**Other commands**

- `widget_resize(height: f64)`: width stays 238, height is clamped to 40 .. (work area height - 8).
- `open_settings()`, `close_settings()`. Close hides the window and emits
  `widget-settings-changed`.
- `quit_app()`.

**Dragging:** the frontend calls `getCurrentWindow().startDragging()`. Rust saves the position on
`Moved`.

## Events (Rust to the `widget` window)

| Event                     | Payload                 | From                                                   |
| ------------------------- | ----------------------- | ------------------------------------------------------ |
| `widget-bg`               | `f64` brightness 0..1   | Screen sampler, every 3 s while `themeMode` is `auto` (wave 2) |
| `widget-toggle-expand`    | none                    | Tray menu                                              |
| `widget-refresh`          | none                    | Tray menu                                              |
| `widget-settings-changed` | none                    | Settings closed, or settings / accounts saved          |

The old `widget-server-status` event is gone because there is no server. The frontend drops the
offline banner's server-restart states.

## Storage (Tauri `app_data_dir`)

- `settings.json`: same shape as `Settings` in `lib/types.ts`. Defaults and patch rules are ported
  from `lib/settings-schema.ts` and `lib/settings.ts`.
- `accounts.json`: OpenCode accounts, holding **emails only**. Keys live in Windows Credential
  Manager (`keyring` crate, service `UsageDeck`, user = lowercase email).
- `usage-cache.json`, `seen-accounts.json`, `window-pos.json`.
- All writes are atomic (temp file + rename) and serialised.

## Provider framework (Rust)

- **JSON shapes:** the types in `lib/types.ts` are the contract. Use serde with camelCase fields.
- **Cache** (`lib/provider-cache.ts`):
  - a TTL per provider;
  - one request in flight;
  - a last good value that a failed **or invalid** reply never overwrites.
- **Tracked accounts:** port `lib/tracked-accounts.ts`, `lib/seen-accounts.ts` and
  `lib/account-key.ts`. The seen list keeps the **newest** 50 entries.
- **Validation:** a provider reply that does not deserialise into its typed struct is a failure. It
  never produces an all-null success.

## Testing (light, by request)

- Rust: `cargo test` for the store, the settings patch, the cache's last-good rule, and one recorded
  reply per provider.
- Frontend: `npm run build` plus the moved pure-logic unit tests.
- Final check: the new app shows the same numbers as the old widget.
