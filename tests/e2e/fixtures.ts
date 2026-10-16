import type { Page, Route } from "@playwright/test";
import type {
  ClaudeUsage,
  CodexUsage,
  CursorUsage,
  GrokUsage,
  OpenCodeAccountSummary,
  OpenCodeAccountUsage,
  Settings,
  TrackedAccount,
} from "../../lib/types";

export const NOW = new Date("2026-10-12T00:57:00Z"); // a Monday, 3 minutes before peak
const iso = (msFromNow: number) => new Date(NOW.getTime() + msFromNow).toISOString();
const H = 3_600_000;

export const settings: Settings = {
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
  };
}

export const usage: OpenCodeAccountUsage[] = [
  account("main@example.com", 2, 10, 85, { lastChangeAt: NOW.getTime() - 60_000 }),
  account("spare@example.com", 0, 100, 40),
  account("hidden@example.com", 5, 5, 5, { enabled: false }),
  account("lapsed@example.com", 0, 0, 0, { subscriptionActive: false }),
];

export const accounts: OpenCodeAccountSummary[] = usage.map((u) => ({
  email: u.email,
  keyHint: "sk-…ab12",
}));

function tracked<T>(
  id: string,
  label: string,
  usage: T,
  extra: Partial<TrackedAccount<T>> = {},
): { accounts: TrackedAccount<T>[] } {
  return {
    accounts: [
      {
        id,
        label,
        active: true,
        usage,
        stale: false,
        cachedAt: null,
        authExpired: false,
        ...extra,
      },
    ],
  };
}

export const providers = {
  cursor: tracked<CursorUsage>("dev@example.com", "dev@example.com", {
    autoPercentUsed: 86,
    apiPercentUsed: 54,
    totalPercentUsed: 70,
    billingCycleEnd: iso(10 * 24 * H),
    totalSpend: null,
    limit: null,
    bonusSpend: null,
    accountName: "dev@example.com",
    planName: "Pro",
  }),
  grok: tracked<GrokUsage>(
    "dev@example.com",
    "dev@example.com",
    {
      email: "dev@example.com",
      name: null,
      percentUsed: 40,
      resetAt: iso(3 * 24 * H),
      planLabel: "SuperGrok",
      cursorPlanName: null,
      source: "grok-bot",
    },
    { stale: true, cachedAt: NOW.getTime() - 3_600_000 },
  ),
  codex: tracked<CodexUsage>("c@example.com", "c@example.com", {
    email: "c@example.com",
    planType: "plus",
    usedPercent: 100,
    limitWindowSeconds: 18000,
    resetAt: iso(2 * H),
    creditsBalance: null,
    resetCredits: 1,
    limitReached: true,
  }),
  claude: tracked<ClaudeUsage>(
    "me@example.com",
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
    },
    { authExpired: true },
  ),
};

export interface ApiOverrides {
  settings?: Partial<Settings>;
  providers?: Partial<typeof providers>;
  fail?: string[]; // route names that should return 500
  onPost?: (path: string, body: unknown) => void;
}

/** Mock every /api route; returns the requests seen, for assertions. */
export async function mockApi(page: Page, o: ApiOverrides = {}) {
  const seen: { method: string; path: string; body: unknown; secret: string | null }[] = [];
  let current: Settings = { ...settings, ...o.settings };
  const payloads = { ...providers, ...o.providers };
  await page.route("**/api/**", async (route: Route) => {
    const req = route.request();
    const url = new URL(req.url());
    const name = url.pathname.replace("/api/", "");
    const body = req.postData() ? JSON.parse(req.postData()!) : null;
    seen.push({
      method: req.method(),
      path: url.pathname + url.search,
      body,
      secret: req.headers()["x-usage-secret"] ?? null,
    });
    if (o.fail?.includes(name)) return route.fulfill({ status: 500, body: "boom" });
    if (req.method() !== "GET") {
      o.onPost?.(name, body);
      if (name === "settings") {
        current = {
          ...current,
          ...body,
          displayNames: { ...current.displayNames, ...(body?.displayNames ?? {}) },
        };
        return route.fulfill({ json: { ok: true, settings: current } });
      }
      return route.fulfill({ json: { ok: true } });
    }
    switch (name) {
      case "settings":
        return route.fulfill({ json: { settings: current } });
      case "usage":
        return route.fulfill({ json: { accounts: usage } });
      case "accounts":
        return route.fulfill({ json: { accounts } });
      case "key":
        return route.fulfill({ json: { key: "sk-test-key-123" } });
      default:
        if (name in payloads)
          return route.fulfill({ json: payloads[name as keyof typeof payloads] });
        return route.fulfill({ status: 404, json: { error: "not found" } });
    }
  });
  return seen;
}

/** Stub the Electron preload bridge and freeze time. */
export async function setupWidget(page: Page) {
  await page.clock.setFixedTime(NOW);
  await page.addInitScript(() => {
    const listeners: Record<string, (() => void)[]> = {};
    const on = (name: string) => (cb: () => void) => {
      (listeners[name] ??= []).push(cb);
      return () => {};
    };
    const w = window as unknown as Record<string, unknown>;
    w.__emit = (name: string) => listeners[name]?.forEach((cb) => cb());
    w.widget = {
      secret: "test-secret",
      resize: () => {},
      onBg: () => () => {},
      onToggleExpand: on("expand"),
      onRefresh: on("refresh"),
      onSettingsChanged: on("settings"),
      onServerStatus: on("server"),
      menuAction: () => {},
      startDrag: () => {},
      moveDrag: () => {},
      endDrag: () => {},
    };
    w.settingsWindow = { secret: "test-secret", close: () => (w.__closed = true) };
  });
}
