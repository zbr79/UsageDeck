import { describe, expect, it } from "vitest";
import type {
  ClaudeUsage,
  CodexUsage,
  CursorUsage,
  GrokUsage,
  OpenCodeAccountUsage,
  TrackedAccount,
} from "@/lib/types";
import {
  claudeRow,
  codexRow,
  cursorRow,
  fixedColumns,
  grokRow,
  openCodeRow,
  tableColumns,
} from "@/settings/_lib/table-rows";

const NOW = Date.parse("2026-11-13T10:00:00Z");
const iso = (h: number) => new Date(NOW + h * 3_600_000).toISOString();

const tracked = <T>(
  usage: T | null,
  extra: Partial<TrackedAccount<T>> = {},
): TrackedAccount<T> => ({
  id: "a",
  label: "a@example.com",
  active: true,
  usage,
  stale: false,
  cachedAt: null,
  authExpired: false,
  error: null,
  ...extra,
});

const claude = (over: Partial<ClaudeUsage> = {}): ClaudeUsage => ({
  email: "a@example.com",
  displayName: null,
  signedIn: true,
  subscribed: true,
  subscriptionType: "max",
  planName: "Ultra",
  fiveHour: { percent: 15, resetsAt: iso(3.5) },
  weekly: { percent: 55, resetsAt: iso(24 * 5 + 9) },
  weeklyOpus: null,
  weeklySonnet: null,
  weeklyScoped: [
    {
      label: "Fable",
      percent: 100,
      resetsAt: iso(24 * 5 + 9),
      sharePctOfWeekly: null,
      allowancePctOfWeekly: null,
    },
  ],
  resetCredits: null,
  resets: null,
  billingType: null,
  rateLimitTier: null,
  orgUuid: null,
  ...over,
});

describe("tableColumns", () => {
  it("adds the model's weekly column when any account has one, minus hidden bars", () => {
    const accounts = [tracked(claude({ weeklyScoped: [] })), tracked(claude())];
    expect(tableColumns("claude", accounts, {}).map((c) => c.name)).toEqual([
      "5-hour",
      "Weekly",
      "Fable weekly",
    ]);
    expect(tableColumns("claude", accounts, { claude: ["weekly"] }).map((c) => c.id)).toEqual([
      "fiveHour",
      "scoped",
    ]);
    expect(tableColumns("claude", [tracked(claude({ weeklyScoped: [] }))], {})).toHaveLength(2);
  });

  it("gives Codex its weekly column only when Codex sends it", () => {
    const codex = (weekly: number | null) =>
      tracked({ usedPercent: 10, weeklyUsedPercent: weekly } as CodexUsage);
    expect(tableColumns("codex", [codex(null)], {}).map((c) => c.id)).toEqual(["fiveHour"]);
    expect(tableColumns("codex", [codex(40)], {}).map((c) => c.id)).toEqual(["fiveHour", "weekly"]);
  });
});

describe("claudeRow", () => {
  it("has each window's percent and countdown, the plan, and no note", () => {
    const row = claudeRow(tracked(claude()), NOW);
    expect(row.note).toBeNull();
    expect(row.plan).toEqual({ label: "Ultra", tone: "amber" });
    expect(row.cells).toEqual({
      fiveHour: { percent: 15, reset: "3h 30m" },
      weekly: { percent: 55, reset: "5d 9h" },
      scoped: { percent: 100, reset: "5d 9h" },
    });
  });

  it("drops a cached window whose reset passed, and names a rejected sign-in", () => {
    const row = claudeRow(
      tracked(claude({ fiveHour: { percent: 92, resetsAt: iso(-1) } }), {
        stale: true,
        authExpired: true,
      }),
      NOW,
    );
    expect(row.cells.fiveHour).toEqual({ percent: null, reset: null });
    expect(row.warning?.text).toBe("re-auth");
  });

  it("is a note without usage or sign-in", () => {
    expect(claudeRow(tracked<ClaudeUsage>(null), NOW).note).toBe("Not signed in or unavailable");
    expect(claudeRow(tracked(claude({ signedIn: false })), NOW).note).toBe("Not signed in");
  });
});

describe("codexRow", () => {
  it("capitalizes the plan and reads both windows", () => {
    const row = codexRow(
      tracked({
        planType: "plus",
        usedPercent: 100,
        resetAt: iso(1),
        weeklyUsedPercent: 64,
        weeklyResetAt: iso(24 * 3),
        resetCredits: 1,
      } as CodexUsage),
      NOW,
    );
    expect(row.plan).toEqual({ label: "Plus", tone: "blue" });
    expect(row.cells.fiveHour).toEqual({ percent: 100, reset: "1h 0m" });
    expect(row.extra).toEqual({ text: "1", title: "Reset: 1 left", icon: "reset" });
  });
});

describe("cursorRow", () => {
  const cursor = (over: Partial<CursorUsage> = {}): CursorUsage => ({
    cursorModelsPercentUsed: 86,
    otherModelsPercentUsed: 54,
    resetsAt: iso(24 * 9),
    onDemand: null,
    accountName: "dev@example.com",
    planName: "Pro",
    apiSpend: { spentUsd: 10.8, limitUsd: 20, byModel: [{ model: "gpt-5", usd: 10.8 }] },
    ...over,
  });

  it("gives both buckets the shared reset and puts API dollars in the last column", () => {
    const row = cursorRow(tracked(cursor()), NOW, null);
    expect(row.cells).toEqual({
      firstParty: { percent: 86, reset: "9d" },
      api: { percent: 54, reset: "9d" },
    });
    expect(row.extra).toEqual({ text: "$10.80 / $20", title: "gpt-5 $10.80" });
    expect(row.plan).toEqual({ label: "Pro", tone: "blue" });
  });

  it("grays a rejected sign-in and drops a passed cycle", () => {
    const out = cursorRow(tracked(cursor(), { authExpired: true }), NOW, null);
    expect(out.dimmed).toBe(true);
    expect(out.warning?.text).toBe("re-auth");
    const passed = cursorRow(tracked(cursor({ resetsAt: iso(-1) }), { stale: true }), NOW, null);
    expect(passed.cells.api).toEqual({ percent: null, reset: null });
    expect(passed.extra).toEqual({ text: "—", muted: true });
  });
});

describe("grokRow", () => {
  it("has one weekly cell and the plan", () => {
    const row = grokRow(
      tracked({
        email: "g@example.com",
        name: null,
        percentUsed: 40,
        resetAt: iso(24 * 2 + 23),
        planLabel: "SuperGrok",
        cursorPlanName: null,
        source: "grok-bot",
      } satisfies GrokUsage),
      NOW,
    );
    expect(row.cells).toEqual({ weekly: { percent: 40, reset: "2d 23h" } });
    expect(row.plan?.label).toBe("SuperGrok");
    expect(row.extra).toBeUndefined();
  });
});

describe("openCodeRow", () => {
  const win = (percent: number, h: number) => ({ status: "ok", percent, resetsAt: iso(h) });
  const oc = (over: Partial<OpenCodeAccountUsage> = {}): OpenCodeAccountUsage => ({
    email: "o@example.com",
    usage: { rolling: win(2, 4), weekly: win(10, 24 * 5), monthly: win(85, 24 * 14) },
    prev: null,
    enabled: true,
    error: null,
    stale: false,
    cachedAt: null,
    subscriptionActive: true,
    lastChangeAt: null,
    lastChangeDetail: null,
    ...over,
  });

  it("reads the three windows under the Go plan", () => {
    const row = openCodeRow(oc(), NOW);
    expect(row.cells.monthly).toEqual({ percent: 85, reset: "14d" });
    expect(row.plan).toEqual({ label: "Go", tone: "blue" });
    expect(row.dimmed).toBe(false);
  });

  it("grays a lapsed subscription and names a rejected key first", () => {
    expect(openCodeRow(oc({ subscriptionActive: false }), NOW)).toMatchObject({
      dimmed: true,
      warning: { text: "not active" },
    });
    expect(openCodeRow(oc({ authExpired: true }), NOW).warning?.text).toBe("key rejected");
    expect(openCodeRow(oc({ usage: null }), NOW).note).toBe("Unavailable");
  });
});

describe("fixedColumns", () => {
  it("lists every bar minus the hidden ones", () => {
    expect(fixedColumns("opencode", { opencode: ["rolling"] }).map((c) => c.name)).toEqual([
      "Weekly",
      "Monthly",
    ]);
    expect(fixedColumns("cursor", {}).map((c) => c.id)).toEqual(["firstParty", "api"]);
  });
});
