import { describe, expect, it } from "vitest";
import {
  calibrationText,
  evenPacePct,
  formatShare,
  formatUsd,
  formatUsdRate,
  headerRate,
  SPEND_BAND_TONE,
  spendBand,
  spendRateTitle,
  spendSeries,
  spendSeriesSummary,
  claudeModelLines,
  modelLabel,
  modelShares,
  usageSeverity,
  WINDOW_MS,
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
      quarters: [],
      quartersFable: [],
      minutes: [],
      minutesFable: [],
      unpricedReplies: 0,
      unpricedModels: [],
      scannedAt: NOW,
      backfilling: false,
      ...over,
    },
    windows: windows ?? { fiveHour: win(), weekly: win({ pct: 47, pctPerHour: 1 }) },
    fableFiveHour: null,
    fiveHourByModel: [],
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
});

describe("headerRate", () => {
  it("shows the hourly rate with the usual rounding", () => {
    expect(headerRate(pace())).toBe("$71/h");
    expect(headerRate(pace({ rate60m: 7.44 }))).toBe("$7.4/h");
    expect(headerRate(pace({ rate60m: 28.4 }))).toBe("$28/h");
  });
  it("shows nothing while backfilling or without pace", () => {
    expect(headerRate(pace({ backfilling: true }))).toBeNull();
    expect(headerRate(null)).toBeNull();
  });
});

describe("spendBand", () => {
  it("is none without a target and idle at zero spend", () => {
    expect(spendBand(50, null)).toBe("none");
    expect(spendBand(0, 100)).toBe("idle");
  });
  it("is below under 80%, on from 80% to 120%, above over 120%", () => {
    expect(spendBand(79.9, 100)).toBe("below");
    expect(spendBand(0.5, 100)).toBe("below");
    expect(spendBand(80, 100)).toBe("on");
    expect(spendBand(100, 100)).toBe("on");
    expect(spendBand(120, 100)).toBe("on");
    expect(spendBand(120.1, 100)).toBe("above");
  });
  it("maps to the muted, warning, success and danger text colours", () => {
    expect(SPEND_BAND_TONE.none).toBe("muted");
    expect(SPEND_BAND_TONE.idle).toBe("muted");
    expect(SPEND_BAND_TONE.below).toBe("warning");
    expect(SPEND_BAND_TONE.on).toBe("success");
    expect(SPEND_BAND_TONE.above).toBe("danger");
  });
});

describe("spend target text", () => {
  it("tooltip says what the number is, the target, the band and where the rate falls", () => {
    const base = "Claude Code spend over the last hour (API-equivalent, this PC)";
    expect(spendRateTitle(35, null)).toBe(base);
    expect(spendRateTitle(35, 100)).toBe(
      `${base} · target $100/h · below target (green $80–$120/h)`,
    );
    expect(spendRateTitle(100, 100)).toContain("· on target (green $80–$120/h)");
    expect(spendRateTitle(128, 100)).toContain("· above target");
    expect(spendRateTitle(0, 100)).toBe(`${base} · target $100/h · idle`);
  });
});

describe("evenPacePct", () => {
  const at = (ms: number) => new Date(NOW + ms).toISOString();
  it("takes the server value when there is one", () => {
    expect(evenPacePct(at(2 * H), WINDOW_MS.fiveHour, NOW, 69)).toBe(69);
  });
  it("computes the elapsed share of the window from its reset and length", () => {
    expect(evenPacePct(at(4 * H), WINDOW_MS.fiveHour, NOW, null)).toBeCloseTo(20);
    expect(evenPacePct(at(3.5 * 24 * H), WINDOW_MS.weekly, NOW)).toBeCloseTo(50);
  });
  it("clamps to the bar", () => {
    expect(evenPacePct(at(9 * H), WINDOW_MS.fiveHour, NOW)).toBe(0);
    expect(evenPacePct(at(H), WINDOW_MS.fiveHour, NOW, 140)).toBe(100);
  });
  it("hides when the reset is unknown or already past", () => {
    expect(evenPacePct(null, WINDOW_MS.fiveHour, NOW, 50)).toBeNull();
    expect(evenPacePct("not a date", WINDOW_MS.fiveHour, NOW)).toBeNull();
    expect(evenPacePct(at(-MIN), WINDOW_MS.fiveHour, NOW, 50)).toBeNull();
    expect(evenPacePct(at(0), WINDOW_MS.fiveHour, NOW)).toBeNull();
  });
});

describe("usageSeverity", () => {
  it("follows the percentage only: green, amber from 80%, red from 100%", () => {
    expect(usageSeverity(1)).toBe("ok");
    expect(usageSeverity(79.9)).toBe("ok");
    expect(usageSeverity(80)).toBe("warn");
    expect(usageSeverity(99.9)).toBe("warn");
    expect(usageSeverity(100)).toBe("danger");
  });
});

describe("claudeModelLines", () => {
  const scoped = (share: number | null) => ({
    label: "Fable",
    percent: 40,
    resetsAt: null,
    sharePctOfWeekly: share,
    allowancePctOfWeekly: 50,
  });

  it("names models by family and version", () => {
    expect(modelLabel("claude-opus-5-5")).toBe("Opus 5.5");
    expect(modelLabel("claude-fable-5-2-20261001")).toBe("Fable 5.2");
    expect(modelLabel("claude-haiku-5")).toBe("Haiku 5");
    expect(modelLabel("gpt-5")).toBe("gpt-5");
  });

  it("splits the weekly bar into the model's own points and the rest", () => {
    expect(claudeModelLines("weekly", 82, scoped(23), [])).toEqual([
      "Fable 23 pts",
      "Other models 59 pts",
    ]);
    expect(claudeModelLines("weekly", 82, scoped(null), [])).toEqual([]);
  });

  it("splits the 5-hour bar by each model's dollars, one line each", () => {
    const byModel = [
      { model: "claude-opus-5-5", usd: 60 },
      { model: "claude-fable-5-2", usd: 30 },
      { model: "claude-sonnet-5-5", usd: 10 },
    ];
    expect(claudeModelLines("fiveHour", 90, undefined, byModel)).toEqual([
      "Opus 5.5 ≈54 pts",
      "Fable 5.2 ≈27 pts",
      "Sonnet 5.5 ≈9 pts",
    ]);
    expect(claudeModelLines("fiveHour", 90, undefined, [])).toEqual([]);
    expect(claudeModelLines("fiveHour", 0, undefined, byModel)).toEqual([]);
  });
});

describe("settings helpers", () => {
  it("turns each range's dollars into $/h, oldest first, and summarises the peak", () => {
    const spend = pace({
      quarters: [5, 20, 10, 0],
      quartersFable: [1, 5, 0, 0],
      minutes: Array.from({ length: 60 }, (_, i) => (i === 3 ? 2 : i === 40 ? 4 : 0)),
      minutesFable: Array.from({ length: 60 }, (_, i) => (i === 3 ? 1 : 0)),
    }).spend;
    const day = spendSeries(spend, "24h");
    expect(day).toEqual([
      { total: 0, fable: 0 },
      { total: 40, fable: 0 },
      { total: 80, fable: 20 },
      { total: 20, fable: 4 },
    ]);
    expect(spendSeriesSummary(day, "24h")).toBe(
      "Spend over the last 24 hours. Peak $80/h, 15 minutes ago. Fable 17% of it.",
    );
    const quarter = spendSeries(spend, "15m");
    expect(quarter).toHaveLength(15);
    expect(quarter[11]).toEqual({ total: 120, fable: 60 });
    expect(spendSeriesSummary(quarter, "15m")).toBe(
      "Spend over the last 15 minutes. Peak $120/h, 3 minutes ago. Fable 50% of it.",
    );
    const hour = spendSeries(spend, "1h");
    expect(hour).toHaveLength(60);
    expect(spendSeriesSummary(hour, "1h")).toContain("Peak $240/h, 40 minutes ago");
    expect(spendSeriesSummary([], "1h")).toBe("Spend over the last hour: no data yet");
  });
  it("splits spend by model with shares", () => {
    const rows = modelShares([
      { model: "claude-sonnet-5-5", usd24h: 25 },
      { model: "claude-opus-5-5", usd24h: 75 },
    ]);
    expect(rows[0]).toEqual({ model: "claude-opus-5-5", usd: 75, share: 0.75 });
  });
  it("shows a tiny share as <1%", () => {
    expect(formatShare(0.003)).toBe("<1%");
    expect(formatShare(0.96)).toBe("96%");
  });
  it("words the calibration", () => {
    expect(calibrationText(185.2)).toBe("1% of your weekly limit ≈ $185 of usage");
    expect(calibrationText(null)).toBeNull();
  });
});

describe("mock scenarios", () => {
  it("cover every state the card can show", () => {
    const state = (p: ClaudePace, key: "fiveHour" | "weekly") => p.windows[key]?.state;
    expect(state(paceScenario("lasts", NOW), "fiveHour")).toBe("lasts");
    expect(state(paceScenario("limit", NOW), "fiveHour")).toBe("limit");
    expect(state(paceScenario("weekly", NOW), "weekly")).toBe("limit");
    expect(paceScenario("measuring", NOW).windows.fiveHour?.state).toBe("measuring");
    expect(headerRate(paceScenario("backfill", NOW))).toBeNull();
    expect(paceScenario("unpriced", NOW).spend.unpricedReplies).toBeGreaterThan(0);
    expect(paceScenario("unpriced", NOW).spend.unpricedModels.length).toBeGreaterThan(0);
    expect(paceScenario("lasts", NOW).spend.quarters).toHaveLength(96);
  });
});
