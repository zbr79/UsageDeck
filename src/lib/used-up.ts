import { isAccountExhausted, isPassed, usableAgainAt } from "./ranking";
import type {
  ClaudeUsage,
  CodexUsage,
  CursorUsage,
  GrokUsage,
  OpenCodeAccountUsage,
  ProviderId,
} from "./types";

// When a card is used up (its red ring), and when it can be used again. The widget cards, the
// settings order and the widget's "hide until it resets" all read this one rule.

export interface UsageOf {
  cursor: CursorUsage;
  grok: GrokUsage;
  codex: CodexUsage;
  claude: ClaudeUsage;
}

/** An account's last fetched usage, and whether it is a cached copy. */
export interface UsageSnapshot<T> {
  usage: T | null;
  stale: boolean;
}

/**
 * A card that cannot be used right now. `until` (epoch ms) is when it can again: the latest reset
 * among its full windows, so a card full on both its 5-hour and weekly windows waits for the
 * weekly one. Null when a full window has no known reset time.
 */
export interface UsedUp {
  until: number | null;
}

interface LimitWindow {
  full: boolean;
  resetsAt: string | null | undefined;
}

/** The windows that can stop each provider. */
const WINDOWS: { [P in ProviderId]: (usage: UsageOf[P]) => LimitWindow[] } = {
  cursor: (c) => [{ full: (c.totalPercentUsed ?? 0) >= 100, resetsAt: c.billingCycleEnd }],
  grok: (g) => [{ full: (g.percentUsed ?? 0) >= 100, resetsAt: g.resetAt }],
  codex: (x) => [
    { full: x.limitReached || (x.usedPercent ?? 0) >= 100, resetsAt: x.resetAt },
    {
      full: x.weeklyUsedPercent != null && x.weeklyUsedPercent >= 100,
      resetsAt: x.weeklyResetAt,
    },
  ],
  claude: (c) =>
    [c.fiveHour, c.weekly].map((w) => ({
      full: w?.percent != null && w.percent >= 100,
      resetsAt: w?.resetsAt,
    })),
};

/** Null while the account can be used. A cached window whose reset has passed tells nothing. */
export function usedUp<P extends ProviderId>(
  provider: P,
  snapshot: UsageSnapshot<UsageOf[P]>,
  now: number,
): UsedUp | null {
  if (!snapshot.usage) return null;
  const full = WINDOWS[provider](snapshot.usage).filter(
    (w) => w.full && !(snapshot.stale && isPassed(w.resetsAt, now)),
  );
  if (full.length === 0) return null;
  const resets = full.map((w) => (w.resetsAt ? new Date(w.resetsAt).getTime() : NaN));
  return { until: resets.every(Number.isFinite) ? Math.max(...resets) : null };
}

/** `usedUp` for an OpenCode account: its weekly and monthly windows, as the widget ranks them. */
export function openCodeUsedUp(row: OpenCodeAccountUsage, now: number): UsedUp | null {
  if (!isAccountExhausted(row, now)) return null;
  return { until: usableAgainAt(row, now) || null };
}

/**
 * Whether the widget hides a card the user hid until it resets: until that time, and only while
 * it is still used up, so a provider that resets early brings the card back early.
 */
export function isSnoozed(
  snoozedUntil: Readonly<Record<string, number>>,
  key: string,
  used: UsedUp | null,
  now: number,
): boolean {
  const until = Object.hasOwn(snoozedUntil, key) ? snoozedUntil[key] : undefined;
  return until !== undefined && now < until && used !== null;
}
