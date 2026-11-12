import type { ClaudeResetGroup, ClaudeResetKind, ClaudeResets } from "./types";

const DEFAULT_LABEL: Record<ClaudeResetKind, string> = {
  full: "full reset",
  fiveHour: "5-hour reset",
  weekly: "weekly reset",
  other: "reset",
};

const day = (iso: string) =>
  new Date(iso).toLocaleDateString("en-US", { month: "short", day: "numeric" });

/** One row of an account's resets in Settings: the reset's name, and how many with when they end. */
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

const SHORT: Record<ClaudeResetKind, string> = {
  full: "Full reset",
  fiveHour: "5-hour reset",
  weekly: "Weekly reset",
  other: "Reset",
};

/** "Fable reset" when the grant names a model, else its kind: "Full reset". */
function shortName(g: ClaudeResetGroup): string {
  const model = /\b(fable|opus|sonnet|haiku)\b/i.exec(g.label)?.[1];
  if (model) return `${model.charAt(0).toUpperCase()}${model.slice(1).toLowerCase()} reset`;
  return SHORT[g.kind];
}

/** The reset pill's tooltip: "1 Fable reset · 2 Full resets · 1 5-hour reset (paused)". */
export function claudeResetsSummary(resets: ClaudeResets): string {
  const parts = resets.groups.map((g) => {
    const name = `${g.count} ${shortName(g)}${g.count === 1 ? "" : "s"}`;
    return g.paused ? `${name} (paused)` : name;
  });
  if (resets.weeklyFiveHourAvailable) parts.push("1 weekly 5-hour reset");
  return parts.join(" · ");
}
