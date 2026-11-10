import { describe, expect, it } from 'vitest'
import { toIsoTimestamp } from '@/lib/time'

describe('toIsoTimestamp', () => {
  it('treats numbers below 1e12 as unix seconds', () => {
    expect(toIsoTimestamp(1_700_000_000)).toBe(new Date(1_700_000_000 * 1000).toISOString())
  })

  it('treats numbers at or above 1e12 as unix milliseconds', () => {
    expect(toIsoTimestamp(1_700_000_000_000)).toBe(new Date(1_700_000_000_000).toISOString())
  })

  it('parses numeric strings and ISO strings', () => {
    expect(toIsoTimestamp('1700000000')).toBe(new Date(1_700_000_000 * 1000).toISOString())
    expect(toIsoTimestamp(' 2026-09-28T01:02:03.000Z ')).toBe('2026-09-28T01:02:03.000Z')
  })

  it('returns null for values that are not timestamps', () => {
    expect(toIsoTimestamp('not-a-date')).toBeNull()
    expect(toIsoTimestamp('')).toBeNull()
    expect(toIsoTimestamp(null)).toBeNull()
    expect(toIsoTimestamp(Number.NaN)).toBeNull()
  })
})
