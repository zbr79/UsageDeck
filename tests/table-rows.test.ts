import { describe, expect, it } from "vitest";
import type { ClaudeUsage, CodexUsage, TrackedAccount } from "@/lib/types";
import { claudeRow, codexRow, tableColumns } from "@/settings/_lib/table-rows";

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
    expect(row.plan).toBe("Ultra");
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
    expect(row.plan).toBe("Plus");
    expect(row.cells.fiveHour).toEqual({ percent: 100, reset: "1h 0m" });
    expect(row.resets.map((r) => r.count)).toEqual([1]);
  });
});
