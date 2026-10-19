// Pure helpers for the Claude "spend and pace" line (widget) and section (settings).
// Data comes from GET /api/claude/pace; see docs/plans/claude-spend-and-pace.md.
import type { ClaudePace, PaceWindow } from "./types";

export type PaceTone = "muted" | "amber" | "red";
export type PaceWindowKey = "fiveHour" | "weekly";

/** Red once the projected limit is this close (or closer). */
export const RED_WITHIN_MS = 30 * 60_000;

/** The line drops its window label above this many characters, to fit a 238 px card. */
export const LINE_MAX_CHARS = 36;

export const WINDOW_LABELS: Record<PaceWindowKey, { short: string; long: string }> = {
  fiveHour: { short: "5h", long: "5-hour" },
  weekly: { short: "wk", long: "Weekly" },
};

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

/**
 * The window that matters more: the one heading for its limit soonest, else the 5-hour window
 * (else the weekly one, when the 5-hour one is missing).
 */
export function pickWindow(
  windows: ClaudePace["windows"],
): { key: PaceWindowKey; window: PaceWindow } | null {
  const five = limitEta(windows.fiveHour);
  const week = limitEta(windows.weekly);
  if (week != null && (five == null || week < five))
    return { key: "weekly", window: windows.weekly! };
  if (windows.fiveHour) return { key: "fiveHour", window: windows.fiveHour };
  if (windows.weekly) return { key: "weekly", window: windows.weekly };
  return null;
}

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

export interface PaceLine {
  text: string;
  tone: PaceTone;
  /** multi-line tooltip */
  title: string;
}

/** The widget's one line: "$71/h · 5h +9%/h · lasts" or "… · limit 3:40 PM". */
export function paceLine(pace: ClaudePace, now: number, maxChars = LINE_MAX_CHARS): PaceLine {
  const rate = formatUsdRate(pace.spend.backfilling ? null : pace.spend.rate60m);
  const picked = pickWindow(pace.windows);
  let text = rate;
  if (picked) {
    const w = picked.window;
    if (isMeasuring(w)) text = `${rate} · measuring…`;
    else {
      const speed = formatPctPerHour(w.pctPerHour!);
      const tail = windowForecast(w, now);
      text = `${rate} · ${WINDOW_LABELS[picked.key].short} ${speed} · ${tail}`;
      if (text.length > maxChars) text = `${rate} · ${speed} · ${tail}`;
    }
  }
  return { text, tone: paceTone(picked?.window ?? null, now), title: paceTitle(pace, now) };
}

/** Tooltip: both rates, both windows' speed and forecast, and the coverage. */
export function paceTitle(pace: ClaudePace, now: number): string {
  const s = pace.spend;
  const lines = [
    s.backfilling
      ? "Spend: reading Claude Code logs…"
      : `Spend: ${formatUsdRate(s.rate60m)} (last 60 min) · ${formatUsdRate(s.rate15m)} (last 15 min)`,
  ];
  for (const key of ["fiveHour", "weekly"] as const) {
    const w = pace.windows[key];
    if (!w) continue;
    const speed = isMeasuring(w) ? "" : ` · ${formatPctPerHour(w.pctPerHour!)}`;
    lines.push(
      `${WINDOW_LABELS[key].long}: ${formatPct(w.pct)}${speed} · ${windowForecast(w, now, true)}`,
    );
  }
  lines.push("Claude Code on this PC (API-equivalent, not your bill)");
  return lines.join("\n");
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
