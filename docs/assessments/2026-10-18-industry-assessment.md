# UsageDeck assessment against industry standards

Date: 2026-10-18 18:09 PDT. Code: branch `claudefix`, commit `f96688b`, the installed build of that
commit.

**Method.** Four reviewers worked in parallel, read-only:

1. UI/UX and accessibility, from headless screenshots of the mock data.
2. Performance, measured on the installed app on the owner's PC.
3. Reliability, security and privacy.
4. Features against similar tools, code quality, and packaging.

Each area is scored 1-10 against one rubric:

- **5:** works, but clearly below what maintained peers ship.
- **7:** on par with good open-source peers (CodexBar, Win-CodexBar, OpenQuota).
- **9:** best in class.

Confidence tags: HIGH means measured or read in the code. MED means from public sources or a
reasoned inference. LOW means an estimate.

## Scorecard

| Area                                   | Score | Why, in one line                                                                                         |
| -------------------------------------- | ----- | -------------------------------------------------------------------------------------------------------- |
| UI / UX and visual design              | 6     | Cards read well at a glance and look the same everywhere; text is small, labels are cryptic, cached state is thin |
| Accessibility (WCAG 2.2 AA)            | 6     | Strong screen-reader roles; buttons too small to hit, dimmed text too faint, menu keyboard handling incomplete |
| Performance and resource use           | 6     | Near-zero CPU, 0.1 s start-up, 4 MB installer; about 283 MB of memory and polling that is not smart     |
| Reliability and error handling         | 7     | Typed errors, last good value kept, atomic writes; no backoff on rate limits, failure reason never shown |
| Security and privacy                   | 6     | Other apps' sign-ins read only, keys in Credential Manager; no content policy, full key sent to the page |
| Features compared with peers           | 5     | Good multi-account OpenCode tracking and ranking; no alerts, history, pace or hotkey                    |
| Code quality, testing and CI           | 7     | Clean modules, all CI steps pass; shared types kept by hand, no UI tests                                 |
| Packaging, distribution and updates    | 3     | One per-user installer; no signing, updater, release pipeline or crash reports                          |
| **Average**                            | **5.75** |                                                                                                       |

Read it this way:

- **The inside is solid:** reliability 7 and code quality 7.
- **What users see is middling:** UI, accessibility, performance and security all sit at 6.
- **Product polish is the weakest part:** features 5, packaging 3.

Most of the fixes that raise a score are small.

## 1. UI / UX and visual design: 6

Strengths:

- **At-a-glance reading.** Every meter has a percentage and a coloured bar: green, amber from 80%,
  red at 100%. Accounts that are used up get a red glow (`src/widget/_components/Card.tsx:42-47,121-128`).
  HIGH
- **One card component for the widget and settings.** Account states are clear. Empty states are
  helpful. Offline shows a banner with Retry (`src/widget/App.tsx:288-306`). HIGH

Gaps:

- **The type is far below the Windows 11 (Fluent) type scale.** All widget text is 10-11 px Arial
  (`src/globals.css:13`) instead of Segoe UI Variable at 12 px or more. There is no Mica or Acrylic
  material. HIGH
- **The right-click menu border is broken.** `border-(--w-border)` points to a variable that is
  never defined (`src/widget/App.tsx:320,352`), so the menu gets a hard white (dark theme) or black
  (light theme) border. HIGH
- **Labels are cryptic.** OpenCode meters are labelled "R / W / M", and "↺ 1" means reset credits.
  "re-auth" and "key rejected" explain what to do only in a hover tooltip. The copy-key action
  appears only on hover (`src/widget/_components/cards.tsx:59-64,140-162`). MED
- **Cached data shows its age only in settings.** Settings says "updated 3h ago"; the widget shows
  cached numbers with no age. MED
- **The settings cards stay at widget scale** (10 px text inside a 720 px dialog), so the dialog
  looks mostly empty. MED

Top fixes:

1. Define the missing menu border colour (S).
2. Spell out meter labels, and show "updated Xh ago" on widget cards with cached data (S-M).
3. Segoe UI Variable with a 12 px minimum, larger settings cards, and Mica via Tauri window effects
   (M).

## 2. Accessibility (WCAG 2.2 AA): 6

Strengths:

- **Base colours pass contrast.** The palette contrast tests pass (57/57). HIGH
- **Screen-reader structure is correct:**
  - meters have a name, a value and a spoken reset time (`Card.tsx:153-161`);
  - tabs use roving focus with arrow keys (`src/ui/Tabs.tsx:47-62`);
  - radio groups, switches and a native dialog are used;
  - reduced motion is respected (`src/globals.css:51-64`).

  HIGH

Gaps:

- **Dimmed accounts fail contrast (1.4.3).** The `opacity-60` used for signed-out and lapsed
  accounts (`Card.tsx:64`) drops muted text to 3.25:1 in dark and 2.62:1 in light. The tests only
  check base colour pairs. HIGH
- **Click targets are under 24 px (2.5.8).** Copy-key is 16 px, switches 18 px, and gear and trash
  are 20 px side by side (`src/ui/controls.tsx:28,143`). HIGH
- **The menu keyboard pattern is incomplete.** Opening it leaves focus on the page body, and there
  is no arrow-key navigation (`App.tsx:82-89,313-358`). HIGH
- **The tab panel's focus ring is clipped** by `overflow-hidden` on the dialog. MED

Top fixes:

1. 24 px hit areas; the visuals can stay the same (S).
2. Replace opacity dimming with colour tokens that pass contrast, and test those combinations (S).
3. Menu: focus the first item on open, support arrow / Home / End, and return focus on close (S-M).

## 3. Performance and resource use: 6

Measured on this PC, idle for 2 minutes:

| Measure                    | UsageDeck (Tauri)                                                       | Old Electron app (measured earlier today) |
| -------------------------- | ----------------------------------------------------------------------- | ----------------------------------------- |
| Processes                  | 8                                                                       | 9                                         |
| Private memory             | 283 MB average (268-289)                                                | 425 MB                                    |
| Working set (summed)       | 459 MB                                                                  | 417 MB                                    |
| CPU                        | 0.01% of the PC on average, 0.10% at peak                               | not measured                              |
| Start-up to window visible | 127 ms (one run)                                                        | not measured                              |
| Installer / installed size | 4.06 MB / 15.6 MB                                                       | about 860 MB of dependencies in the repo  |
| Polling                    | every 60 s; each provider's cache lasts 10-60 s, so every poll fetches again | similar                              |

All measurements are HIGH. Peer figures are LOW: native tray tools are said to use 5-50 MB, and
Tauri on WebView2 is close to Electron once shared memory is counted (MED, tauri issue #5889).

Gaps:

- **The WebView2 GPU process is the biggest single cost** (141 MB private). The settings window is
  created hidden at launch, which keeps a second renderer (about 30 MB) loaded all the time. HIGH
- **Polling never pauses** while the widget is hidden, the session is locked or the PC sleeps
  (no handling found). HIGH
- **The cache file is rewritten after every successful fetch:** about 17 times a minute, roughly
  8 MB of writes an hour (`src-tauri/src/providers/mod.rs:184-222`). HIGH

Top fixes:

1. Smarter polling: pause when hidden or locked, write the cache once per poll and only on change,
   and use longer intervals for slow-moving windows (S-M).
2. Create the settings window only when it is opened, and destroy it on close (S, about 30 MB).
3. Try running without GPU compositing for this small static window. This is a hypothesis to test,
   because transparency may need the GPU (S).

## 4. Reliability and error handling: 7

Strengths:

- Concurrent requests for one account share one fetch.
- A good cached value is never overwritten by a failure or a malformed reply.
- Every request has a timeout.
- Persistence is atomic, with per-file locks.
- Timers ignore wall-clock changes.
- There is almost no `unwrap` on runtime paths.

HIGH

Gaps:

- **No backoff on rate limits.** A 429 is only cached for 15-120 s. `Retry-After` is ignored and
  there is no exponential backoff, so a rate-limited Claude endpoint is asked again every minute
  (`providers/mod.rs:96,304-333`, `claude.rs:29-32`). HIGH
- **The failure reason never reaches the user.** `ProviderResult.error` is never sent to the
  frontend (`mod.rs:115-117`, `#[serde(skip)]`). The UI cannot tell rate-limited from
  format-changed from offline, and `--dump` always shows a null error. HIGH
- **Writes are not durable.** There is no flush to disk before the rename, and a corrupt settings
  file silently becomes defaults and is then overwritten (`store.rs:57-60,85-88,198-205`). MED
- **A blocking SQLite read runs on an async worker** for Cursor (`cursor.rs:270`). HIGH
- **A failed key read can permanently drop an OpenCode account** (`store.rs:232-269`). PLAUSIBLE
- **The log keeps very little history:** about 40 KB in one file, and it fills within hours. HIGH

Top fixes:

1. Read `Retry-After` and back off exponentially, up to about 15 minutes (S).
2. Send the error reason to the cards and to `--dump` (S).
3. Flush to disk before rename, move corrupt files aside, and run the Cursor read on a blocking
   thread (S).

## 5. Security and privacy: 6

Strengths:

- Other apps' sign-ins are only ever read; there is no token refresh, and Cursor's database is
  opened read-only.
- OpenCode keys are stored only in Credential Manager.
- DPAPI is called in-process.
- TLS uses rustls only.
- Nothing listens on a network port, and there is no telemetry.
- The webview gets few permissions.
- The test fixtures are free of secrets.

HIGH

Gaps:

- **No content security policy, and the full OpenCode key is sent into the page.** The policy is
  `csp: null`, and `/api/key` returns the full key (`src-tauri/src/api.rs:172-186`). Any injected
  script could read and send out every key. That is unlikely, since there are no raw-HTML insertion
  points in the code. Exposure HIGH, likelihood LOW.
- **Rust dependencies are never audited.** CI runs only `npm audit`, which found 0 issues. HIGH
- **Unsigned installer and no updater,** so no way to check a download is authentic and no path
  for security fixes. HIGH
- **Terms-of-service risk.** Requests imitate the official clients' user agents (`claude.rs:491`,
  `codex.rs:18`) to call undocumented endpoints with first-party tokens. The code does this (HIGH);
  the size of the risk is a judgement.
- **`SECURITY.md` leaves out two things:** the screen sampler, which reads the pixels around the
  widget and averages them in memory, and log lines that contain account emails (`mod.rs:305`).
  HIGH
- **Small hardening gaps:**
  - DPAPI is called without the no-UI flag;
  - page script can toggle autostart;
  - every command is open to both windows.

Top fixes:

1. A strict content security policy, and copy the key to the clipboard from Rust so it never
   enters the page (S).
2. `cargo audit` or `cargo deny` in CI (S).
3. A signed installer and the Tauri updater with signed update files (M; signing needs a
   certificate, which costs money).

## 6. Features compared with similar tools: 5

| Feature              | UsageDeck                        | CodexBar      | Win-CodexBar      | CodeZeno       | OpenQuota   |
| -------------------- | -------------------------------- | ------------- | ----------------- | -------------- | ----------- |
| Providers            | 5                                | ~100          | ~56               | ~5             | several     |
| Multiple accounts    | OpenCode (+ remembered sign-ins) | yes           | yes               | no             | ?           |
| Alerts / thresholds  | no                               | yes           | yes               | ?              | pace alerts |
| History / charts     | no                               | burndown      | ?                 | no             | ?           |
| Pace indicator       | no                               | yes           | yes               | ?              | yes         |
| Tray                 | menu + icon                      | menu bar      | pace-coloured     | taskbar        | popup       |
| Global hotkey        | no                               | ?             | yes               | ?              | ?           |
| Privacy mode         | no                               | ?             | yes               | ?              | ?           |
| CLI / API            | `--dump` JSON                    | JSON CLI      | ?                 | ?              | ?           |
| Auto-update          | no                               | yes           | ?                 | ?              | ?           |
| Start-at-login setting | always on, no toggle           | yes           | yes               | ?              | ?           |

Peer cells are MED, from earlier research; "?" means not checked.

Unique strengths:

- per-account change detection, with the most recently used account floated to the top;
- rules for used-up accounts;
- the DeepSeek peak-hour hint;
- adaptive theme sampling.

HIGH

Top fixes:

1. Notifications at a threshold (for example 70% and 90%) and on reset (M).
2. Usage history with a pace or burndown line (M-L).
3. A start-at-login toggle and an "open log folder" entry (S).

## 7. Code quality, testing and CI: 7

Strengths:

- Sensible module layout: Rust `providers/`, `win/`, `store`, `api`, `cli`; frontend `widget/`,
  `settings/`, `ui/`, `lib/`.
- Every CI step passes locally: 87 Vitest, 35 cargo tests, clean fmt and clippy, Prettier clean.
- Accurate README and `SECURITY.md`.

HIGH

Gaps:

- **Shared types are kept in sync by hand.** The plan asked for ts-rs or specta generation, and it
  was not done. The header of `src/lib/types.ts` still describes the retired Next.js server. HIGH
- **No UI or end-to-end tests.** Vitest runs in a Node environment, and the live provider tests are
  skipped in CI. HIGH
- **Duplicated card code:** `CursorCard` and `GrokCard` differ on only 48 of 144 lines
  (`cards.tsx:217-360`). HIGH
- **CI has no Rust build cache,** so a cold Windows build is slow. There is no LICENSE or
  CHANGELOG. MED

Top fixes:

1. Generate the shared types and check for drift in CI (M).
2. One data-driven card component (S-M).
3. A few render tests per card state, plus a scheduled run of the live provider tests (M).

## 8. Packaging, distribution and updates: 3

Gaps:

- **NSIS per-user installer only:** no signing (SmartScreen warns), no updater plugin, and no
  tag-triggered release job (`src-tauri/tauri.conf.json:48-59`). HIGH
- **Version 0.1.0 is not tied to releases.** HIGH
- **No crash reports, and the logs can't be reached from the app.** HIGH
- **Uninstall probably leaves the stored keys and the autostart entry behind.** Not verified, MED.
- **The app ID `com.usagedeck.app` ends in `.app`,** which Tauri warns about because of macOS. It
  is harmless on Windows, but renaming it now needs a data move. HIGH

Top fixes:

1. A tag-triggered release job with checksums (S-M).
2. The updater with signed update files, plus code signing (M; certificate cost).
3. A crash file and a capped log, and an uninstall cleanup of the startup entry and the keys (S-M).

## Ranked backlog across all areas

Ranked by impact for effort. "Lifts" names the score each item moves.

| #   | Change                                                                                           | Effort | Lifts                   |
| --- | ------------------------------------------------------------------------------------------------ | ------ | ----------------------- |
| 1   | Fix the menu border colour; 24 px hit areas; dimmed text that passes contrast                    | S      | UI, Accessibility       |
| 2   | Back off on 429 (honour `Retry-After`); show the error reason and "updated Xh ago" on cards      | S      | Reliability, UI         |
| 3   | Strict content security policy; copy keys from Rust                                              | S      | Security                |
| 4   | Smarter polling (pause when hidden or locked, write the cache only on change); create the settings window on demand | S-M | Performance |
| 5   | Start-at-login toggle, open-log entry, larger log, flush before rename, `cargo audit` in CI      | S      | Features, Reliability, Security |
| 6   | Spelled-out meter labels; Segoe UI Variable with a 12 px minimum; larger settings cards; Mica    | M      | UI                      |
| 7   | Threshold and reset notifications                                                                | M      | Features                |
| 8   | Usage history with a pace line                                                                   | M-L    | Features, UI            |
| 9   | Release job, updater, code signing                                                               | M      | Packaging, Security     |
| 10  | Generated shared types; one data-driven card; render tests                                       | M      | Code quality            |

Estimate (LOW): items 1-5 are small and could lift UI, accessibility, performance, reliability and
security by about one point each, for an average of about 6.5. Items 6-8 are what move the product
from "works" to on par with its peers.

## Notes and corrections from this session

- **Not a real bug: the off-centre settings window.** The owner was told the settings window opened
  off-centre on the 4K display at 200% scale. That was a screenshot artefact: the capture was not
  DPI-aware. Measured properly, the panel is centred. The fix that shipped addressed a real related
  issue: a dragged panel reopened where it was left.
- **Agent incidents on the live desktop:**
  - The fixer's scripted drags selected text in the owner's foreground app, and it pressed Escape
    once.
  - The performance tester's restart left the widget down for about 88 s, from 18:06:40 to
    18:08:07, before it was started again.
  - Both have been reported to the owner. Future live checks must not send input while the owner
    is active.
- **Not checked in this assessment:**
  - screen-reader output;
  - auto-theme behaviour on the real desktop;
  - WebView2 crash recovery;
  - sleep / resume behaviour;
  - CI timing on GitHub Actions;
  - uninstall leftovers;
  - peer apps measured on this PC.
