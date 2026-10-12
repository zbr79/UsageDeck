import { describe, expect, it } from 'vitest'
import { formatAgo, formatDuration, formatPercent, formatUntil } from '@/lib/format'
import { getPeakInfo } from '@/lib/peak'
import { increasedWindows, isAccountExhausted, rankAccounts } from '@/lib/ranking'
import type { OpenCodeAccountUsage } from '@/lib/types'
import { nextBrightness, resolveWidgetTheme } from '@/app/widget/_lib/theme'

const M = 60_000
const H = 60 * M
const D = 24 * H

describe('formatDuration', () => {
  it('starts at the largest unit and caps the unit count', () => {
    expect(formatDuration(15 * D + 20 * H + 28 * M)).toBe('15d 20h 28m')
    expect(formatDuration(15 * D + 20 * H + 28 * M, 2)).toBe('15d 20h')
    expect(formatDuration(61 * H + 5 * M, 2)).toBe('2d 13h') // was "61h 5m"
    expect(formatDuration(4 * H + 40 * M)).toBe('4h 40m')
    expect(formatDuration(3 * M + 59_000)).toBe('3m')
    expect(formatDuration(6 * D, 1)).toBe('6d')
  })
  it('never goes negative', () => {
    expect(formatDuration(0)).toBe('0m')
    expect(formatDuration(-5 * M)).toBe('0m')
    expect(formatDuration(NaN)).toBe('0m')
  })
  it('formatUntil handles bad input', () => {
    expect(formatUntil(null, 0)).toBeNull()
    expect(formatUntil('nope', 0)).toBeNull()
    expect(formatUntil(new Date(2 * H).toISOString(), 0)).toBe('2h 0m')
  })
})

describe('formatPercent / formatAgo', () => {
  it('never rounds 99.x up to 100', () => {
    expect(formatPercent(99.96)).toBe('100')
    expect(formatPercent(99.4)).toBe('99.4')
    expect(formatPercent(42.4)).toBe('42')
  })
  it('describes recency', () => {
    expect(formatAgo(1000, 30_000)).toBe('just now')
    expect(formatAgo(0, 3 * M)).toBe('3m ago')
    expect(formatAgo(0, 2 * H)).toBe('2h ago')
    expect(formatAgo(0, 3 * D)).toBe('3d ago')
  })
})

describe('getPeakInfo (UTC weekdays 01–04 and 06–10)', () => {
  const at = (iso: string) => getPeakInfo(new Date(iso))
  it('reports the end of an active window', () => {
    expect(at('2026-10-12T02:00:00Z')).toEqual({ active: true, endAt: Date.parse('2026-10-12T04:00:00Z'), nextStartAt: null })
    expect(at('2026-10-12T09:59:00Z').endAt).toBe(Date.parse('2026-10-12T10:00:00Z'))
  })
  it('finds the next start: same day, next day, or Monday', () => {
    expect(at('2026-10-12T00:57:00Z').nextStartAt).toBe(Date.parse('2026-10-12T01:00:00Z'))
    expect(at('2026-10-12T05:00:00Z').nextStartAt).toBe(Date.parse('2026-10-12T06:00:00Z'))
    expect(at('2026-10-12T12:00:00Z').nextStartAt).toBe(Date.parse('2026-10-13T01:00:00Z'))
    expect(at('2026-10-16T12:00:00Z').nextStartAt).toBe(Date.parse('2026-10-19T01:00:00Z')) // Fri → Mon
    expect(at('2026-10-17T02:00:00Z')).toMatchObject({ active: false }) // Saturday
  })
})

const NOW = Date.parse('2026-10-12T00:00:00Z')
const win = (percent: number, resetsIn = D) => ({
  status: 'ok',
  percent,
  resetsAt: new Date(NOW + resetsIn).toISOString(),
})
function row(email: string, weekly: number, monthly: number, extra: Partial<OpenCodeAccountUsage> = {}): OpenCodeAccountUsage {
  return {
    email,
    usage: { rolling: win(0), weekly: win(weekly, 2 * D), monthly: win(monthly, 10 * D) },
    prev: null,
    enabled: true,
    error: null,
    stale: false,
    cachedAt: null,
    subscriptionActive: true,
    lastChangeAt: null,
    lastChangeDetail: null,
    ...extra,
  }
}

describe('rankAccounts', () => {
  it('hides disabled/lapsed, sinks exhausted (soonest reset first), recent first', () => {
    const used: Record<string, number> = { a: 1, b: 5 }
    const rows = [
      row('a', 10, 10),
      row('b', 10, 10),
      row('late', 0, 100),
      row('soon', 100, 0),
      row('off', 0, 0, { enabled: false }),
      row('lapsed', 0, 0, { subscriptionActive: false }),
    ]
    expect(rankAccounts(rows, (e) => used[e] ?? 0, NOW).map((r) => r.email)).toEqual(['b', 'a', 'soon', 'late'])
  })

  it('stale data whose window already reset is not "exhausted"', () => {
    const r = row('x', 100, 0, { stale: true })
    r.usage!.weekly.resetsAt = new Date(NOW - 1000).toISOString()
    expect(isAccountExhausted(r, NOW)).toBe(false)
  })

  it('increasedWindows compares against the previous snapshot', () => {
    const cur = row('x', 20, 30).usage!
    const prev = row('x', 10, 30).usage!
    expect(increasedWindows(cur, prev)).toEqual(['weekly'])
    expect(increasedWindows(cur, null)).toEqual([])
  })
})

describe('widget theme', () => {
  it('fixed modes ignore brightness; auto puts light cards on dark screens', () => {
    expect(resolveWidgetTheme('white', 0.9)).toBe('white')
    expect(resolveWidgetTheme('dark', 0.1)).toBe('dark')
    expect(resolveWidgetTheme('auto', null)).toBe('dark')
    expect(resolveWidgetTheme('auto', 0.2)).toBe('light')
    expect(resolveWidgetTheme('auto', 0.8)).toBe('dark')
  })
  it('brightness has hysteresis', () => {
    expect(nextBrightness(null, 0.4)).toBe(0.4) // first sample always taken
    expect(nextBrightness(0.8, 0.35)).toBe(0.8) // not dark enough to flip to light cards
    expect(nextBrightness(0.8, 0.2)).toBe(0.2)
    expect(nextBrightness(0.2, 0.4)).toBe(0.2) // not bright enough to flip back
    expect(nextBrightness(0.2, 0.6)).toBe(0.6)
  })
})
