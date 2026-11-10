import { describe, expect, it } from 'vitest'
import { getPeakInfo, resetsIn, sortAccounts, type UsageWindow } from '@/lib/ranking'

function window(percent: number, resetsAt: string, status = 'ok'): UsageWindow {
  return { status, percent, resetsAt }
}

function row(
  email: string,
  opts: {
    enabled?: boolean
    weekly?: UsageWindow
    monthly?: UsageWindow
    usage?: null
  } = {},
) {
  const usage =
    opts.usage === null
      ? null
      : {
          weekly: opts.weekly ?? window(10, '2026-10-08T00:00:00.000Z'),
          monthly: opts.monthly ?? window(10, '2026-11-01T00:00:00.000Z'),
        }
  return { email, enabled: opts.enabled ?? true, usage }
}

describe('sortAccounts', () => {
  it('hides disabled accounts', () => {
    const disabled = row('off@example.com', { enabled: false })
    const active = row('on@example.com')
    const sorted = sortAccounts([disabled, active], () => 0)
    expect(sorted.map((r) => r.email)).toEqual(['on@example.com'])
  })

  it('places monthly or weekly exhaustion below accounts still in quota', () => {
    const monthlyFull = row('month@example.com', {
      monthly: window(100, '2026-11-01T00:00:00.000Z'),
    })
    const weeklyFull = row('week@example.com', {
      weekly: window(40, '2026-10-08T00:00:00.000Z', 'exhausted'),
    })
    const open = row('open@example.com')
    const sorted = sortAccounts([monthlyFull, weeklyFull, open], () => 0)
    expect(sorted[0]?.email).toBe('open@example.com')
    expect(sorted.slice(1).map((r) => r.email).sort()).toEqual([
      'month@example.com',
      'week@example.com',
    ])
  })

  it('ranks exhausted accounts by the soonest reset', () => {
    const later = row('later@example.com', {
      monthly: window(100, '2026-11-20T00:00:00.000Z'),
      weekly: window(100, '2026-10-20T00:00:00.000Z'),
    })
    const sooner = row('sooner@example.com', {
      monthly: window(100, '2026-11-01T00:00:00.000Z'),
      weekly: window(100, '2026-10-02T00:00:00.000Z'),
    })
    const sorted = sortAccounts([later, sooner], () => 0)
    expect(sorted.map((r) => r.email)).toEqual(['sooner@example.com', 'later@example.com'])
  })

  it('floats the most recently changed account when neither is exhausted', () => {
    const older = row('older@example.com')
    const newer = row('newer@example.com')
    const changed = new Map([
      ['older@example.com', 1_000],
      ['newer@example.com', 5_000],
    ])
    const sorted = sortAccounts([older, newer], (r) => changed.get(r.email) ?? 0)
    expect(sorted.map((r) => r.email)).toEqual(['newer@example.com', 'older@example.com'])
  })
})

describe('resetsIn', () => {
  const now = Date.parse('2026-09-28T00:00:00.000Z')

  it('formats hours and minutes under a day', () => {
    expect(resetsIn('2026-09-28T05:07:00.000Z', now)).toBe('5h 7m')
  })

  it('formats whole days once the wait is at least 24 hours', () => {
    expect(resetsIn('2026-09-29T02:00:00.000Z', now)).toBe('1d 2h')
  })

  it('reports a reset that has already passed', () => {
    expect(resetsIn('2026-09-27T23:00:00.000Z', now)).toBe('now')
  })
})

describe('getPeakInfo', () => {
  it('is active on weekday mornings from 01:00 until 04:00 UTC', () => {
    const start = getPeakInfo(new Date('2026-09-28T01:00:00.000Z'))
    const end = getPeakInfo(new Date('2026-09-28T03:59:00.000Z'))
    expect(start).toEqual({
      active: true,
      endAt: Date.parse('2026-09-28T04:00:00.000Z'),
      nextStartAt: null,
    })
    expect(end.active).toBe(true)
    expect(end.endAt).toBe(Date.parse('2026-09-28T04:00:00.000Z'))
  })

  it('is active on weekday mornings from 06:00 until 10:00 UTC', () => {
    const start = getPeakInfo(new Date('2026-09-28T06:00:00.000Z'))
    const end = getPeakInfo(new Date('2026-09-28T09:59:00.000Z'))
    expect(start).toEqual({
      active: true,
      endAt: Date.parse('2026-09-28T10:00:00.000Z'),
      nextStartAt: null,
    })
    expect(end.active).toBe(true)
    expect(end.endAt).toBe(Date.parse('2026-09-28T10:00:00.000Z'))
  })

  it('points at the next window when the current hour is outside peak', () => {
    const beforeFirst = getPeakInfo(new Date('2026-09-28T00:30:00.000Z'))
    const gap = getPeakInfo(new Date('2026-09-28T04:00:00.000Z'))
    const afterSecond = getPeakInfo(new Date('2026-09-28T10:00:00.000Z'))
    const sunday = getPeakInfo(new Date('2026-09-27T15:00:00.000Z'))

    expect(beforeFirst).toEqual({
      active: false,
      endAt: null,
      nextStartAt: Date.parse('2026-09-28T01:00:00.000Z'),
    })
    expect(gap.nextStartAt).toBe(Date.parse('2026-09-28T06:00:00.000Z'))
    expect(afterSecond.nextStartAt).toBe(Date.parse('2026-09-29T01:00:00.000Z'))
    expect(sunday.nextStartAt).toBe(Date.parse('2026-09-28T01:00:00.000Z'))
  })
})
