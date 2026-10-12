import type { OpenCodeAccountUsage, OpenCodeUsage, UsageWindow } from './types'

// Ranking rules for OpenCode accounts on the widget:
//  1. accounts exhausted on the weekly or monthly window sink to the bottom;
//  2. among those, the one resetting soonest ranks first;
//  3. otherwise the most recently used account floats to the top.

export function isPassed(iso: string | null | undefined, now = Date.now()): boolean {
  if (!iso) return false
  const t = new Date(iso).getTime()
  return Number.isFinite(t) && t <= now
}

/** Cached data whose window has since reset tells us nothing. */
export function isUnknownWindow(w: UsageWindow, stale: boolean, now = Date.now()): boolean {
  return stale && isPassed(w.resetsAt, now)
}

export function isExhaustedWindow(w: UsageWindow, stale = false, now = Date.now()): boolean {
  if (isUnknownWindow(w, stale, now)) return false
  return w.percent >= 100 || w.status === 'exhausted'
}

export function isAccountExhausted(r: OpenCodeAccountUsage, now = Date.now()): boolean {
  return (
    !!r.usage &&
    (isExhaustedWindow(r.usage.monthly, r.stale, now) ||
      isExhaustedWindow(r.usage.weekly, r.stale, now))
  )
}

function earliestReset(r: OpenCodeAccountUsage, now: number): number {
  const u = r.usage
  if (!u) return 0
  const times = (['monthly', 'weekly'] as const)
    .filter((k) => isExhaustedWindow(u[k], r.stale, now))
    .map((k) => new Date(u[k].resetsAt).getTime())
    .filter(Number.isFinite)
  return times.length ? Math.min(...times) : 0
}

/** Visible accounts (enabled, subscription not known-inactive), best first. */
export function rankAccounts(
  rows: OpenCodeAccountUsage[],
  lastUsedAt: (email: string) => number,
  now = Date.now(),
): OpenCodeAccountUsage[] {
  return rows
    .filter((r) => r.enabled && r.subscriptionActive !== false)
    .sort((a, b) => {
      const ea = isAccountExhausted(a, now)
      const eb = isAccountExhausted(b, now)
      if (ea !== eb) return ea ? 1 : -1
      if (ea) return earliestReset(a, now) - earliestReset(b, now)
      return lastUsedAt(b.email) - lastUsedAt(a.email)
    })
}

export const WINDOW_KEYS = ['rolling', 'weekly', 'monthly'] as const

/** Windows whose usage went up between two snapshots. */
export function increasedWindows(
  cur: OpenCodeUsage,
  prev: OpenCodeUsage | null,
): (typeof WINDOW_KEYS)[number][] {
  if (!prev) return []
  return WINDOW_KEYS.filter(
    (k) => !isExhaustedWindow(cur[k]) && cur[k].percent > prev[k].percent,
  )
}
