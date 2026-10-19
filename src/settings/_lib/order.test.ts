import { describe, expect, it } from "vitest";
import type { ClaudeUsage, CursorUsage, OpenCodeAccountUsage, TrackedAccount } from "@/lib/types";
import { isUsedUp, orderOpenCode, orderTracked } from "./order";

const D = 24 * 3_600_000;
const NOW = Date.parse("2026-10-12T00:00:00Z");
const win = (percent: number, resetsIn = D) => ({
  status: "ok",
  percent,
  resetsAt: new Date(NOW + resetsIn).toISOString(),
});
function row(
  email: string,
  weekly: number,
  monthly: number,
  extra: Partial<OpenCodeAccountUsage> = {},
): OpenCodeAccountUsage {
  return {
    email,
    usage: { rolling: win(0), weekly: win(weekly, 2 * D), monthly: win(monthly, 10 * D) },
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

describe("orderOpenCode", () => {
  it("widget order first, then hidden, lapsed and no-data accounts", () => {
    const rows = [
      row("nodata", 0, 0, { usage: null }),
      row("lapsed", 0, 0, { subscriptionActive: false }),
      row("hidden-full", 100, 0),
      row("hidden", 0, 0, { lastChangeAt: 5 }),
      row("old", 10, 10, { lastChangeAt: 1 }),
      row("late", 0, 100),
      row("soon", 100, 0),
      row("recent", 10, 10, { lastChangeAt: 9 }),
      row("off", 0, 0, { enabled: false }),
    ];
    const hidden = (e: string) => e === "hidden" || e === "hidden-full";
    expect(orderOpenCode(rows, hidden, NOW).map((r) => r.email)).toEqual([
      "recent",
      "old",
      "soon",
      "late",
      "hidden",
      "off",
      "hidden-full",
      "lapsed",
      "nodata",
    ]);
  });

  it("keeps every account exactly once and returns the original rows", () => {
    const rows = [row("a", 0, 0, { enabled: false }), row("b", 0, 0, { usage: null })];
    const out = orderOpenCode(rows, () => false, NOW);
    expect(out).toHaveLength(2);
    expect(out[0]).toBe(rows[0]);
  });
});

function tracked<T>(id: string, usage: T | null, extra: Partial<TrackedAccount<T>> = {}) {
  return {
    id,
    label: id,
    active: false,
    usage,
    stale: false,
    cachedAt: null,
    authExpired: false,
    ...extra,
  } as TrackedAccount<T>;
}
const claude = (five: number, weekly: number) =>
  ({
    fiveHour: { percent: five, resetsAt: new Date(NOW + D).toISOString() },
    weekly: { percent: weekly, resetsAt: new Date(NOW + 2 * D).toISOString() },
  }) as ClaudeUsage;

describe("orderTracked", () => {
  it("signed-in first, used-up last, then most recent cached use; hidden after the rest", () => {
    const accounts = [
      tracked("old", claude(10, 10), { cachedAt: 1 }),
      tracked("full", claude(100, 10), { cachedAt: 9 }),
      tracked("hidden", claude(0, 0), { cachedAt: 10 }),
      tracked("recent", claude(10, 10), { cachedAt: 5 }),
      tracked("me", claude(100, 100), { active: true }),
    ];
    const out = orderTracked("claude", accounts, (id) => id === "hidden", NOW);
    expect(out.map((a) => a.id)).toEqual(["me", "recent", "old", "full", "hidden"]);
  });

  it("a stale window that already reset is not used up", () => {
    const u = claude(100, 10);
    u.fiveHour!.resetsAt = new Date(NOW - 1000).toISOString();
    expect(isUsedUp("claude", tracked("x", u, { stale: true }), NOW)).toBe(false);
    expect(isUsedUp("claude", tracked("x", u), NOW)).toBe(true);
  });

  it("cursor is used up at 100 % of its cycle", () => {
    const c = { totalPercentUsed: 100, billingCycleEnd: null } as unknown as CursorUsage;
    expect(isUsedUp("cursor", tracked("c", c), NOW)).toBe(true);
  });
});
