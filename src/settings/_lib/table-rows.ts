// The Claude and Codex accounts tables: which usage columns they show and each account's cells.
// Pure, so it is unit-tested (tests/table-rows.test.ts).
import { claudeBars, codexBars, shownBars, type HiddenBars } from "@/lib/bars";
import { claudeResetRows, codexResetRows, type ResetRow } from "@/lib/claude-resets";
import { cardErrorKind, errorHint, isAuthRejected } from "@/lib/errors";
import { capitalize, claudePlanLabel, formatUntil } from "@/lib/format";
import { isPassed } from "@/lib/ranking";
import type { ClaudeUsage, CodexUsage, TrackedAccount } from "@/lib/types";
import type { TableColumn, UsageCell } from "../_components/AccountTable";

export type TableProvider = "claude" | "codex";

export interface TableRowData {
  cells: Partial<Record<string, UsageCell>>;
  /** replaces the cells: no usage, or not signed in */
  note: string | null;
  plan: string | null;
  resets: ResetRow[];
  warning: { text: string; title: string } | null;
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
  if (!c) return { ...base, note: "Not signed in or unavailable", plan: null, resets: [] };
  if (!c.signedIn) return { ...base, note: "Not signed in", plan: null, resets: [] };
  const scoped = c.weeklyScoped[0];
  return {
    ...base,
    note: null,
    plan: claudePlanLabel(
      c.planName ?? (c.subscribed ? (c.subscriptionType ?? "subscribed") : "Free"),
    ),
    resets: c.resets ? claudeResetRows(c.resets) : [],
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
    return { warning, cells: {}, note: "Not signed in or unavailable", plan: null, resets: [] };
  }
  return {
    warning,
    note: null,
    plan: x.planType && capitalize(x.planType),
    resets: codexResetRows(x),
    cells: {
      fiveHour: cell(x.usedPercent, x.resetAt, account.stale, now),
      weekly: cell(x.weeklyUsedPercent, x.weeklyResetAt, account.stale, now),
    },
  };
}
