import { describe, expect, it } from "vitest";
import { toIso } from "@/lib/time";
import { parseCodexUsage } from "@/lib/codex";
import { parseCursorUsage } from "@/lib/cursor";
import { parseGrokUsage } from "@/lib/grokbot";
import { diffText, keyHint, parseOpenCodeUsage } from "@/lib/opencode";
import { pickToken, planNameOf, windowOf } from "@/lib/claude";

describe("toIso", () => {
  it("handles epoch seconds, ms, numeric strings and ISO", () => {
    expect(toIso(1_700_000_000)).toBe("2023-11-14T22:13:20.000Z");
    expect(toIso(1_700_000_000_000)).toBe("2023-11-14T22:13:20.000Z");
    expect(toIso("1700000000")).toBe("2023-11-14T22:13:20.000Z");
    expect(toIso("2026-01-02T03:04:05Z")).toBe("2026-01-02T03:04:05.000Z");
  });
  it("returns null instead of throwing on garbage", () => {
    expect(toIso("not a date")).toBeNull();
    expect(toIso(NaN)).toBeNull();
    expect(toIso({})).toBeNull();
    expect(toIso("")).toBeNull();
  });
});

describe("parseOpenCodeUsage", () => {
  const w = (percent: number) => ({ status: "ok", percent, resetsAt: "2026-01-01T00:00:00Z" });
  it("parses a complete payload", () => {
    const u = parseOpenCodeUsage({ usage: { rolling: w(1), weekly: w(2), monthly: w(3) } });
    expect(u?.monthly.percent).toBe(3);
  });
  it("rejects malformed payloads", () => {
    expect(parseOpenCodeUsage({ usage: { rolling: w(1), weekly: w(2) } })).toBeNull();
    expect(
      parseOpenCodeUsage({ usage: { rolling: w(1), weekly: w(2), monthly: { percent: "x" } } }),
    ).toBeNull();
    expect(parseOpenCodeUsage(null)).toBeNull();
  });
  it("describes what changed", () => {
    const a = { rolling: w(1), weekly: w(2), monthly: w(3) };
    const b = { rolling: w(5), weekly: w(2), monthly: w(4) };
    expect(diffText(a, b)).toBe("rolling 1% → 5%, monthly 3% → 4%");
  });
});

describe("keyHint", () => {
  it("reveals only the prefix and last four characters", () => {
    expect(keyHint("sk-abcdefghijklmnop1234")).toBe("sk-…1234");
    expect(keyHint("sk-short")).toBe("•••");
  });
});

describe("parseCursorUsage", () => {
  it("maps plan usage, billing end and plan name", () => {
    const u = parseCursorUsage(
      {
        planUsage: { autoPercentUsed: 86, apiPercentUsed: 54, totalPercentUsed: 70 },
        billingCycleEnd: "1767225600000",
      },
      { planInfo: { planName: "Pro" } },
      "me@x.com",
    );
    expect(u).toMatchObject({
      autoPercentUsed: 86,
      apiPercentUsed: 54,
      totalPercentUsed: 70,
      billingCycleEnd: "2026-01-01T00:00:00.000Z",
      accountName: "me@x.com",
      planName: "Pro",
    });
  });
  it("tolerates a missing plan response and odd types", () => {
    const u = parseCursorUsage({ planUsage: { autoPercentUsed: "86" } }, null, null);
    expect(u.autoPercentUsed).toBeNull();
    expect(u.planName).toBeNull();
  });
});

describe("parseGrokUsage", () => {
  it("maps sand usage and keeps the account attribution", () => {
    const g = parseGrokUsage(
      { usagePercent: 12.5, nextResetTimestampUtc: "bogus", grokPlanLabel: "SuperGrok" },
      { email: "g@x.com", name: "G", source: "cursor" },
    );
    expect(g).toMatchObject({
      percentUsed: 12.5,
      resetAt: null,
      planLabel: "SuperGrok",
      source: "cursor",
    });
  });
});

describe("parseCodexUsage", () => {
  it("maps the primary window", () => {
    const c = parseCodexUsage({
      email: "c@x.com",
      plan_type: "plus",
      rate_limit: {
        limit_reached: true,
        primary_window: { used_percent: 99, limit_window_seconds: 18000, reset_at: 1_700_000_000 },
      },
      credits: { balance: "3.5" },
      rate_limit_reset_credits: { available_count: 2 },
    });
    expect(c).toEqual({
      email: "c@x.com",
      planType: "plus",
      usedPercent: 99,
      limitWindowSeconds: 18000,
      resetAt: "2023-11-14T22:13:20.000Z",
      creditsBalance: 3.5,
      resetCredits: 2,
      limitReached: true,
    });
  });
  it("never throws on an empty payload", () => {
    expect(parseCodexUsage({}).usedPercent).toBeNull();
    expect(parseCodexUsage(null).limitReached).toBe(false);
  });
});

describe("claude helpers", () => {
  it("windowOf reads utilization and epoch/ISO resets", () => {
    expect(windowOf({ utilization: 31, resets_at: 1_700_000_000 })).toEqual({
      percent: 31,
      resetsAt: "2023-11-14T22:13:20.000Z",
    });
    expect(windowOf(null)).toEqual({ percent: null, resetsAt: null });
  });
  it("planNameOf maps subscription types", () => {
    expect(planNameOf("claude_max", null)).toBe("Max");
    expect(planNameOf(null, "claude_pro")).toBe("Pro");
    expect(planNameOf(null, null)).toBeNull();
  });
  it("pickToken prefers live Claude Code tokens, then any live, then expired", () => {
    const past = Date.now() - 1000;
    const future = Date.now() + 3_600_000;
    expect(
      pickToken({
        other: { token: "o", expiresAt: future },
        x_claude_code: { token: "c", expiresAt: future },
      })?.token,
    ).toBe("c");
    expect(
      pickToken({
        other: { token: "o", expiresAt: future },
        x_claude_code: { token: "c", expiresAt: past },
      })?.token,
    ).toBe("o");
    expect(pickToken({ x_claude_code: { token: "c", expiresAt: past } })).toMatchObject({
      token: "c",
      expired: true,
    });
    expect(pickToken({})).toBeNull();
  });
});
