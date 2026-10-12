// Display formatting shared by the widget and settings. Pure functions.

const MINUTE = 60_000
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR

/**
 * "15d 20h 28m" / "4h 40m" / "3m". Starts at the largest non-zero unit and
 * shows at most `maxUnits` units from there. Zero or negative → "0m".
 */
export function formatDuration(ms: number, maxUnits = 3): string {
  if (!Number.isFinite(ms) || ms <= 0) return '0m'
  const totalMinutes = Math.floor(ms / MINUTE)
  const parts: [number, string][] = [
    [Math.floor(totalMinutes / 1440), 'd'],
    [Math.floor((totalMinutes % 1440) / 60), 'h'],
    [totalMinutes % 60, 'm'],
  ]
  const first = parts.findIndex(([n]) => n > 0)
  if (first === -1) return '0m'
  return parts
    .slice(first, first + maxUnits)
    .map(([n, unit]) => `${n}${unit}`)
    .join(' ')
}

/** Time until an ISO timestamp, formatted; null if unparseable. */
export function formatUntil(iso: string | null | undefined, now: number, maxUnits = 3): string | null {
  if (!iso) return null
  const t = new Date(iso).getTime()
  return Number.isFinite(t) ? formatDuration(t - now, maxUnits) : null
}

/** Whole percent, except 99.x stays precise so "100%" is never shown early. */
export function formatPercent(v: number): string {
  const r = Math.round(v * 10) / 10
  if (r > 99 && r < 100) return r.toFixed(1)
  return String(Math.round(r))
}

export function formatAgo(at: number, now: number): string {
  const ms = now - at
  if (ms < MINUTE) return 'just now'
  if (ms < DAY) return `${formatDuration(ms, 1)} ago`
  return `${Math.floor(ms / DAY)}d ago`
}

/** "plus" → "Plus" */
export function capitalize(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1)
}
