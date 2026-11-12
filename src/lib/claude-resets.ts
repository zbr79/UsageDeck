import type { ClaudeResetGroup, ClaudeResetKind, ClaudeResets } from "./types";

const NOUN: Record<ClaudeResetKind, string> = {
  full: "full reset",
  fiveHour: "five-hour reset",
  weekly: "weekly reset",
  other: "reset",
};
const DEFAULT_LABEL: Record<ClaudeResetKind, string> = {
  full: "full reset",
  fiveHour: "5-hour reset",
  weekly: "weekly reset",
  other: "reset",
};

const day = (iso: string) =>
  new Date(iso).toLocaleDateString("en-US", { month: "short", day: "numeric" });

/** "full reset" for Claude's own names, else its label: "Welcome back" reads `"Welcome back" reset`. */
function noun(g: ClaudeResetGroup): string {
  const label = g.label.trim();
  if (label.toLowerCase() === DEFAULT_LABEL[g.kind]) return NOUN[g.kind];
  if (/reset$/i.test(label)) return label.charAt(0).toLowerCase() + label.slice(1);
  return `"${label}" reset`;
}

/** "2 full resets (earliest ends Dec 3)", "1 five-hour reset (paused)". */
function groupText(g: ClaudeResetGroup): string {
  const notes = [
    g.paused && "paused",
    g.earliestExpiry && `${g.count > 1 ? "earliest ends" : "ends"} ${day(g.earliestExpiry)}`,
  ].filter(Boolean);
  const text = `${g.count} ${noun(g)}${g.count === 1 ? "" : "s"}`;
  return notes.length ? `${text} (${notes.join(", ")})` : text;
}

/** Each kind of reset with its count and nearest expiry, joined by " · ". */
export function claudeResetsText(resets: ClaudeResets): string {
  const parts = resets.groups.map(groupText);
  if (resets.weeklyFiveHourAvailable !== null) {
    parts.push(
      `weekly 5-hour reset ${resets.weeklyFiveHourAvailable ? "available" : "used this week"}`,
    );
  }
  return parts.join(" · ");
}

/** One row of Settings' Resets section: the reset's name, and how many with when they end. */
export interface ResetRow {
  key: string;
  title: string;
  detail: string;
}

/** "Free limit reset" for a grant's own label, "Full reset" for Claude's plain kinds. */
function rowTitle(g: ClaudeResetGroup): string {
  const label = g.label.trim() || DEFAULT_LABEL[g.kind];
  return label.charAt(0).toUpperCase() + label.slice(1);
}

/** The Resets section's rows: one per kind of reset, then the weekly 5-hour reset if known. */
export function claudeResetRows(resets: ClaudeResets): ResetRow[] {
  const rows = resets.groups.map((g, i) => ({
    key: `${g.kind}-${i}`,
    title: rowTitle(g),
    detail: [
      `${g.count} left`,
      g.paused && "paused",
      g.earliestExpiry && `${g.count > 1 ? "earliest ends" : "ends"} ${day(g.earliestExpiry)}`,
    ]
      .filter(Boolean)
      .join(" · "),
  }));
  if (resets.weeklyFiveHourAvailable !== null) {
    rows.push({
      key: "weekly-five-hour",
      title: "Weekly 5-hour reset",
      detail: resets.weeklyFiveHourAvailable ? "Available" : "Used this week",
    });
  }
  return rows;
}
