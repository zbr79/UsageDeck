// Browser-only mock of the Rust `api` command, so `npm run dev` can be viewed
// in a normal browser. Data adapted from tests/e2e/fixtures.ts, with times
// relative to the page load instead of a frozen clock.
import type {
  ClaudeUsage,
  CodexUsage,
  CursorUsage,
  GrokUsage,
  OpenCodeAccountSummary,
  OpenCodeAccountUsage,
  Settings,
  TrackedAccount,
} from "./types";

const NOW = Date.now();
const H = 3_600_000;
const iso = (msFromNow: number) => new Date(NOW + msFromNow).toISOString();

let settings: Settings = {
  opencodeEnabled: true,
  cursorEnabled: true,
  grokEnabled: true,
  codexEnabled: true,
  claudeEnabled: true,
  deepseekPeakHourWarning: true,
  showProviderNames: false,
  themeMode: "dark",
  panelTheme: "dark",
  displayNames: { "main@example.com": "Main", "spare@example.com": "Spare" },
  disabledAccounts: ["hidden@example.com"],
  providerOrder: ["opencode", "cursor", "grok", "codex", "claude"],
  startAtLogin: true,
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
  usage.map((u) => ({ email: u.email, keyHint: "sk-…ab12" }) as OpenCodeAccountSummary);

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
      autoPercentUsed: 86,
      apiPercentUsed: 54,
      totalPercentUsed: 70,
      billingCycleEnd: iso(10 * 24 * H),
      totalSpend: null,
      limit: null,
      bonusSpend: null,
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
      resetAt: iso(2 * H),
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
      planName: "Max 20x",
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

// `?panel=light` shows the settings panel in its light theme (browser preview only).
if (typeof location !== "undefined") {
  const panel = new URLSearchParams(location.search).get("panel");
  if (panel === "light" || panel === "dark") settings = { ...settings, panelTheme: panel };
}

/** Browser stand-in for the Rust `copy_key` command: fails for an unknown account, like Rust. */
export async function mockCopyKey(email: string): Promise<void> {
  if (!usage.some((u) => u.email === email.toLowerCase())) throw new Error("unknown account");
  await navigator.clipboard?.writeText("sk-test-key-123");
}

type Reply = { status: number; body: unknown };
const json = (body: unknown, status = 200): Reply => ({ status, body });

export async function mockApi(method: string, path: string, body: unknown): Promise<Reply> {
  const url = new URL(path, "http://mock.local");
  const name = url.pathname.replace(/^\/api\//, "");
  const b = (body ?? {}) as Record<string, unknown>;
  if (method !== "GET") {
    if (name === "settings") {
      settings = {
        ...settings,
        ...(b as Partial<Settings>),
        displayNames: {
          ...settings.displayNames,
          ...((b.displayNames as Record<string, string>) ?? {}),
        },
      };
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
    default:
      if (name in providers) return json(providers[name]);
      return json({ error: "not found" }, 404);
  }
}
