import { rankAccounts } from "@/lib/ranking";
import { usedUp } from "@/lib/used-up";
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
      Number(!!usedUp(id, a, now)) - Number(!!usedUp(id, b, now)) ||
      (b.cachedAt ?? 0) - (a.cachedAt ?? 0),
  );
}
