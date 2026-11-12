import { describe, expect, it } from "vitest";
import type {
  ClaudeUsage,
  CodexUsage,
  CursorUsage,
  GrokUsage,
  OpenCodeAccountUsage,
} from "./types";
import type { HiddenBars } from "./bars";
import { isSnoozed, openCodeUsedUp, usedUp } from "./used-up";

const NOW = Date.parse("2026-11-10T12:00:00.000Z");
const M = 60_000;
const H = 60 * M;
const at = (ms: number) => new Date(NOW + ms).toISOString();
/** every bar on, the default */
const ALL: HiddenBars = {};

const codex = (five: number, weekly: number | null, extra: Partial<CodexUsage> = {}) =>
  ({
    usedPercent: five,
    limitWindowSeconds: 18000,
    resetAt: at(81 * M),
    weeklyUsedPercent: weekly,
    weeklyResetAt: weekly == null ? null : at(3 * 24 * H),
    limitReached: false,
    ...extra,
  }) as CodexUsage;

const claude = (five: number | null, weekly: number | null, fiveResetIn = 2 * H) =>
  ({
    fiveHour: { percent: five, resetsAt: at(fiveResetIn) },
    weekly: { percent: weekly, resetsAt: at(30 * H) },
    weeklyScoped: [] as ClaudeUsage["weeklyScoped"],
  }) as ClaudeUsage;

const fresh = <T>(usage: T | null) => ({ usage, stale: false });
const cached = <T>(usage: T | null) => ({ usage, stale: true });

describe("usedUp", () => {
  it("5-hour window full: usable again at its reset", () => {
    expect(usedUp("codex", fresh(codex(100, 40)), NOW, ALL)).toEqual({ until: NOW + 81 * M });
    expect(usedUp("claude", fresh(claude(100, 40)), NOW, ALL)).toEqual({ until: NOW + 2 * H });
  });

  it("weekly window full: usable again at the weekly reset", () => {
    expect(usedUp("codex", fresh(codex(20, 100)), NOW, ALL)).toEqual({ until: NOW + 72 * H });
    expect(usedUp("claude", fresh(claude(10, 100)), NOW, ALL)).toEqual({ until: NOW + 30 * H });
  });

  it("both windows full: waits for the later reset, not the 5-hour one", () => {
    expect(usedUp("codex", fresh(codex(100, 100)), NOW, ALL)).toEqual({ until: NOW + 72 * H });
    expect(usedUp("claude", fresh(claude(100, 100)), NOW, ALL)).toEqual({ until: NOW + 30 * H });
  });

  it("claude: a full Fable limit leaves the card usable, since the other models still work", () => {
    const fableFull = {
      ...claude(30, 40),
      weeklyScoped: [
        {
          label: "Fable",
          percent: 100,
          resetsAt: at(30 * H),
          sharePctOfWeekly: 50,
          allowancePctOfWeekly: 50,
        },
      ],
    };
    expect(usedUp("claude", fresh(fableFull), NOW, ALL)).toBeNull();
  });

  it("Codex's limit flag counts as full below 100 %", () => {
    const flagged = codex(97, null, { limitReached: true });
    expect(usedUp("codex", fresh(flagged), NOW, ALL)).toEqual({ until: NOW + 81 * M });
  });

  it("cached data whose full window already reset is usable; fresh data still counts", () => {
    const passed = claude(100, 40, -M);
    expect(usedUp("claude", cached(passed), NOW, ALL)).toBeNull();
    expect(usedUp("claude", fresh(passed), NOW, ALL)).toEqual({ until: NOW - M });
    const codexPassed = codex(100, 40, { resetAt: at(-M) });
    expect(usedUp("codex", cached(codexPassed), NOW, ALL)).toBeNull();
  });

  it("not full, unknown, or no data: usable", () => {
    expect(usedUp("codex", fresh(codex(99, 99)), NOW, ALL)).toBeNull();
    expect(usedUp("claude", fresh(claude(null, null)), NOW, ALL)).toBeNull();
    expect(usedUp("claude", fresh(null), NOW, ALL)).toBeNull();
  });

  it("cursor: every included bucket full and no on-demand budget left", () => {
    const full = {
      cursorModelsPercentUsed: 100,
      otherModelsPercentUsed: 100,
      resetsAt: at(5 * 24 * H),
      onDemand: { state: "disabled" },
    } as CursorUsage;
    const usable = (c: Partial<CursorUsage>) =>
      usedUp("cursor", fresh({ ...full, ...c }), NOW, ALL) === null;
    expect(usedUp("cursor", fresh(full), NOW, ALL)).toEqual({ until: NOW + 5 * 24 * H });
    expect(usable({ otherModelsPercentUsed: 99 }), "Other Models left").toBe(true);
    expect(usable({ cursorModelsPercentUsed: 40 }), "Cursor Models left").toBe(true);
    expect(usable({ otherModelsPercentUsed: null }), "one bucket reported, full").toBe(false);
    expect(usable({ onDemand: null }), "on-demand unknown").toBe(false);
    const enabled = (spentUsd: number, limitUsd: number | null) =>
      ({ state: "enabled", spentUsd, limitUsd }) as const;
    expect(usable({ onDemand: enabled(12, 50) }), "on-demand budget left").toBe(true);
    expect(usable({ onDemand: enabled(900, null) }), "on-demand without a limit").toBe(true);
    expect(usable({ onDemand: enabled(50, 50) }), "on-demand limit spent").toBe(false);
    expect(
      usable({ cursorModelsPercentUsed: null, otherModelsPercentUsed: null }),
      "nothing reported",
    ).toBe(true);
  });

  it("grok: one window", () => {
    const grok = { percentUsed: 100, resetAt: at(H) } as GrokUsage;
    expect(usedUp("grok", fresh(grok), NOW, ALL)).toEqual({ until: NOW + H });
    expect(usedUp("grok", fresh({ ...grok, percentUsed: 50 }), NOW, ALL)).toBeNull();
  });

  it("full with no known reset time: used up, but no time to wait for", () => {
    const cursor = {
      cursorModelsPercentUsed: 100,
      otherModelsPercentUsed: 100,
      resetsAt: null,
      onDemand: null,
    } as CursorUsage;
    expect(usedUp("cursor", fresh(cursor), NOW, ALL)).toEqual({ until: null });
  });
});

describe("usedUp with bars turned off", () => {
  it("a full bar the card does not show leaves it usable", () => {
    expect(usedUp("codex", fresh(codex(100, 40)), NOW, { codex: ["fiveHour"] })).toBeNull();
    expect(usedUp("claude", fresh(claude(10, 100)), NOW, { claude: ["weekly"] })).toBeNull();
    const cursorApiFull = {
      cursorModelsPercentUsed: 40,
      otherModelsPercentUsed: 100,
      resetsAt: at(H),
      onDemand: null,
    } as CursorUsage;
    expect(usedUp("cursor", fresh(cursorApiFull), NOW, { cursor: ["firstParty"] })).toEqual({
      until: NOW + H,
    });
    expect(usedUp("cursor", fresh(cursorApiFull), NOW, { cursor: ["api"] })).toBeNull();
  });

  it("the full bars still shown count, waiting only for their resets", () => {
    expect(usedUp("codex", fresh(codex(100, 100)), NOW, { codex: ["weekly"] })).toEqual({
      until: NOW + 81 * M,
    });
    expect(usedUp("claude", fresh(claude(100, 100)), NOW, { claude: ["weekly"] })).toEqual({
      until: NOW + 2 * H,
    });
  });

  it("a card left with no shown bar it has data for shows, and counts, what it has", () => {
    // Codex without a weekly window: hiding its 5-hour bar cannot leave the card empty.
    expect(usedUp("codex", fresh(codex(100, null)), NOW, { codex: ["fiveHour"] })).toEqual({
      until: NOW + 81 * M,
    });
    // Claude with only its per-model bar on, and no per-model limit in the data.
    expect(
      usedUp("claude", fresh(claude(100, 10)), NOW, { claude: ["fiveHour", "weekly"] }),
    ).toEqual({ until: NOW + 2 * H });
  });
});

describe("openCodeUsedUp", () => {
  const win = (percent: number, resetIn: number) => ({
    status: "ok",
    percent,
    resetsAt: at(resetIn),
  });
  const row = (weekly: number, monthly: number, stale = false) =>
    ({
      email: "a@example.com",
      usage: { rolling: win(100, H), weekly: win(weekly, 24 * H), monthly: win(monthly, 240 * H) },
      stale,
    }) as OpenCodeAccountUsage;

  it("weekly or monthly full; both full waits for the monthly reset", () => {
    expect(openCodeUsedUp(row(100, 10), NOW, ALL)).toEqual({ until: NOW + 24 * H });
    expect(openCodeUsedUp(row(100, 100), NOW, ALL)).toEqual({ until: NOW + 240 * H });
  });

  it("a full rolling window alone does not stop an OpenCode account", () => {
    expect(openCodeUsedUp(row(10, 10), NOW, ALL)).toBeNull();
  });

  it("counts only the weekly and monthly bars the card shows", () => {
    expect(openCodeUsedUp(row(10, 100), NOW, { opencode: ["monthly"] })).toBeNull();
    expect(openCodeUsedUp(row(100, 100), NOW, { opencode: ["monthly"] })).toEqual({
      until: NOW + 24 * H,
    });
    expect(openCodeUsedUp(row(100, 10), NOW, { opencode: ["rolling"] })).toEqual({
      until: NOW + 24 * H,
    });
  });
});

describe("isSnoozed", () => {
  const used = { until: NOW + H };
  const snoozes = { "codex:c@example.com": NOW + H };

  it("hidden until the time it was hidden for", () => {
    expect(isSnoozed(snoozes, "codex:c@example.com", used, NOW)).toBe(true);
    expect(isSnoozed(snoozes, "codex:c@example.com", used, NOW + H - 1)).toBe(true);
    expect(isSnoozed(snoozes, "codex:c@example.com", used, NOW + H)).toBe(false);
  });

  it("back early once the account is usable again", () => {
    expect(isSnoozed(snoozes, "codex:c@example.com", null, NOW)).toBe(false);
  });

  it("only the account that was hidden", () => {
    expect(isSnoozed(snoozes, "claude:c@example.com", used, NOW)).toBe(false);
    expect(isSnoozed(snoozes, "constructor", used, NOW)).toBe(false);
  });
});
