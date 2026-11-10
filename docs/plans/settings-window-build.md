# Settings window build notes (branch settings-window)

Built against `docs/plans/settings-window-spec.md`.

## Departures from the spec

- Backdrop is solid, not Mica: Tauri's Mica needs a transparent window and webview, and WebView2
  loses ClearType on a transparent surface (`src-tauri/src/settings_window.rs:1-7`).
- App theme offers Light / Dark only: "Use system setting" needs a new `panelTheme` value in the
  settings schema (`store.rs`, `settings-schema.ts`, `types.ts`), outside this package.
- Ctrl+, from the widget is not wired: it belongs to `src/widget/App.tsx` (widget builder).
- About has no licences link: the repo has no licence file to point at.
- Widget card text is drawn at 1.2x zoom in settings (`settings.css` `.ud-cards`) so 10 px card
  text reads at 12 px. Set it back to 1 if the widget typography reaches 12 px.
- "Refresh now" emits the widget's existing refresh event (all accounts), then reloads this window's
  data after 5 s; it does not refresh one account alone.

## Review map

- `src-tauri/src/settings_window.rs:33-111` `Rect`, `clamp_to_work_areas`, `centred_in`,
  `parse_state`, `next_state`: pure placement rules, unit-tested at the end of the file.
- `src-tauri/src/settings_window.rs:145-183` `save_now`, `schedule_save`: save outer rect and
  maximised flag to `settings-window.json`, debounced 400 ms on move / resize, immediately on close.
- `src-tauri/src/settings_window.rs:185-231` `place`: restore the clamped saved rect, else centre
  960x680 logical on the widget's monitor.
- `src-tauri/src/settings_window.rs:233-292` `create`, `build`: decorated, resizable, taskbar
  window with the app icon, theme-coloured background; X / Alt+F4 save, then emit
  `widget-settings-changed`.
- `src-tauri/src/settings_window.rs:294-335` `show_settings` (focus if open), `hide_settings`
  (page close: save, destroy, emit), `open_about_link` (fixed "logs" / "repo" targets).
- `src-tauri/src/lib.rs` old overlay builder removed; module, imports and command registered.
- `src-tauri/capabilities/default.json` adds `core:window:allow-set-theme` (caption follows theme).
- `src/widget/_components/Card.tsx:15,77-79` `CardExtras.headerActions` replaces the unused
  `footer`, rendered at the right end of the header row.
- `src/ui/Menu.tsx:21-122` `Menu`: roving focus, Esc / Tab / outside click / blur close, focus
  returns to the opener; portal into `[data-menu-root]`.
- `src/ui/menu-focus.ts` `menuFocusIndex`, `isContextMenuKey`, `menuPosition`: pure, tested.
- `src/ui/Dialog.tsx:9-90` `Dialog`: 440 px native modal, no outside-click close, optional form.
- `src/ui/Toggle.tsx` `Toggle`: WinUI toggle switch (role="switch").
- `src/settings/App.tsx:37-150` shell: theme vars, caption theme, Esc / Ctrl+W close with reasons,
  page routing.
- `src/settings/_lib/pages.ts` `pageFromSearch`, `searchWithPage`, `moveItem`: routing, tested.
- `src/settings/_lib/hooks.ts:92-145` `useSummaries` gains `reloadAll`; `refreshWidget`.
- `src/settings/_components/Nav.tsx` left navigation.
- `src/settings/_components/fields.tsx` `Page`, `Section`, `SettingRow`, button classes.
- `src/settings/_components/SettingsPages.tsx` General, Appearance, Widget (drag + Move up/down,
  per-provider switch), About.
- `src/settings/_components/CardGrid.tsx:22-106` `CardGrid` (container query 2 / 1 columns),
  `AccountCell` (switch, "⋯", right-click / Shift+F10 / Menu key).
- `src/settings/_components/ProviderPanel.tsx`, `OpenCodeAccounts.tsx`: account pages and menus.
- `src/settings/_components/dialogs.tsx` Rename, Remove (Cancel focused), Add / Replace key.
- Deleted: `SettingsPanel.tsx`, `GeneralTab.tsx`, `src/ui/Tabs.tsx`, `src/ui/close-reason.ts`
  (+ test), `CardFooter`.
