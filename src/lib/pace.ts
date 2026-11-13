// Pure helpers for Claude spend and pace: the widget card (header rate, even-pace tick and meter
// tooltip) and the settings section.
// Data comes from GET /api/claude/pace; see docs/plans/claude-spend-and-pace.md.
import type { ClaudePace, ClaudeScopedWindow, ModelShare } from "./types";

export type PaceWindowKey = "fiveHour" | "weekly";

const H = 3_600_000;

/** Length of each Claude usage window. */
export const WINDOW_MS: Record<PaceWindowKey, number> = { fiveHour: 5 * H, weekly: 7 * 24 * H };

/** Dollars: one decimal under $10, whole dollars otherwise ("$7.4", "$71", "$1,293"). */
export function formatUsd(v: number): string {
  const abs = Math.abs(v);
  const sign = v < 0 ? "-" : "";
  if (abs < 10) {
    const s = abs.toFixed(1);
    // 9.96 rounds up to "10.0": show it as a whole number instead.
    return `${sign}$${s === "10.0" ? "10" : s}`;
  }
  return `${sign}$${Math.round(abs).toLocaleString("en-US")}`;
}

/** "$71/h"; "$—/h" when the number is not known yet. */
export function formatUsdRate(v: number | null): string {
  return v == null ? "$—/h" : `${formatUsd(v)}/h`;
}

/** Whole percent, or one decimal under 1 ("41%", "0.4%"). */
export function formatPct(v: number): string {
  const abs = Math.abs(v);
  return `${abs > 0 && abs < 1 ? v.toFixed(1) : Math.round(v)}%`;
}

/** The card header's rate, "$28/h"; null while backfilling or without pace (the call failed). */
export function headerRate(pace: ClaudePace | null): string | null {
  return !pace || pace.spend.backfilling ? null : formatUsdRate(pace.spend.rate60m);
}

/** Where the hourly spend falls against the owner's target. */
export type SpendBand = "none" | "idle" | "below" | "on" | "above";
/** Text colour token per band; the caller maps it to its own palette (widget or settings). */
export type SpendTone = "muted" | "warning" | "success" | "danger";

/** "On target" is 80 %-120 % of the target, both ends included. */
export const TARGET_LOW = 0.8;
export const TARGET_HIGH = 1.2;

export const SPEND_BAND_TONE: Record<SpendBand, SpendTone> = {
  none: "muted",
  idle: "muted",
  below: "warning",
  on: "success",
  above: "danger",
};

/** No target: none. Zero spend: idle (not a nudge). Else below / on / above the 80-120 % band. */
export function spendBand(rate: number, target: number | null): SpendBand {
  if (target == null || !(target > 0)) return "none";
  if (!(rate > 0)) return "idle";
  if (rate < target * TARGET_LOW) return "below";
  if (rate > target * TARGET_HIGH) return "above";
  return "on";
}

const BAND_WORDS: Record<SpendBand, string> = {
  none: "",
  idle: "idle",
  below: "below target",
  on: "on target",
  above: "above target",
};

/** The header rate's tooltip and accessible name; with a target it names the band, so colour is not the only cue. */
export function spendRateTitle(rate: number, target: number | null): string {
  const base = "Claude Code spend over the last hour (API-equivalent, this PC)";
  const band = spendBand(rate, target);
  if (band === "none" || target == null) return base;
  const green = `green ${formatUsd(target * TARGET_LOW)}–${formatUsd(target * TARGET_HIGH)}/h`;
  const where = band === "idle" ? BAND_WORDS.idle : `${BAND_WORDS[band]} (${green})`;
  return `${base} · target ${formatUsdRate(target)} · ${where}`;
}

const resetTime = (resetsAt: string | null | undefined): number | null => {
  const t = resetsAt ? Date.parse(resetsAt) : NaN;
  return Number.isFinite(t) ? t : null;
};

/**
 * Where the even-pace tick sits, in percent of the bar: the server's value, else the share of the
 * window already gone (from its reset time and length; needs no history). Null hides the tick: the
 * reset is unknown or already past.
 */
export function evenPacePct(
  resetsAt: string | null | undefined,
  lengthMs: number,
  now: number,
  fromServer: number | null = null,
): number | null {
  const reset = resetTime(resetsAt);
  if (reset == null || reset <= now) return null;
  const v = fromServer ?? (1 - (reset - now) / lengthMs) * 100;
  return Math.min(100, Math.max(0, v));
}

export type MeterSeverity = "ok" | "warn" | "danger";

/** Every meter's colour follows its percentage only: warn from 80%, danger from 100%. */
export function usageSeverity(percent: number): MeterSeverity {
  return percent >= 100 ? "danger" : percent >= 80 ? "warn" : "ok";
}

/** One step of the spend chart: all dollars and Fable's, as hourly rates. */
export interface SpendPoint {
  /** USD per hour */
  total: number;
  fable: number;
}

/** The chart's ranges: the last 15 minutes and hour minute by minute, 24 hours by quarter hour. */
export type SpendRange = "15m" | "1h" | "24h";

export const SPEND_RANGES: { value: SpendRange; label: string; title: string }[] = [
  { value: "15m", label: "15m", title: "Last 15 minutes" },
  { value: "1h", label: "1h", title: "Last hour" },
  { value: "24h", label: "24h", title: "Last 24 hours" },
];

/** Minutes per point of each range. */
export const RANGE_STEP: Record<SpendRange, number> = { "15m": 1, "1h": 1, "24h": 15 };

/** A range's points oldest first, as $/h. The API sends USD per step, index 0 = now. */
export function spendSeries(spend: ClaudePace["spend"], range: SpendRange): SpendPoint[] {
  const [values, fable] =
    range === "24h" ? [spend.quarters, spend.quartersFable] : [spend.minutes, spend.minutesFable];
  const count = range === "15m" ? 15 : values.length;
  const perHour = 60 / RANGE_STEP[range];
  return values
    .slice(0, count)
    .map((v, i) => ({ total: v * perHour, fable: (fable[i] ?? 0) * perHour }))
    .reverse();
}

/** The chart's accessible summary: the peak rate and when, and Fable's part of the period. */
export function spendSeriesSummary(points: readonly SpendPoint[], range: SpendRange): string {
  const span = SPEND_RANGES.find((r) => r.value === range)!.title.toLowerCase();
  if (points.length === 0) return `Spend over the ${span}: no data yet`;
  let peak = 0;
  for (let i = 1; i < points.length; i++) if (points[i]!.total > points[peak]!.total) peak = i;
  const minsAgo = (points.length - 1 - peak) * RANGE_STEP[range];
  const hours = Math.round(minsAgo / 60);
  const ago =
    minsAgo === 0
      ? "now"
      : minsAgo < 60
        ? `${minsAgo} ${minsAgo === 1 ? "minute" : "minutes"} ago`
        : `${hours} ${hours === 1 ? "hour" : "hours"} ago`;
  const total = points.reduce((a, p) => a + p.total, 0);
  const fable = points.reduce((a, p) => a + p.fable, 0);
  return (
    `Spend over the ${span}. Peak ${formatUsdRate(points[peak]!.total)}, ${ago}. ` +
    `Fable ${total > 0 ? formatShare(fable / total) : "0%"} of it.`
  );
}

/** Each model's 24-hour spend with its share of the total, largest first. */
export function modelShares(
  byModel: ClaudePace["spend"]["byModel"],
): { model: string; usd: number; share: number }[] {
  const total = byModel.reduce((sum, m) => sum + m.usd24h, 0);
  return [...byModel]
    .sort((a, b) => b.usd24h - a.usd24h)
    .map((m) => ({ model: m.model, usd: m.usd24h, share: total > 0 ? m.usd24h / total : 0 }));
}

/** A model's share of the total: "96%", "<1%". */
export function formatShare(share: number): string {
  return share > 0 && share < 0.005 ? "<1%" : `${Math.round(share * 100)}%`;
}

/** "1% of your weekly limit ≈ $185 of usage", or null before it can be measured. */
export function calibrationText(usdPerWeeklyPct: number | null): string | null {
  return usdPerWeeklyPct == null
    ? null
    : `1% of your weekly limit ≈ ${formatUsd(usdPerWeeklyPct)} of usage`;
}

/** A lighter part at the end of a meter's fill: the points one model used. */
export interface MeterSegment {
  pct: number;
}

const pts = (v: number) => formatPct(v).replace("%", " pts");

/**
 * Fable's part of the weekly bar, exact from Anthropic: its use in points of the weekly bar, at
 * most the bar's own fill. Null without any use.
 */
export function weeklyModelSegment(
  weeklyPct: number,
  scoped: ClaudeScopedWindow | undefined,
): MeterSegment | null {
  const share = scoped?.sharePctOfWeekly;
  if (share == null || !(share > 0) || !(weeklyPct > 0)) return null;
  return { pct: Math.min(share, weeklyPct) };
}

/**
 * Fable's part of the 5-hour bar, an estimate: the bar times Fable's share of the API-equivalent
 * dollars Claude Code on this PC spent in the window. Use elsewhere (claude.ai, other PCs) is not
 * in the logs, so this can be off either way. Null without Fable dollars.
 */
export function fiveHourFableSegment(
  fiveHourPct: number,
  share: ModelShare | null,
): MeterSegment | null {
  if (!share || !(share.totalUsd > 0) || !(share.usd > 0) || !(fiveHourPct > 0)) return null;
  return { pct: fiveHourPct * Math.min(1, share.usd / share.totalUsd) };
}

/** "claude-opus-5-5" → "Opus 5.5"; other ids stay as they are. */
export function modelLabel(id: string): string {
  const m = /^claude-([a-z]+)-(\d+)(?:-(\d+))?/.exec(id);
  if (!m) return id;
  const family = m[1]!.charAt(0).toUpperCase() + m[1]!.slice(1);
  return `${family} ${m[2]}${m[3] ? `.${m[3]}` : ""}`;
}

/**
 * A Claude bar's tooltip lines, one per model. The weekly bar: the per-model limit's exact points
 * (Fable), then the rest. The 5-hour bar: each model's share of this PC's Claude Code dollars in
 * the window, as points ("≈", an estimate). Empty when nothing splits the bar.
 */
export function claudeModelLines(
  bar: PaceWindowKey,
  pct: number,
  scoped: ClaudeScopedWindow | undefined,
  byModel: readonly { model: string; usd: number }[],
): string[] {
  if (!(pct > 0)) return [];
  if (bar === "weekly") {
    const share = scoped?.sharePctOfWeekly;
    if (share == null || !(share > 0)) return [];
    const own = Math.min(share, pct);
    return [`${scoped!.label} ${pts(own)}`, `Other models ${pts(pct - own)}`];
  }
  const total = byModel.reduce((sum, m) => sum + m.usd, 0);
  if (!(total > 0)) return [];
  return byModel
    .filter((m) => m.usd > 0)
    .map((m) => `${modelLabel(m.model)} ≈${pts((pct * m.usd) / total)}`);
}
