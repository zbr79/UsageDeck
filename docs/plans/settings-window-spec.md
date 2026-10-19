# Settings window spec (rework)

Date: 2026-10-18. Sources: a design research pass (PowerToys, Windows app-settings guidelines,
CommunityToolkit SettingsCard / SettingsExpander, Codex / ChatGPT, Raycast, CodexBar, 1Password,
VS Code) and the owner's decisions. Where they differ, the owner's decision wins.

## Owner decisions (locked)

- Settings is a **real window**: title bar, taskbar button, closes only when asked. It never closes
  on an outside click.
- Account usage is shown as the **widget's own cards, 2 per row**. There is no extra controls line
  under a card; the controls live in the card header.
- Cached numbers look like fresh ones. A text hint appears only when the user must act.

## Window

- A normal top-level Tauri window titled "UsageDeck Settings", with the app icon and its own taskbar
  button.
  - Native frame (decorations on), so Snap layouts, caption buttons and drag work. The caption
    follows the theme.
  - Not always-on-top. No transparent overlay. It does not close on blur.
  - Source: PowerToys (`MainWindow.xaml`), HIGH.
- Default size 960x680, minimum 640x480, resizable and maximizable.
  - Source: PowerToys has a 480x480 minimum, HIGH. 960x680 is our own choice.
- **Placement:** the first open is centred on the widget's monitor. Later opens restore the saved
  position and size, clamped to a visible monitor (PowerToys `WindowHelper`, HIGH). Save it to the
  app data folder, as `window-pos.json` already is; no new plugin.
- **Re-opening:** opening again focuses the existing window.
- **Keyboard:**
  - Esc closes the window, but only when no menu or dialog is open (those close first).
  - Ctrl+W and Alt+F4 close it.
  - Ctrl+, opens Settings from the widget. Source: Codex / ChatGPT and Raycast, HIGH.
- **Closing:** closes only the settings window; the widget and tray keep running.
- **Backdrop:** Mica where available, solid colour otherwise. Source: PowerToys, HIGH.

## Navigation (left pane, about 220-240 px)

- **General:** Start with Windows; DeepSeek peak-hours warning.
- **Appearance:**
  - App (panel) theme: Light / Dark / Use system setting;
  - Widget theme;
  - Show provider names.
- **Widget:** provider order (drag, plus Move up / Move down for keyboard users) and a per-provider
  "Show on widget" switch.
- **Accounts:** one sub-item per provider in widget order (Claude, Codex, Cursor, Grok Bot,
  OpenCode). Each opens that provider's page.
- **About:** app name, version and icon; links to the repo, the log folder and the licences.
- Labels are one word each (Microsoft guidance, HIGH).

## Page anatomy (WinUI SettingsCard, rebuilt in React + Tailwind)

- **Type and spacing:**
  - page title 28/36 semibold;
  - section header 14/20 semibold, 30 px above and 6 px below;
  - setting rows 4 px apart, content column at most about 1000 px wide, and the page scrolls.
- **Setting row:**
  - min height 68 px, padding 16, 1 px border, 4 px radius;
  - an optional 20 px icon;
  - title 14 regular and description 12 secondary;
  - the control on the right, at least 120 px wide.
- Changes apply at once; there is no Save button. Switches are used for on/off settings.
- **Font:**
  - family: "Segoe UI Variable Text", "Segoe UI", system-ui;
  - Windows 11 ramp: Caption 12/16, Body 14/20, Subtitle 20/28, Title 28/36;
  - sentence case, semibold rather than bold.

## Provider account page

- **Header row:** the provider icon, the name and an "N accounts" count. OpenCode also gets an "Add
  account" button.
- **Cards:** the widget's own usage card for each account, 2 per row, wrapping to 1 when the window
  is narrow.
- **Card header:**
  - the name (nickname, or else the email) and the plan badge;
  - then, on the right, the **"Show on widget" switch** and a **"⋯" button** (tooltip and
    accessible name "More options for {name}").
  - There is no footer line.
- **The "⋯" menu**, also opened by right-click, Shift+F10 or the Menu key:
  - Rename…;
  - Copy API key (OpenCode);
  - Replace API key… (OpenCode);
  - Refresh now;
  - a separator;
  - Remove account… shown in red (OpenCode).
  - Source: the Windows collection-commanding guidance, 1Password and VS Code, HIGH/MED.
- **Dialogs** are modal, about 440 px wide, over the settings window:
  - **Rename:** a pre-filled text box; Enter saves, Esc cancels.
  - **Remove:** a confirmation that names the account. The destructive button is not the default.
  - **Replace key / Add account:** a password box with a show toggle. Accept `oc_sk_…` or `sk-…`,
    with inline errors. The primary button is disabled until the input is valid.
  - **Copy key:** shows a short "Key copied" toast.
- **Signed-in account:** keeps the green highlight.
- **Show / hide:** the "⋯" menu's first item, "Hide from widget" / "Show on widget". No switch on
  the card.
- **Hidden accounts:** a gray card (dim palette tokens) with an eye-slash icon after the name,
  tooltip "Hidden from widget". No dashed or dotted borders anywhere.
- **Lapsed OpenCode subscription:** a gray card with no badge, note or hint.
- **Used up:** the widget's solid red ring.
- **Order:** the widget's order first, then the rest (`src/settings/_lib/order.ts`). OpenCode: the
  shown accounts in `rankAccounts` order, then hidden, lapsed and no-data accounts. Other providers:
  shown before hidden; the signed-in account first, used-up last, then the most recent cached use.
- **Actionable problems** ("re-auth", "key rejected") show in the card's hint area only.
- **Empty state:** "No accounts yet". OpenCode adds an "Add account" button; the other providers
  get a one-line note that they read the app's own sign-in.

## Accessibility

- Every interactive element is at least 24x24.
- Focus rings are visible and never clipped.
- Menus use roving focus with arrow / Home / End keys and return focus when they close.
- Dialogs trap focus.
- Tab order runs from the navigation to the content.
- Forced-colors (high contrast) mode is respected.
