import { describe, expect, it } from "vitest";
import {
  calibrationText,
  formatClock,
  formatPctPerHour,
  formatUsd,
  formatUsdRate,
  hourlyOldestFirst,
  hourlySummary,
  modelShares,
  paceLine,
  paceTone,
  paceVsEven,
  pickWindow,
  windowForecast,
} from "@/lib/pace";
import { paceScenario } from "@/lib/mock";
import type { ClaudePace, PaceWindow } from "@/lib/types";

// Local times, so the checks hold in any time zone.
const NOW = new Date(2026, 9, 18, 14, 0).getTime(); // Sun 2:00 PM
const MIN = 60_000;
const H = 60 * MIN;

const win = (over: Partial<PaceWindow> = {}): PaceWindow => ({
  pct: 41,
  pctPerHour: 9,
  evenPacePct: 30,
  etaToLimit: null,
  resetsAt: new Date(NOW + 2 * H).toISOString(),
  state: "lasts",
  ...over,
});

const pace = (over: Partial<ClaudePace["spend"]> = {}, windows?: ClaudePace["windows"]) =>
  ({
    spend: {
      lastHour: 71,
      last5h: 353,
      today: 600,
      last7d: 8723,
      rate60m: 71,
      rate15m: 64,
      byModel: [],
      hourly: [],
      unpricedReplies: 0,
      scannedAt: NOW,
      backfilling: false,
      ...over,
    },
    windows: windows ?? { fiveHour: win(), weekly: win({ pct: 47, pctPerHour: 1 }) },
    calibration: { usdPerWeeklyPct: 185 },
    coverage: "claude-code-this-pc",
  }) satisfies ClaudePace;

describe("formatting", () => {
  it("rounds dollars to one decimal under $10, whole dollars otherwise", () => {
    expect(formatUsd(7.44)).toBe("$7.4");
    expect(formatUsd(0)).toBe("$0.0");
    expect(formatUsd(9.97)).toBe("$10");
    expect(formatUsd(71.4)).toBe("$71");
    expect(formatUsd(1293.2)).toBe("$1,293");
    expect(formatUsdRate(71.4)).toBe("$71/h");
    expect(formatUsdRate(null)).toBe("$—/h");
  });

  it("signs the percent speed", () => {
    expect(formatPctPerHour(9.2)).toBe("+9%/h");
    expect(formatPctPerHour(0.44)).toBe("+0.4%/h");
    expect(formatPctPerHour(-2)).toBe("-2%/h");
    expect(formatPctPerHour(0)).toBe("+0%/h");
  });

  it("shows a local clock time, with a weekday when not today", () => {
    expect(formatClock(new Date(2026, 9, 18, 15, 40).getTime(), NOW)).toBe("3:40 PM");
    expect(formatClock(new Date(2026, 9, 20, 9, 5).getTime(), NOW)).toBe("Tue 9:05 AM");
  });
});

describe("pickWindow", () => {
  it("defaults to the 5-hour window", () => {
    expect(pickWindow(pace().windows)?.key).toBe("fiveHour");
  });
  it("picks the window in state limit", () => {
    const w = { fiveHour: win(), weekly: win({ state: "limit", etaToLimit: NOW + 5 * H }) };
    expect(pickWindow(w)?.key).toBe("weekly");
  });
  it("picks the earliest limit when both are heading there", () => {
    const w = {
      fiveHour: win({ state: "limit", etaToLimit: NOW + 2 * H }),
      weekly: win({ state: "limit", etaToLimit: NOW + 1 * H }),
    };
    expect(pickWindow(w)?.key).toBe("weekly");
    w.weekly.etaToLimit = NOW + 3 * H;
    expect(pickWindow(w)?.key).toBe("fiveHour");
  });
  it("falls back to weekly, then null", () => {
    expect(pickWindow({ fiveHour: null, weekly: win() })?.key).toBe("weekly");
    expect(pickWindow({ fiveHour: null, weekly: null })).toBeNull();
  });
});

describe("paceTone", () => {
  it("is muted unless heading for a limit", () => {
    expect(paceTone(win(), NOW)).toBe("muted");
    expect(paceTone(null, NOW)).toBe("muted");
  });
  it("is amber beyond 30 minutes and red at 30 minutes or less", () => {
    expect(paceTone(win({ state: "limit", etaToLimit: NOW + 31 * MIN }), NOW)).toBe("amber");
    expect(paceTone(win({ state: "limit", etaToLimit: NOW + 30 * MIN }), NOW)).toBe("red");
    expect(paceTone(win({ state: "limit", etaToLimit: NOW + 5 * MIN }), NOW)).toBe("red");
  });
});

describe("paceLine", () => {
  it("reads rate, 5h speed and lasts", () => {
    const line = paceLine(pace(), NOW);
    expect(line.text).toBe("$71/h · 5h +9%/h · lasts");
    expect(line.tone).toBe("muted");
  });
  it("shows the limit time and drops the window label when too long", () => {
    const w = { fiveHour: win({ state: "limit", etaToLimit: NOW + 100 * MIN }), weekly: win() };
    expect(paceLine(pace({}, w), NOW, 99).text).toBe("$71/h · 5h +9%/h · limit 3:40 PM");
    expect(paceLine(pace({}, w), NOW).text).toBe("$71/h · +9%/h · limit 3:40 PM");
    expect(paceLine(pace({}, w), NOW).tone).toBe("amber");
  });
  it("says measuring when the speed is not known yet", () => {
    const w = { fiveHour: win({ state: "measuring", pctPerHour: null }), weekly: win() };
    expect(paceLine(pace({}, w), NOW).text).toBe("$71/h · measuring…");
  });
  it("hides the dollars while backfilling", () => {
    expect(paceLine(pace({ backfilling: true }), NOW).text).toBe("$—/h · 5h +9%/h · lasts");
  });
  it("puts both rates, both windows and the coverage in the tooltip", () => {
    const title = paceLine(pace(), NOW).title;
    expect(title).toContain("$64/h (last 15 min)");
    expect(title).toContain("5-hour: 41% · +9%/h · lasts until reset");
    expect(title).toContain("Weekly: 47% · +1%/h");
    expect(title).toContain("Claude Code on this PC");
  });
});

describe("settings helpers", () => {
  it("compares with even pace", () => {
    expect(paceVsEven(win({ pct: 41, evenPacePct: 30 }))).toBe("ahead of even pace by 11 pts");
    expect(paceVsEven(win({ pct: 20, evenPacePct: 30 }))).toBe("behind even pace by 10 pts");
    expect(paceVsEven(win({ evenPacePct: null }))).toBeNull();
  });
  it("forecasts until reset", () => {
    expect(windowForecast(win(), NOW, true)).toBe("lasts until reset");
  });
  it("orders bars oldest first and summarises peak and current", () => {
    expect(hourlyOldestFirst([3, 2, 1])).toEqual([1, 2, 3]);
    expect(hourlySummary([33, 69, 119, 5])).toBe(
      "Spend per hour over the last 4 hours. Peak $119/h, 2 hours ago. Current hour so far $33.",
    );
  });
  it("splits spend by model with shares", () => {
    const rows = modelShares([
      { model: "claude-sonnet-5-5", usd24h: 25 },
      { model: "claude-opus-5-5", usd24h: 75 },
    ]);
    expect(rows[0]).toEqual({ model: "claude-opus-5-5", usd: 75, share: 0.75 });
  });
  it("words the calibration", () => {
    expect(calibrationText(185.2)).toBe("1% of your weekly limit ≈ $185 of usage");
    expect(calibrationText(null)).toBeNull();
  });
});

describe("mock scenarios", () => {
  it("cover every state the line can show", () => {
    expect(paceLine(paceScenario("lasts", NOW), NOW).tone).toBe("muted");
    expect(paceLine(paceScenario("limit", NOW), NOW).tone).toBe("red");
    const weekly = paceScenario("weekly", NOW);
    expect(pickWindow(weekly.windows)?.key).toBe("weekly");
    expect(paceLine(weekly, NOW).tone).toBe("amber");
    expect(paceLine(paceScenario("measuring", NOW), NOW).text).toContain("measuring…");
    expect(paceLine(paceScenario("backfill", NOW), NOW).text).toMatch(/^\$—\/h/);
    expect(paceScenario("unpriced", NOW).spend.unpricedReplies).toBeGreaterThan(0);
    expect(paceScenario("lasts", NOW).spend.hourly).toHaveLength(24);
  });
});
