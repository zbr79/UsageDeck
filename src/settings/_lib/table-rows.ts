// The accounts table of every provider page: which usage columns it shows, each account's cells,
// and the provider's own last column (Claude and Codex resets, Cursor API spend).
// Pure, so it is unit-tested (tests/table-rows.test.ts).
import { claudeBars, codexBars, shownBars, type HiddenBars } from "@/lib/bars";
import {
  claudeResetRows,
  codexResetRows,
  resetCellText,
  resetCellTitle,
  type ResetRow,
} from "@/lib/claude-resets";
import { cardErrorKind, cursorErrorHint, errorHint, isAuthRejected } from "@/lib/errors";
import { apiByModel, capitalize, claudePlanLabel, formatUntil, formatUsd } from "@/lib/format";
import { isPassed, isUnknownWindow } from "@/lib/ranking";
import type {
  ClaudeUsage,
  CodexUsage,
  CursorAttemptNote,
  CursorUsage,
  GrokUsage,
  OpenCodeAccountUsage,
  TrackedAccount,
} from "@/lib/types";
import type { Tone } from "@/ui/palette";
import type { ExtraCell, TableColumn, UsageCell } from "../_components/AccountTable";

export type TableProvider = "claude" | "codex";

export interface TableRowData {
  cells: Partial<Record<string, UsageCell>>;
  /** replaces the cells: no usage, or not signed in */
  note: string | null;
  plan: { label: string; tone: Tone } | null;
  /** the provider's last column; undefined when its table has none */
  extra?: ExtraCell;
  warning: { text: string; title: string } | null;
  /** the numbers are old or another window is full: gray bars */
  dimmed?: boolean;
}

/** The last column's header per provider; none for Grok Bot and OpenCode. */
export const EXTRA_COLUMN: Partial<Record<string, string>> = {
  claude: "Resets",
  codex: "Resets",
  cursor: "API spend",
};

const planOf = (label: string | null | undefined, tone: Tone = "blue") =>
  label ? { label, tone } : null;

/** "2 Full · 1 Fable" with the dates on hover, or "No resets". */
function resetsCell(rows: readonly ResetRow[]): ExtraCell {
  const text = resetCellText(rows);
  return text
    ? { text, title: resetCellTitle(rows), icon: "reset" }
    : { text: "No resets", muted: true };
}

/** Claude's tones: free green, Ultra amber, other plans blue (the widget card's). */
function planTone(plan: string): Tone {
  const p = plan.toLowerCase();
  return p === "free" ? "green" : p === "ultra" ? "amber" : "blue";
}

/** The columns every account's bars fit in: the bars any account has, minus the hidden ones. */
export function tableColumns(
  provider: TableProvider,
  accounts: readonly TrackedAccount<ClaudeUsage | CodexUsage>[],
  hidden: HiddenBars,
): TableColumn[] {
  if (provider === "codex") {
    const has = new Set(
      accounts.flatMap((a) => (a.usage ? codexBars(a.usage as CodexUsage) : ["fiveHour"])),
    );
    const shown = shownBars(
      "codex",
      hidden,
      has.has("weekly") ? ["fiveHour", "weekly"] : ["fiveHour"],
    );
    return [
      { id: "fiveHour", name: "5-hour" },
      { id: "weekly", name: "Weekly" },
    ].filter((c) => shown.has(c.id as "fiveHour" | "weekly"));
  }
  const claude = accounts.map((a) => a.usage as ClaudeUsage | null);
  const scoped = claude.map((c) => c?.weeklyScoped[0]?.label).find(Boolean);
  const has = claude.some((c) => c && claudeBars(c).includes("scoped"));
  const shown = shownBars(
    "claude",
    hidden,
    has ? ["fiveHour", "weekly", "scoped"] : ["fiveHour", "weekly"],
  );
  return [
    { id: "fiveHour", name: "5-hour" },
    { id: "weekly", name: "Weekly" },
    { id: "scoped", name: `${scoped ?? "Model"} weekly` },
  ].filter((c) => shown.has(c.id as "fiveHour" | "weekly" | "scoped"));
}

/** A window whose reset has passed in cached usage is unknown: its old percent no longer holds. */
function cell(
  percent: number | null | undefined,
  resetsAt: string | null | undefined,
  stale: boolean,
  now: number,
): UsageCell {
  if (percent == null || (stale && isPassed(resetsAt, now))) {
    return { percent: null, reset: null };
  }
  return { percent, reset: formatUntil(resetsAt, now) };
}

function warningOf(
  account: TrackedAccount<unknown>,
  how: string,
): { text: string; title: string } | null {
  if (isAuthRejected(account.authExpired, account.error)) {
    return { text: "re-auth", title: `Sign-in expired · ${how} to refresh it` };
  }
  const hint = errorHint(cardErrorKind(account.error, account.active), how);
  return hint && { text: hint.short, title: hint.long };
}

export function claudeRow(account: TrackedAccount<ClaudeUsage>, now: number): TableRowData {
  const c = account.usage;
  const base = { warning: warningOf(account, "open Claude Code"), cells: {} };
  const none = resetsCell([]);
  if (!c) return { ...base, note: "Not signed in or unavailable", plan: null, extra: none };
  if (!c.signedIn) return { ...base, note: "Not signed in", plan: null, extra: none };
  const scoped = c.weeklyScoped[0];
  const plan = claudePlanLabel(
    c.planName ?? (c.subscribed ? (c.subscriptionType ?? "subscribed") : "Free"),
  );
  return {
    ...base,
    note: null,
    plan: { label: plan, tone: planTone(plan) },
    extra: resetsCell(c.resets ? claudeResetRows(c.resets) : []),
    cells: {
      fiveHour: cell(c.fiveHour.percent, c.fiveHour.resetsAt, account.stale, now),
      weekly: cell(c.weekly.percent, c.weekly.resetsAt, account.stale, now),
      ...(scoped && { scoped: cell(scoped.percent, scoped.resetsAt, account.stale, now) }),
    },
  };
}

export function codexRow(account: TrackedAccount<CodexUsage>, now: number): TableRowData {
  const x = account.usage;
  const warning = warningOf(account, "run codex");
  if (!x) {
    return {
      warning,
      cells: {},
      note: "Not signed in or unavailable",
      plan: null,
      extra: resetsCell([]),
    };
  }
  const plan = x.planType && capitalize(x.planType);
  return {
    warning,
    note: null,
    plan: plan ? { label: plan, tone: planTone(plan) } : null,
    extra: resetsCell(codexResetRows(x)),
    cells: {
      fiveHour: cell(x.usedPercent, x.resetAt, account.stale, now),
      weekly: cell(x.weeklyUsedPercent, x.weeklyResetAt, account.stale, now),
    },
  };
}

/** Cursor's two buckets share one reset; a passed cycle in cached usage is unknown. */
export function cursorRow(
  account: TrackedAccount<CursorUsage>,
  now: number,
  note: CursorAttemptNote | null,
): TableRowData {
  const c = account.usage;
  const signedOut = isAuthRejected(account.authExpired, account.error);
  const failure = cursorErrorHint(account.error, note);
  const warning = signedOut
    ? { text: "re-auth", title: note?.reason ?? "Sign-in expired · open Cursor to refresh it" }
    : failure && { text: failure.short, title: failure.long };
  if (!c) {
    return {
      warning,
      cells: {},
      note: "Not signed in or unavailable",
      plan: null,
      extra: { text: "—", muted: true },
    };
  }
  const passed = account.stale && isPassed(c.resetsAt, now);
  const at = (percent: number | null): UsageCell =>
    passed || percent == null
      ? { percent: null, reset: null }
      : { percent, reset: formatUntil(c.resetsAt, now) };
  const api = passed ? null : c.apiSpend;
  return {
    warning,
    note: null,
    plan: planOf(c.planName),
    dimmed: signedOut,
    cells: { firstParty: at(c.cursorModelsPercentUsed), api: at(c.otherModelsPercentUsed) },
    extra: api
      ? {
          text: `${formatUsd(api.spentUsd)}${api.limitUsd == null ? "" : ` / ${formatUsd(api.limitUsd)}`}`,
          title: apiByModel(api.byModel).join("\n") || undefined,
        }
      : { text: "—", muted: true },
  };
}

export function grokRow(account: TrackedAccount<GrokUsage>, now: number): TableRowData {
  const g = account.usage;
  const warning = warningOf(account, "open Grok Bot");
  if (!g) return { warning, cells: {}, note: "Not signed in or unavailable", plan: null };
  const passed = account.stale && isPassed(g.resetAt, now);
  return {
    warning,
    note: null,
    plan: planOf(g.planLabel ?? g.cursorPlanName),
    cells: {
      weekly:
        passed || g.percentUsed == null
          ? { percent: null, reset: null }
          : { percent: g.percentUsed, reset: formatUntil(g.resetAt, now) },
    },
  };
}

/** OpenCode's three windows; a lapsed Go subscription grays the row and says so. */
export function openCodeRow(row: OpenCodeAccountUsage, now: number): TableRowData {
  const rejected = isAuthRejected(row.authExpired, row.error);
  const failure = errorHint(row.error, "add the key again in Settings");
  const lapsed = row.subscriptionActive === false;
  const warning = rejected
    ? { text: "key rejected", title: "OpenCode rejected this API key · add a new key in Settings" }
    : lapsed
      ? { text: "not active", title: "The OpenCode Go subscription is not active" }
      : failure && { text: failure.short, title: failure.long };
  const u = row.usage;
  const plan = planOf("Go");
  if (!u) return { warning, cells: {}, note: "Unavailable", plan };
  const cell = (w: "rolling" | "weekly" | "monthly"): UsageCell =>
    isUnknownWindow(u[w], row.stale, now)
      ? { percent: null, reset: null }
      : { percent: u[w].percent, reset: formatUntil(u[w].resetsAt, now) };
  return {
    warning,
    note: null,
    plan,
    dimmed: lapsed,
    cells: { rolling: cell("rolling"), weekly: cell("weekly"), monthly: cell("monthly") },
  };
}

const ALL_COLUMNS = {
  cursor: [
    { id: "firstParty", name: "First Party" },
    { id: "api", name: "API" },
  ],
  grok: [{ id: "weekly", name: "Weekly" }],
  opencode: [
    { id: "rolling", name: "Rolling" },
    { id: "weekly", name: "Weekly" },
    { id: "monthly", name: "Monthly" },
  ],
} as const;

/** Cursor, Grok Bot and OpenCode columns: every bar minus the hidden ones. */
export function fixedColumns(
  provider: keyof typeof ALL_COLUMNS,
  hidden: HiddenBars,
): TableColumn[] {
  const shown = shownBars(provider, hidden) as ReadonlySet<string>;
  return ALL_COLUMNS[provider].filter((c) => shown.has(c.id));
}
