# Plan: Claude spend rate and pace ("speedometer")

Status: **plan only** (2026-10-18). Claude first; other providers later.

## The owner's ask

The widget shows only a flat percentage. The owner wants numbers instead:

- a dollar rate, e.g. "about $100 per hour on Claude";
- whether usage is climbing fast or slow;
- where it will end up.

Today these answers need an agent to dig them up each time. The analogy is a car: knowing its position every so often is not enough; you also want its speed and arrival time.

## What data exists (checked on this PC, 2026-10-18)

### 1. Claude Code's local transcripts

Path: `%USERPROFILE%\.claude\projects\**\*.jsonl`. This covers Claude Code, the Claude desktop Code tab, agents and subagents.

- Each assistant reply records `message.usage`:
  - `input_tokens`, `output_tokens`, `cache_read_input_tokens`;
  - `cache_creation.ephemeral_5m_input_tokens` and `ephemeral_1h_input_tokens`;
  - `message.model`, `timestamp`, `message.id` and `requestId` (used to drop duplicates).
  - HIGH (read from a live file).
- Claude Code also writes `cost-state` records with its own `costUSD` per model. They serve as a cross-check. HIGH.
- Volume:
  - 3,987 files, 4.9 GB in total, 639 MB touched in the last 24 h;
  - 150,499 distinct replies in the last 7 days;
  - a full 8-day scan with a simple Node script took 16 s. HIGH (measured).

### 2. Anthropic's OAuth usage endpoint

The app already polls this every 60 s or more. It gives the 5-hour and weekly utilization percentages plus their reset times. Storing these readings over time gives the speed in percent per hour.

## Prices (API-equivalent, $ per million tokens)

| Model             | Input | Output | Cache write 5 min (1.25×) | Cache write 1 h (2×) | Cache read |
| ----------------- | ----- | ------ | ------------------------- | -------------------- | ---------- |
| claude-opus-5-5   | 4     | 20     | 5                         | 8                    | 0.20       |
| claude-sonnet-5-5 | 2     | 10     | 2.5                       | 4                    | 0.20       |
| claude-fable-5-1  | 10    | 50     | 12.5                      | 20                   | 0.25       |
| claude-haiku-5-5  | 0.10  | 0.50   | 0.125                     | 0.20                 | see note   |

- **Sources:** the claude-api reference (cached 2026-10-06) for input, output and cache reads, and `shared/prompt-caching.md` for the 1.25× / 2× write multipliers.
- **Haiku cache read:** not listed. Haiku 5.5 prices are for prompts up to 100K tokens, and larger prompts cost more.
- **Older models** (Opus 5, Sonnet 5 and earlier) go in the same table from the same reference. A model missing from the table is counted and flagged, never priced as zero.
- **Cross-check:** one real `cost-state` record (Opus 5.5) reported $4.2579004. The table above, applied to the same token counts, gives $4.2579. **HIGH:** it matches Claude Code's own cost to the cent.
- **Output tokens:** `output_tokens` already includes thinking tokens. Do not add `thinkingTokens` again.

## First real numbers (probe, 2026-10-18, this PC, Claude Code only)

API-equivalent spend:

- Last hour: **$33**
- Last 5 hours: **$353**, about $71/h on average
- Last 24 hours: **$1,293**
- Last 7 days: **$8,723**

All of it was on `claude-opus-5-5`.

The last 12 hours, newest first, in $/h: 33, 69, 81, 86, 84, 88, 119, 61, 26, 5, 6, 64.

These are "what this would cost on the API" figures. They are not the subscription bill. The probe script lives only in a scratch folder.

## What to show

### Widget, Claude card: one extra line, glanceable

- The line reads `$71/h · 5h +9%/h · lasts` or `… · limit 3:40 PM`.
  - **Spend rate:** dollars per hour over the last 60 minutes (rolling).
  - **Speed:** percent per hour on the 5-hour window. When the weekly window is the tighter one, show the weekly speed instead.
  - **Forecast:** "lasts" when the current speed won't reach 100% before reset. Otherwise, the local time it hits the limit.
- Colour, which only applies when the owner needs to act:
  - normal: muted text;
  - on track to hit a limit before it resets: amber;
  - under 30 minutes left: red.
- Tooltip: the 15-minute rate, both windows' speeds, and "Claude Code on this PC".

### Settings → Claude: a "Spend & pace" section

| Spend        | Last hour | Last 5 h | Today | Last 7 days |
| ------------ | --------- | -------- | ----- | ----------- |
| API-equivalent | $       | $        | $     | $           |

Below the table:

- spend per model, with each model's share of the total;
- a 24-hour bar chart of $/h in 15-minute or hourly buckets (plain SVG, no chart library);
- for each window (5-hour and weekly):
  - current %;
  - speed in %/h;
  - even-pace % (elapsed time ÷ window length);
  - "ahead / behind pace";
  - the projected time of hitting 100%, or "lasts until reset".
- **Calibration:** "1% of your weekly limit ≈ $X of usage", measured over the current weekly window as dollars spent ÷ percentage points gained.
- **Coverage note** (once, muted): "Dollars cover Claude Code on this PC. Percentages cover all your Claude use."

## Definitions

- **Spend rate:** the sum of reply cost with a timestamp in (now − 60 min, now], in $/h. Also compute a 15-minute rate × 4 for the tooltip.
- **Speed (%/h):** a least-squares slope over the readings from the last 60 minutes, using only the readings after the most recent reset. With fewer than 3 readings or less than 15 minutes of data, show "measuring…".
- **Projected limit time:** now + (100 − current %) / speed. If that is later than `resetsAt`, show "lasts until reset". If the speed is 0 or less, show "lasts".
- **Even pace:** window length × elapsed fraction. Ahead means current % is above even pace.

## Architecture (Rust engine, React UI)

- **`src-tauri/src/history.rs`**
  - Append every fresh Claude usage reading (time, 5-hour %, weekly %, both reset times) to `usage-history.jsonl` in the app data folder.
  - Keep 14 days; thin readings older than 24 hours to one every 5 minutes.
  - Generic per provider, but only Claude is used in version 1.
- **`src-tauri/src/claude_logs.rs`**
  - Read `~/.claude/projects/**/*.jsonl` **read-only and incrementally**:
    - keep a byte offset per file in `claude-logs-index.json`;
    - on first run, backfill only files changed in the last 8 days;
    - after that, read only newly appended bytes every 60 s on a blocking thread.
  - Parse only assistant lines that carry `usage`, and only the usage fields. Never keep message content.
  - Drop duplicates by `message.id` + `requestId`, with a bounded seen-set covering 8 days.
  - Aggregate into one-minute buckets per model, split by token type, in `spend-buckets.json` (rolling 8 days).
  - Never write to Claude's files.
- **`src-tauri/src/pricing.rs`**
  - The table above plus the older models.
  - An unknown model is counted and logged once, and the UI says "N replies on unpriced models".
  - A unit test reproduces the `cost-state` cross-check from a redacted fixture.
- **API:** `GET /api/claude/pace` returns:
  - `spend {lastHour, last5h, today, last7d, rate60m, rate15m, byModel, hourly[24]}`;
  - `windows {fiveHour, weekly}`, each `{pct, pctPerHour, evenPacePct, etaToLimit, resetsAt, state: "measuring"|"lasts"|"limit"}`;
  - `calibration {usdPerWeeklyPct}`;
  - `coverage: "claude-code-this-pc"`.
- **UI:** the widget Claude card's speed line, plus the Settings "Spend & pace" section. Mock data covers every state.

## Performance and privacy budgets

- The first backfill runs once in the background. It should take 30 s or less, based on the 16 s measured with Node.
- Steady state: under 1 s of CPU per minute, and under 20 MB of extra memory.
- Only aggregates are stored. No prompts or replies are ever read into storage, logged or sent anywhere.
- `SECURITY.md` gains a line about reading the Claude Code transcripts (usage fields only).

## Tests (light)

- The scanner, against a small redacted fixture:
  - duplicates dropped;
  - several models;
  - the 5 min / 1 h cache split;
  - a file growing between scans;
  - a truncated last line.
- The pricing maths, including the cost-state cross-check.
- Slope, projected time and pace maths:
  - a reset inside the window;
  - too few readings;
  - zero or negative speed.
- History thinning.

## Build plan (after the owner says go)

1. **Two builders in parallel**, each with a contract fixed in advance:
   - Engine: `history.rs`, `claude_logs.rs`, `pricing.rs` and the API route.
   - UI: the widget line, the settings section and the mock data.
2. **Live check on this PC:** the widget's $/h must match the probe within a few percent, and the backfill time and memory must be measured.
3. **Then:** install, plus a docs update.

Expected time: about 30-40 minutes. No new dependencies are expected.

## Later (not in version 1)

- **Codex:** `~/.codex/sessions/**/*.jsonl` logs token counts; same method.
- **OpenCode and Cursor:** percent speed only, unless their logs or APIs give token counts.
- **Daily and weekly reports**, and a "today vs typical day" comparison.
