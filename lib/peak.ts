/**
 * DeepSeek peak windows (UTC, weekdays only): 01:00–04:00 and 06:00–10:00.
 * Either the current window's end, or the next window's start.
 */
export const PEAK_WINDOWS_UTC: readonly (readonly [number, number])[] = [
  [1, 4],
  [6, 10],
]

export interface PeakInfo {
  active: boolean
  endAt: number | null
  nextStartAt: number | null
}

function utcAt(day: Date, hour: number): number {
  return Date.UTC(day.getUTCFullYear(), day.getUTCMonth(), day.getUTCDate(), hour, 0, 0)
}

const isWeekday = (d: Date) => d.getUTCDay() >= 1 && d.getUTCDay() <= 5

export function getPeakInfo(now: Date): PeakInfo {
  const t = now.getTime()
  if (isWeekday(now)) {
    for (const [start, end] of PEAK_WINDOWS_UTC) {
      if (t >= utcAt(now, start) && t < utcAt(now, end)) {
        return { active: true, endAt: utcAt(now, end), nextStartAt: null }
      }
    }
  }
  // Next window start: later today, or the first window of the next weekday.
  for (let offset = 0; offset <= 7; offset++) {
    const day = new Date(t + offset * 86_400_000)
    if (!isWeekday(day)) continue
    for (const [start] of PEAK_WINDOWS_UTC) {
      const at = utcAt(day, start)
      if (at > t) return { active: false, endAt: null, nextStartAt: at }
    }
  }
  return { active: false, endAt: null, nextStartAt: null }
}
