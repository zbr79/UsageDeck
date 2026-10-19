// Pure helpers for Claude spend and pace: the widget card (header rate, even-pace tick, meter colour
// and tooltip) and the settings section.
// Data comes from GET /api/claude/pace; see docs/plans/claude-spend-and-pace.md.
import type { ClaudePace, PaceWindow } from "./types";

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
const RANK: Record<MeterSeverity, number> = { ok: 0, warn: 1, danger: 2 };

/** The meter's usage colours: warn from 80%, danger from 100%. */
export function usageSeverity(percent: number): MeterSeverity {
  return percent >= 100 ? "danger" : percent >= 80 ? "warn" : "ok";
}

/**
 * A Claude meter's fill: the more severe of its usage colour and its pace (amber when heading for
 * the limit more than 30 minutes away, red at 30 minutes or less).
 */
export function meterSeverity(percent: number, w: PaceWindow | null, now: number): MeterSeverity {
  const tone = paceTone(w, now);
  const pace: MeterSeverity = tone === "red" ? "danger" : tone === "amber" ? "warn" : "ok";
  const usage = usageSeverity(percent);
  return RANK[pace] > RANK[usage] ? pace : usage;
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
