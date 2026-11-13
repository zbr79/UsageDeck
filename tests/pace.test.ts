import { describe, expect, it } from "vitest";
import {
  calibrationText,
  evenPacePct,
  formatShare,
  formatClock,
  formatPctPerHour,
  formatUsd,
  formatUsdRate,
  headerRate,
  SPEND_BAND_TONE,
  spendBand,
  spendRateTitle,
  spendSeries,
  spendSeriesSummary,
  meterPaceText,
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
      unpricedReplies: 0,
      unpricedModels: [],
      scannedAt: NOW,
      backfilling: false,
      ...over,
    },
    windows: windows ?? { fiveHour: win(), weekly: win({ pct: 47, pctPerHour: 1 }) },
    fableFiveHour: null,
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

describe("meterPaceText", () => {
  it("reads used, even pace, speed and where it ends at reset", () => {
    // 2 h left at +7%/h from 37%: ends ~51%
    const w = win({ pct: 37, pctPerHour: 7 });
    expect(meterPaceText(37, 69, w, NOW)).toBe(
      "37% used · even pace 69% · +7%/h · ends ~51% at reset",
    );
  });
  it("caps the end at 100%", () => {
    expect(meterPaceText(90, 50, win({ pctPerHour: 20 }), NOW)).toContain("ends ~100% at reset");
  });
  it("gives the full time when heading for the limit", () => {
    const w = win({ pctPerHour: 9, state: "limit", etaToLimit: NOW + 100 * MIN });
    expect(meterPaceText(41, 30, w, NOW)).toBe(
      "41% used · even pace 30% · +9%/h · full by 3:40 PM",
    );
  });
  it("says measuring while the speed is unknown", () => {
    const w = win({ state: "measuring", pctPerHour: null });
    expect(meterPaceText(2, 1, w, NOW)).toBe("2% used · even pace 1% · measuring speed…");
  });
  it("drops the parts it does not know", () => {
    expect(meterPaceText(37, null, null, NOW)).toBe("37% used");
    expect(meterPaceText(37, 20, win({ resetsAt: null }), NOW)).toBe(
      "37% used · even pace 20% · +9%/h",
    );
  });
});

describe("settings helpers", () => {
  it("turns quarter-hour dollars into $/h, oldest first, and summarises the peak", () => {
    const points = spendSeries([5, 20, 10, 0], [1, 5, 0, 0]);
    expect(points).toEqual([
      { total: 0, fable: 0 },
      { total: 40, fable: 0 },
      { total: 80, fable: 20 },
      { total: 20, fable: 4 },
    ]);
    expect(spendSeriesSummary(points)).toBe(
      "Spend over the last 24 hours. Peak $80/h, 15 minutes ago. Fable 17% of it.",
    );
    expect(spendSeriesSummary([])).toBe("Spend over the last 24 hours: no data yet");
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
    const m = paceScenario("measuring", NOW).windows.fiveHour!;
    expect(meterPaceText(m.pct, m.evenPacePct, m, NOW)).toContain("measuring speed…");
    expect(headerRate(paceScenario("backfill", NOW))).toBeNull();
    expect(paceScenario("unpriced", NOW).spend.unpricedReplies).toBeGreaterThan(0);
    expect(paceScenario("unpriced", NOW).spend.unpricedModels.length).toBeGreaterThan(0);
    expect(paceScenario("lasts", NOW).spend.quarters).toHaveLength(96);
  });
});
