import { isPassed, rankAccounts } from "@/lib/ranking";
import type {
  ClaudeUsage,
  CodexUsage,
  CursorUsage,
  GrokUsage,
  OpenCodeAccountUsage,
  ProviderId,
  TrackedAccount,
} from "@/lib/types";

// Settings card order: what the widget shows comes first, in the widget's own order; the
// accounts that don't work on the widget come after it.

/** When the account last changed: the widget's "last used" clock for OpenCode. */
const lastUsed = (r: OpenCodeAccountUsage) => r.lastChangeAt ?? 0;

/**
 * The widget's ranking applied to accounts it would not show (hidden, lapsed, no data), so the
 * same rules order them within their group. Returns the original rows.
 */
function rankAll(rows: OpenCodeAccountUsage[], now: number): OpenCodeAccountUsage[] {
  const byEmail = new Map(rows.map((r) => [r.email, r]));
  const lastOf = (email: string) => lastUsed(byEmail.get(email)!);
  return rankAccounts(
    rows.map((r) => ({ ...r, enabled: true, subscriptionActive: null })),
    lastOf,
    now,
  ).map((r) => byEmail.get(r.email)!);
}

/**
 * OpenCode cards: the accounts the widget shows, in `rankAccounts` order; then hidden accounts,
 * then lapsed subscriptions, then accounts with no usage data.
 */
export function orderOpenCode(
  rows: OpenCodeAccountUsage[],
  hidden: (email: string) => boolean,
  now = Date.now(),
): OpenCodeAccountUsage[] {
  const isHidden = (r: OpenCodeAccountUsage) => hidden(r.email) || !r.enabled;
  const isLapsed = (r: OpenCodeAccountUsage) => r.subscriptionActive === false;
  const hiddenRows = rows.filter(isHidden);
  const lapsedRows = rows.filter((r) => !isHidden(r) && isLapsed(r));
  const noDataRows = rows.filter((r) => !isHidden(r) && !isLapsed(r) && !r.usage);
  const shown = rankAccounts(
    rows.filter((r) => !isHidden(r) && r.usage),
    (email) => lastUsed(rows.find((r) => r.email === email)!),
    now,
  );
  return [
    ...shown,
    ...rankAll(hiddenRows, now),
    ...rankAll(lapsedRows, now),
    ...rankAll(noDataRows, now),
  ];
}

type AnyUsage = CursorUsage | GrokUsage | CodexUsage | ClaudeUsage;

/**
 * Whether the card shows the account as used up (its red ring). Mirrors the `exhausted` checks of
 * the widget cards in src/widget/_components/cards.tsx.
 */
export function isUsedUp(
  id: ProviderId,
  account: TrackedAccount<AnyUsage>,
  now = Date.now(),
): boolean {
  const stale = account.stale;
  const past = (iso: string | null | undefined) => stale && isPassed(iso, now);
  switch (id) {
    case "cursor": {
      const c = account.usage as CursorUsage | null;
      return !!c && !past(c.billingCycleEnd) && (c.totalPercentUsed ?? 0) >= 100;
    }
    case "grok": {
      const g = account.usage as GrokUsage | null;
      return !!g && !past(g.resetAt) && (g.percentUsed ?? 0) >= 100;
    }
    case "codex": {
      const x = account.usage as CodexUsage | null;
      if (!x) return false;
      const hasWeekly = x.weeklyUsedPercent != null;
      return (
        (!past(x.resetAt) && (x.limitReached || (x.usedPercent ?? 0) >= 100)) ||
        (hasWeekly && !past(x.weeklyResetAt) && (x.weeklyUsedPercent ?? 0) >= 100)
      );
    }
    case "claude": {
      const c = account.usage as ClaudeUsage | null;
      if (!c) return false;
      return [c.fiveHour, c.weekly].some(
        (w) => w?.percent != null && !past(w.resetsAt) && w.percent >= 100,
      );
    }
    default:
      return false;
  }
}

/**
 * Claude / Codex / Cursor / Grok cards: shown accounts before hidden ones; within each, the
 * signed-in account first, used-up accounts last, then the most recent cached use first.
 */
export function orderTracked(
  id: ProviderId,
  accounts: readonly TrackedAccount<AnyUsage>[],
  hidden: (accountId: string) => boolean,
  now = Date.now(),
): TrackedAccount<AnyUsage>[] {
  return [...accounts].sort(
    (a, b) =>
      Number(hidden(a.id)) - Number(hidden(b.id)) ||
      Number(b.active) - Number(a.active) ||
      Number(isUsedUp(id, a, now)) - Number(isUsedUp(id, b, now)) ||
      (b.cachedAt ?? 0) - (a.cachedAt ?? 0),
  );
}
