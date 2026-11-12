// Pure helpers for Claude spend and pace: the widget card (header rate, even-pace tick and meter
// tooltip) and the settings section.
// Data comes from GET /api/claude/pace; see docs/plans/claude-spend-and-pace.md.
import type { ClaudePace, ClaudeScopedWindow, ModelShare, PaceWindow } from "./types";

export type PaceTone = "muted" | "amber" | "red";
export type PaceWindowKey = "fiveHour" | "weekly";

/** Red once the projected limit is this close (or closer). */
export const RED_WITHIN_MS = 30 * 60_000;

const H = 3_600_000;

export const WINDOW_LABELS: Record<PaceWindowKey, { long: string }> = {
  fiveHour: { long: "5-hour" },
  weekly: { long: "Weekly" },
};

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

/** Signed percent per hour: "+9%/h", "+0.4%/h", "-2%/h". */
export function formatPctPerHour(v: number): string {
  const abs = Math.abs(v);
  const s = abs < 1 && abs > 0 ? abs.toFixed(1) : String(Math.round(abs));
  const sign = v < 0 && s !== "0" && s !== "0.0" ? "-" : "+";
  return `${sign}${s}%/h`;
}

/** Whole percent, or one decimal under 1 ("41%", "0.4%"). */
export function formatPct(v: number): string {
  const abs = Math.abs(v);
  return `${abs > 0 && abs < 1 ? v.toFixed(1) : Math.round(v)}%`;
}

const sameDay = (a: Date, b: Date) =>
  a.getFullYear() === b.getFullYear() &&
  a.getMonth() === b.getMonth() &&
  a.getDate() === b.getDate();

/** Local clock time, with a weekday when it is not today: "3:40 PM", "Tue 3:40 PM". */
export function formatClock(at: number, now: number): string {
  const d = new Date(at);
  const time = d.toLocaleTimeString("en-US", { hour: "numeric", minute: "2-digit" });
  if (sameDay(d, new Date(now))) return time;
  return `${d.toLocaleDateString("en-US", { weekday: "short" })} ${time}`;
}

/** A window heading for its limit: state "limit" with a known ETA. */
const limitEta = (w: PaceWindow | null): number | null =>
  w && w.state === "limit" && w.etaToLimit != null ? w.etaToLimit : null;

/** Muted normally; amber when heading for a limit; red when that is 30 minutes away or less. */
export function paceTone(w: PaceWindow | null, now: number): PaceTone {
  const eta = limitEta(w);
  if (eta == null) return "muted";
  return eta - now <= RED_WITHIN_MS ? "red" : "amber";
}

const isMeasuring = (w: PaceWindow) => w.state === "measuring" || w.pctPerHour == null;

/** One window's forecast in words: "measuring…", "lasts", "limit 3:40 PM". */
export function windowForecast(w: PaceWindow, now: number, untilReset = false): string {
  if (isMeasuring(w)) return "measuring…";
  const eta = limitEta(w);
  if (eta != null) return `limit ${formatClock(eta, now)}`;
  return untilReset ? "lasts until reset" : "lasts";
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

/** "Green between $80 and $120 per hour". */
export function targetBandText(target: number): string {
  return `Green between ${formatUsd(target * TARGET_LOW)} and ${formatUsd(target * TARGET_HIGH)} per hour`;
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

/**
 * The pace part of a meter's tooltip: "37% used · even pace 69% · +9%/h · ends ~51% at reset", or
 * "… · full by 3:40 PM", or "… · measuring speed…". The end is pct + speed × hours left, capped at 100.
 */
export function meterPaceText(
  pct: number,
  even: number | null,
  w: PaceWindow | null,
  now: number,
): string {
  const parts = [`${formatPct(pct)} used`];
  if (even != null) parts.push(`even pace ${formatPct(even)}`);
  if (w) {
    if (isMeasuring(w)) parts.push("measuring speed…");
    else {
      parts.push(formatPctPerHour(w.pctPerHour!));
      const eta = limitEta(w);
      const reset = resetTime(w.resetsAt);
      if (eta != null) parts.push(`full by ${formatClock(eta, now)}`);
      else if (reset != null && reset > now) {
        const end = Math.max(0, Math.min(100, pct + (w.pctPerHour! * (reset - now)) / H));
        parts.push(`ends ~${formatPct(end)} at reset`);
      }
    }
  }
  return parts.join(" · ");
}

/** "ahead of even pace by 12 pts" / "behind even pace by 5 pts" / "on even pace". */
export function paceVsEven(w: PaceWindow): string | null {
  if (w.evenPacePct == null) return null;
  const diff = Math.round(w.pct - w.evenPacePct);
  if (diff === 0) return "on even pace";
  return `${diff > 0 ? "ahead of" : "behind"} even pace by ${Math.abs(diff)} pts`;
}

/** Bars oldest first: the API sends index 0 = the current partial hour. */
export function hourlyOldestFirst(hourly: number[]): number[] {
  return [...hourly].reverse();
}

/** The chart's accessible summary: the peak hour and the current hour. */
export function hourlySummary(hourly: number[]): string {
  if (hourly.length === 0) return "Spend per hour: no data yet";
  let peak = 0;
  for (let i = 1; i < hourly.length; i++) if (hourly[i] > hourly[peak]) peak = i;
  const ago = peak === 0 ? "this hour" : `${peak} ${peak === 1 ? "hour" : "hours"} ago`;
  return (
    `Spend per hour over the last ${hourly.length} hours. ` +
    `Peak ${formatUsdRate(hourly[peak])}, ${ago}. ` +
    `Current hour so far ${formatUsd(hourly[0])}.`
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

/** A lighter part at the end of a meter's fill: the points one model used, and what to say. */
export interface MeterSegment {
  pct: number;
  title: string;
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
  const allowance =
    scoped!.allowancePctOfWeekly == null
      ? ""
      : ` (allowance ${formatPct(scoped!.allowancePctOfWeekly)} of weekly)`;
  return { pct: Math.min(share, weeklyPct), title: `${scoped!.label} ${pts(share)}${allowance}` };
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
  const pct = fiveHourPct * Math.min(1, share.usd / share.totalUsd);
  return { pct, title: `Fable ≈${pts(pct)} (estimate from Claude Code on this PC)` };
}
