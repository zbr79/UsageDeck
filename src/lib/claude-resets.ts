import type { ClaudeResetGroup, ClaudeResetKind, ClaudeResets, CodexUsage } from "./types";

const day = (iso: string) =>
  new Date(iso).toLocaleDateString("en-US", { month: "short", day: "numeric" });

const KIND_NAME: Record<ClaudeResetKind, string> = {
  full: "Full reset",
  fiveHour: "5-hour reset",
  weekly: "Weekly reset",
  other: "Reset",
};

/** "Fable reset" when the grant names a model, else its kind: "Full reset". */
function typeName(g: ClaudeResetGroup): string {
  const model = /\b(fable|opus|sonnet|haiku)\b/i.exec(g.label)?.[1];
  if (model) return `${model.charAt(0).toUpperCase()}${model.slice(1).toLowerCase()} reset`;
  return KIND_NAME[g.kind];
}

/** Claude's resets of one type: their count and soonest expiry. Paused ones are their own type. */
interface ResetType {
  name: string;
  count: number;
  earliestExpiry: string | null;
  paused: boolean;
}

/** Resets merged by type name; the weekly 5-hour reset, while available, counts as a 5-hour reset. */
function resetTypes(resets: ClaudeResets): ResetType[] {
  const groups: ClaudeResetGroup[] = [...resets.groups];
  if (resets.weeklyFiveHourAvailable) {
    groups.push({ kind: "fiveHour", label: "", count: 1, earliestExpiry: null, paused: false });
  }
  const types = new Map<string, ResetType>();
  for (const g of groups) {
    const name = typeName(g);
    const key = `${name}|${g.paused}`;
    const t = types.get(key);
    if (!t) {
      types.set(key, { name, count: g.count, earliestExpiry: g.earliestExpiry, paused: g.paused });
      continue;
    }
    t.count += g.count;
    if (g.earliestExpiry && (!t.earliestExpiry || g.earliestExpiry < t.earliestExpiry)) {
      t.earliestExpiry = g.earliestExpiry;
    }
  }
  return [...types.values()];
}

/** One line of an account's resets in Settings: the type, and how many with when the first ends. */
export interface ResetRow {
  key: string;
  title: string;
  detail: string;
}

const detail = (count: number, expiry: string | null, paused = false) =>
  [`${count} left`, paused && "paused", expiry && `ends ${day(expiry)}`]
    .filter(Boolean)
    .join(" · ");

/** Claude's resets by type: "Full reset — 2 left · ends Dec 10". */
export function claudeResetRows(resets: ClaudeResets): ResetRow[] {
  return resetTypes(resets).map((t) => ({
    key: `${t.name}|${t.paused}`,
    title: t.name,
    detail: detail(t.count, t.earliestExpiry, t.paused),
  }));
}

/** Codex's banked resets: one type, with the soonest expiry when Codex lists it. */
export function codexResetRows(usage: CodexUsage): ResetRow[] {
  const count = usage.resetCredits ?? 0;
  if (count <= 0) return [];
  return [
    { key: "codex", title: "Reset", detail: detail(count, usage.resetExpiries?.[0] ?? null) },
  ];
}

/** The reset pill's tooltip: "1 Fable reset · 2 Full resets · 1 5-hour reset (paused)". */
export function claudeResetsSummary(resets: ClaudeResets): string {
  return resetTypes(resets)
    .map((t) => `${t.count} ${t.name}${t.count === 1 ? "" : "s"}${t.paused ? " (paused)" : ""}`)
    .join(" · ");
}
