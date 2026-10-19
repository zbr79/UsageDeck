import { describe, expect, it } from "vitest";
import { rankAccounts, usableAgainAt } from "./ranking";
import type { OpenCodeAccountUsage, UsageWindow } from "./types";

const NOW = Date.parse("2026-10-18T12:00:00.000Z");
const H = 3_600_000;

function win(percent: number, resetInMs: number): UsageWindow {
  return { status: "ok", percent, resetsAt: new Date(NOW + resetInMs).toISOString() };
}

function row(email: string, weekly: UsageWindow, monthly: UsageWindow): OpenCodeAccountUsage {
  return {
    email,
    usage: { rolling: win(0, H), weekly, monthly },
    prev: null,
    enabled: true,
    error: null,
    stale: false,
    cachedAt: null,
    subscriptionActive: true,
    lastChangeAt: null,
    lastChangeDetail: null,
  };
}

describe("usableAgainAt", () => {
  it("waits for the latest reset when weekly and monthly are both exhausted", () => {
    // weekly resets in 1 day, monthly in 10 days: usable again only after 10 days
    const both = row("both@example.com", win(100, 24 * H), win(100, 240 * H));
    expect(usableAgainAt(both, NOW)).toBe(NOW + 240 * H);

    // weekly-only exhausted, resetting in 3 days, is usable sooner and ranks first
    const weeklyOnly = row("weekly@example.com", win(100, 72 * H), win(50, 240 * H));
    const ranked = rankAccounts([both, weeklyOnly], () => 0, NOW).map((r) => r.email);
    expect(ranked).toEqual(["weekly@example.com", "both@example.com"]);
  });
});
