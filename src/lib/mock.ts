// Browser-only mock of the Rust `api` command, so `npm run dev` can be viewed
// in a normal browser. Data adapted from tests/e2e/fixtures.ts, with times
// relative to the page load instead of a frozen clock.
import { applySettingsPatch } from "./settings-schema";
import type {
  ClaudePace,
  ClaudeUsage,
  CodexUsage,
  CursorUsage,
  GrokUsage,
  OpenCodeAccountSummary,
  OpenCodeAccountUsage,
  Settings,
  TrackedAccount,
} from "./types";
import type { UpdatePhase, UpdateStatus } from "./updates";

const NOW = Date.now();
const H = 3_600_000;
const query = () => new URLSearchParams(typeof location === "undefined" ? "" : location.search);
// `?codexReset=5000` puts the used-up Codex card's 5-hour reset that many ms after page load
// (browser preview only); it defaults to 1h 21m.
const CODEX_RESET_MS = Number(query().get("codexReset")) || 81 * 60_000;
const iso = (msFromNow: number) => new Date(NOW + msFromNow).toISOString();

let settings: Settings = {
  opencodeEnabled: true,
  cursorEnabled: true,
  grokEnabled: true,
  codexEnabled: true,
  claudeEnabled: true,
  deepseekPeakHourWarning: true,
  hideInFullscreen: true,
  showProviderNames: false,
  themeMode: "dark",
  panelTheme: "dark",
  displayNames: { "main@example.com": "Main", "spare@example.com": "Spare" },
  disabledAccounts: ["hidden@example.com", "claude:hidden@example.com"],
  providerOrder: ["opencode", "cursor", "grok", "codex", "claude"],
  startAtLogin: true,
  claudeHourlyTarget: null,
  snoozedUntil: {},
};

const win = (percent: number, resetsIn: number) => ({
  status: "ok",
  percent,
  resetsAt: iso(resetsIn),
});

function account(
  email: string,
  r: number,
  w: number,
  m: number,
  extra: Partial<OpenCodeAccountUsage> = {},
): OpenCodeAccountUsage {
  return {
    email,
    usage: { rolling: win(r, 4 * H), weekly: win(w, 6 * 24 * H), monthly: win(m, 15 * 24 * H) },
    prev: null,
    enabled: true,
    error: null,
    stale: false,
    cachedAt: null,
    subscriptionActive: true,
    lastChangeAt: null,
    lastChangeDetail: null,
    ...extra,
  } as OpenCodeAccountUsage;
}

let usage: OpenCodeAccountUsage[] = [
  account("main@example.com", 2, 10, 85, { lastChangeAt: NOW - 60_000 }),
  account("spare@example.com", 0, 100, 40, {
    error: "rate-limited",
    stale: true,
    cachedAt: NOW - 2 * H,
  }),
  account("hidden@example.com", 5, 5, 5, { enabled: false }),
  account("lapsed@example.com", 0, 0, 0, { subscriptionActive: false }),
];

const summaries = (): OpenCodeAccountSummary[] =>
  usage.map((u) => ({ email: u.email, keyHint: "oc_sk_…ab12" }) as OpenCodeAccountSummary);

function tracked<T>(
  id: string,
  usage: T,
  extra: Partial<TrackedAccount<T>> = {},
): { accounts: TrackedAccount<T>[] } {
  return {
    accounts: [
      {
        id,
        label: id,
        active: true,
        usage,
        stale: false,
        cachedAt: null,
        authExpired: false,
        ...extra,
      } as TrackedAccount<T>,
    ],
  };
}

// The cached cards carry transient errors (network, schema-changed, http, locked); they render
// exactly like fresh ones, with no hint. Only Claude's rejected sign-in shows "re-auth".
const providers: Record<string, unknown> = {
  cursor: tracked<CursorUsage>(
    "dev@example.com",
    {
      cursorModelsPercentUsed: 86,
      otherModelsPercentUsed: 54,
      resetsAt: iso(10 * 24 * H),
      onDemand: { state: "enabled", spentUsd: 12.4, limitUsd: 50 },
      accountName: "dev@example.com",
      planName: "Pro",
    } as CursorUsage,
    { stale: true, cachedAt: NOW - 25 * 60_000, error: "network" },
  ),
  grok: tracked<GrokUsage>(
    "dev@example.com",
    {
      email: "dev@example.com",
      name: null,
      percentUsed: 40,
      resetAt: iso(3 * 24 * H),
      planLabel: "SuperGrok",
      cursorPlanName: null,
      source: "grok-bot",
    } as GrokUsage,
    { stale: true, cachedAt: NOW - H, error: "schema-changed" },
  ),
  codex: tracked<CodexUsage>(
    "c@example.com",
    {
      email: "c@example.com",
      planType: "plus",
      usedPercent: 100,
      limitWindowSeconds: 18000,
      resetAt: iso(CODEX_RESET_MS),
      weeklyUsedPercent: 64,
      weeklyLimitWindowSeconds: 604800,
      weeklyResetAt: iso(4 * 24 * H),
      creditsBalance: null,
      resetCredits: 1,
      limitReached: true,
    } as CodexUsage,
    { stale: true, cachedAt: NOW - 5 * H, error: "http" },
  ),
  claude: tracked<ClaudeUsage>(
    "me@example.com",
    {
      email: "me@example.com",
      displayName: "Me",
      signedIn: true,
      subscribed: true,
      subscriptionType: "pro",
      planName: "Pro",
      fiveHour: { percent: 31, resetsAt: iso(4 * H) },
      weekly: { percent: 60, resetsAt: iso(52 * H) },
      weeklyOpus: null,
      weeklySonnet: null,
      resetCredits: null,
      billingType: null,
      rateLimitTier: null,
      orgUuid: null,
    } as ClaudeUsage,
    { authExpired: true, error: "auth-rejected" },
  ),
};

// `?cursorAuth=rejected` shows Cursor's sign-in as expired, its last usage cached and used up (browser
// preview only). "Open Cursor" then brings it back a few seconds later, as Cursor renewing it would.
const cursorRow = (providers.cursor as { accounts: TrackedAccount<CursorUsage>[] }).accounts[0];
if (query().get("cursorAuth") === "rejected") {
  Object.assign(cursorRow, {
    authExpired: true,
    error: "auth-rejected",
    stale: true,
    usage: {
      ...cursorRow.usage,
      cursorModelsPercentUsed: 100,
      otherModelsPercentUsed: 100,
      onDemand: { state: "disabled" },
    },
  });
}

/** Browser stand-in for the Rust `open_cursor` command. */
export function mockOpenCursor(): void {
  setTimeout(() => {
    Object.assign(cursorRow, { authExpired: false, error: null, stale: false, cachedAt: null });
  }, 3_000);
}

// A remembered Codex account whose sign-in file is gone (not the signed-in one: no hint).
(providers.codex as { accounts: TrackedAccount<CodexUsage>[] }).accounts.push(
  tracked<CodexUsage | null>("old@example.com", null, {
    active: false,
    error: "missing-credentials",
  }).accounts[0] as TrackedAccount<CodexUsage>,
);

// A remembered Claude account that is not signed in: its cached usage, shown like fresh usage,
// with the 5h window already past its reset (shows "--") and the weekly one still running.
(providers.claude as { accounts: TrackedAccount<ClaudeUsage>[] }).accounts.push(
  tracked<ClaudeUsage>(
    "work@example.com",
    {
      email: "work@example.com",
      displayName: "Work",
      signedIn: true,
      subscribed: true,
      subscriptionType: "max",
      planName: "Ultra",
      fiveHour: { percent: 92, resetsAt: iso(-H) },
      weekly: { percent: 47, resetsAt: iso(3 * 24 * H) },
      weeklyOpus: null,
      weeklySonnet: null,
      resetCredits: 2,
      billingType: null,
      rateLimitTier: null,
      orgUuid: null,
    } as ClaudeUsage,
    { active: false, stale: true, cachedAt: NOW - 3 * H, error: "locked" },
  ).accounts[0],
);

// Two more remembered Claude accounts: one used up on its weekly window, one hidden from the widget.
const claudeUsage = (email: string, five: number, weekly: number): ClaudeUsage =>
  ({
    email,
    displayName: null,
    signedIn: true,
    subscribed: true,
    subscriptionType: "pro",
    planName: "Pro",
    fiveHour: { percent: five, resetsAt: iso(2 * H) },
    weekly: { percent: weekly, resetsAt: iso(30 * H) },
    weeklyOpus: null,
    weeklySonnet: null,
    resetCredits: null,
    billingType: null,
    rateLimitTier: null,
    orgUuid: null,
  }) as ClaudeUsage;
(providers.claude as { accounts: TrackedAccount<ClaudeUsage>[] }).accounts.push(
  tracked("full@example.com", claudeUsage("full@example.com", 40, 100), {
    active: false,
    stale: true,
    cachedAt: NOW - H,
  }).accounts[0],
  tracked("hidden@example.com", claudeUsage("hidden@example.com", 12, 20), {
    active: false,
    stale: true,
    cachedAt: NOW - 5 * H,
  }).accounts[0],
);

// `?panel=light` shows the settings panel in its light theme (browser preview only).
if (typeof location !== "undefined") {
  const panel = new URLSearchParams(location.search).get("panel");
  if (panel === "light" || panel === "dark" || panel === "system")
    settings = { ...settings, panelTheme: panel };
  // `?target=100` sets the Claude hourly spend target (browser preview only).
  const target = Number(new URLSearchParams(location.search).get("target"));
  if (target > 0) settings = { ...settings, claudeHourlyTarget: Math.round(target) };
}

/** Browser stand-in for the Rust `copy_key` command: fails for an unknown account, like Rust. */
export async function mockCopyKey(email: string): Promise<void> {
  if (!usage.some((u) => u.email === email.toLowerCase())) throw new Error("unknown account");
  await navigator.clipboard?.writeText("sk-test-key-123");
}

type Reply = { status: number; body: unknown };
const json = (body: unknown, status = 200): Reply => ({ status, body });

// ---- Claude spend and pace ------------------------------------------------

export type PaceScenario = "lasts" | "limit" | "weekly" | "measuring" | "backfill" | "unpriced";
const PACE_SCENARIOS: PaceScenario[] = [
  "lasts",
  "limit",
  "weekly",
  "measuring",
  "backfill",
  "unpriced",
];

// Windows match the active mock Claude account (31% / 60%). The 2026-10-18 probe (newest first, $/h), padded to 24 hours.
const PROBE_HOURLY = [
  33, 69, 81, 86, 84, 88, 119, 61, 26, 5, 6, 64, 72, 58, 40, 12, 0, 0, 0, 3, 18, 44, 57, 66,
];

/**
 * A realistic /api/claude/pace reply for one scenario: "lasts" (normal), "limit" (5-hour limit
 * in 25 min, red), "weekly" (weekly limit before reset, amber), "measuring", "backfill" and
 * "unpriced" (replies on models without a price).
 */
export function paceScenario(scenario: PaceScenario, now = NOW): ClaudePace {
  const at = (ms: number) => new Date(now + ms).toISOString();
  const pace: ClaudePace = {
    spend: {
      lastHour: 33.4,
      last5h: 353.1,
      today: 612.8,
      last7d: 8723.4,
      rate60m: 71.2,
      rate15m: 64.6,
      byModel: [
        { model: "claude-opus-5-5", usd24h: 1240.3 },
        { model: "claude-sonnet-5-5", usd24h: 48.6 },
        { model: "claude-haiku-5-5", usd24h: 4.2 },
      ],
      hourly: [...PROBE_HOURLY],
      unpricedReplies: 0,
      scannedAt: now - 20_000,
      backfilling: false,
    },
    windows: {
      fiveHour: {
        pct: 31,
        pctPerHour: 9.2,
        evenPacePct: 20,
        etaToLimit: null,
        resetsAt: at(4 * H),
        state: "lasts",
      },
      weekly: {
        pct: 60,
        pctPerHour: 0.4,
        evenPacePct: 69,
        etaToLimit: null,
        resetsAt: at(52 * H),
        state: "lasts",
      },
    },
    calibration: { usdPerWeeklyPct: 185.6 },
    coverage: "claude-code-this-pc",
  };
  const fiveHour = pace.windows.fiveHour!;
  const weekly = pace.windows.weekly!;
  switch (scenario) {
    case "limit":
      Object.assign(pace.spend, { rate60m: 128.4, rate15m: 151.2, lastHour: 128.4 });
      Object.assign(fiveHour, { pct: 88, pctPerHour: 28.8, evenPacePct: 20 });
      Object.assign(fiveHour, { etaToLimit: now + 25 * 60_000, state: "limit" });
      break;
    case "weekly":
      Object.assign(weekly, { pct: 60, pctPerHour: 1.9, evenPacePct: 69 });
      Object.assign(weekly, { etaToLimit: now + 21 * H, state: "limit" });
      break;
    case "measuring":
      Object.assign(fiveHour, { pct: 2, pctPerHour: null, evenPacePct: 1, state: "measuring" });
      break;
    case "backfill":
      Object.assign(pace.spend, { lastHour: 0, last5h: 0, today: 0, last7d: 0, rate60m: 0 });
      Object.assign(pace.spend, { rate15m: 0, byModel: [], scannedAt: null, backfilling: true });
      pace.spend.hourly = new Array(24).fill(0);
      pace.calibration.usdPerWeeklyPct = null;
      break;
    case "unpriced":
      pace.spend.unpricedReplies = 17;
      break;
  }
  return pace;
}

/** `?pace=limit` picks a scenario (browser preview only); `?pace=fail` makes the call fail. */
function paceReply(): Reply {
  const q =
    typeof location === "undefined" ? null : new URLSearchParams(location.search).get("pace");
  if (q === "fail") return json({ error: "pace unavailable" }, 500);
  const scenario = PACE_SCENARIOS.find((s) => s === q) ?? "lasts";
  return json(paceScenario(scenario, Date.now()));
}

export async function mockApi(method: string, path: string, body: unknown): Promise<Reply> {
  const url = new URL(path, "http://mock.local");
  const name = url.pathname.replace(/^\/api\//, "");
  const b = (body ?? {}) as Record<string, unknown>;
  if (method !== "GET") {
    if (name === "settings") {
      settings = applySettingsPatch(settings, b);
      return json({ ok: true, settings });
    }
    if (name === "accounts") {
      const email = String(b.email ?? "").toLowerCase();
      if (method === "DELETE") usage = usage.filter((u) => u.email !== email);
      else if (email && !usage.some((u) => u.email === email))
        usage = [...usage, account(email, 0, 0, 0)];
      return json({ ok: true });
    }
    return json({ error: "not found" }, 404);
  }
  switch (name) {
    case "settings":
      return json({ settings });
    case "usage":
      return json({ accounts: usage });
    case "accounts":
      return json({ accounts: summaries() });
    case "claude/pace":
      return paceReply();
    default:
      if (name in providers) return json(providers[name]);
      return json({ error: "not found" }, 404);
  }
}

// ---- updater ----------------------------------------------------------------

const MOCK_VERSION = "1.4.0";
const MOCK_NEXT = "1.5.0";
const MOCK_STEP_MS = 1200;
const MOCK_PHASES: Record<string, UpdatePhase> = {
  idle: { state: "idle" },
  checking: { state: "checking" },
  downloading: { state: "downloading", version: MOCK_NEXT },
  upToDate: { state: "upToDate" },
  failed: { state: "failed", message: "network error: could not reach api.github.com" },
  ready: { state: "ready", version: MOCK_NEXT, dismissed: false },
};
// `?update=ready` starts the preview in that phase (idle, checking, downloading, upToDate, failed,
// ready; `never` is idle with no check yet). `?updateResult=ready` (or upToDate, failed) is what
// "Check for updates" finds; default upToDate. Browser preview only.
const updateQuery = query().get("update") ?? "idle";
let updateStatus: UpdateStatus = {
  ...(MOCK_PHASES[updateQuery] ?? MOCK_PHASES.idle),
  lastChecked: updateQuery === "never" ? null : NOW - 5 * 60_000,
  currentVersion: MOCK_VERSION,
};
const updateListeners = new Set<(status: UpdateStatus) => void>();

function setUpdatePhase(phase: UpdatePhase, checked = false) {
  updateStatus = {
    ...phase,
    lastChecked: checked ? Date.now() : updateStatus.lastChecked,
    currentVersion: MOCK_VERSION,
  };
  for (const cb of updateListeners) cb(updateStatus);
}

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/** Browser stand-in for the Rust updater commands and the `update-status` event. */
export const mockUpdates = {
  status: async () => updateStatus,
  subscribe(cb: (status: UpdateStatus) => void) {
    updateListeners.add(cb);
    return () => void updateListeners.delete(cb);
  },
  async check() {
    if (updateStatus.state === "ready") {
      setUpdatePhase({ ...updateStatus, dismissed: false });
      return;
    }
    setUpdatePhase(MOCK_PHASES.checking);
    await wait(MOCK_STEP_MS);
    const result = query().get("updateResult") ?? "upToDate";
    if (result === "ready") {
      setUpdatePhase(MOCK_PHASES.downloading);
      await wait(MOCK_STEP_MS);
    }
    setUpdatePhase(MOCK_PHASES[result] ?? MOCK_PHASES.upToDate, true);
  },
  async install() {
    if (updateStatus.state !== "ready") throw new Error("no update is ready to install");
    console.info("mock install_update: the app would restart into the installer");
  },
  async dismiss() {
    console.info("mock dismiss_update");
    if (updateStatus.state === "ready") setUpdatePhase({ ...updateStatus, dismissed: true });
  },
};
