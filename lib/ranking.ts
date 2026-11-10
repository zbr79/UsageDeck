export interface UsageWindow {
  status: string
  percent: number
  resetsAt: string
}

export interface PeakInfo {
  active: boolean
  endAt: number | null
  nextStartAt: number | null
}

interface RankableAccount {
  email: string
  enabled?: boolean
  usage: { monthly: UsageWindow; weekly: UsageWindow } | null
}

export function isExhaustedWindow(w: UsageWindow): boolean {
  return w.percent >= 100 || w.status === 'exhausted'
}

export function isExhausted(r: RankableAccount): boolean {
  return !!r.usage && (isExhaustedWindow(r.usage.monthly) || isExhaustedWindow(r.usage.weekly))
}

export function earliestReset(r: RankableAccount): number {
  const u = r.usage
  if (!u) return 0
  const times: number[] = []
  if (isExhaustedWindow(u.monthly)) times.push(new Date(u.monthly.resetsAt).getTime())
  if (isExhaustedWindow(u.weekly)) times.push(new Date(u.weekly.resetsAt).getTime())
  return times.length ? Math.min(...times) : 0
}

export function sortAccounts<T extends RankableAccount>(
  rows: readonly T[],
  lastChangedAt: (row: T) => number,
): T[] {
  return rows
    .filter((r) => r.enabled !== false)
    .sort((a, b) => {
      const ea = isExhausted(a)
      const eb = isExhausted(b)
      if (ea !== eb) return ea ? 1 : -1
      if (ea) return earliestReset(a) - earliestReset(b)
      return lastChangedAt(b) - lastChangedAt(a)
    })
}

export function resetsIn(resetsAt: string, now = Date.now()): string {
  const ms = new Date(resetsAt).getTime() - now
  if (ms <= 0) return 'now'
  const h = Math.floor(ms / 3600000)
  const m = Math.floor((ms % 3600000) / 60000)
  if (h >= 24) return `${Math.floor(h / 24)}d ${h % 24}h`
  return `${h}h ${m}m`
}

function utcOf(d: Date, hour: number): number {
  return Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate(), hour, 0, 0)
}

export function getPeakInfo(now: Date): PeakInfo {
  const h = now.getUTCHours()
  const day = now.getUTCDay()
  const isWeekday = day >= 1 && day <= 5

  if (isWeekday) {
    if (h >= 1 && h < 4) return { active: true, endAt: utcOf(now, 4), nextStartAt: null }
    if (h >= 6 && h < 10) return { active: true, endAt: utcOf(now, 10), nextStartAt: null }
  }

  const candidates: number[] = []
  if (isWeekday) {
    if (h < 1) candidates.push(utcOf(now, 1))
    if (h >= 4 && h < 6) candidates.push(utcOf(now, 6))
  }
  for (let i = 1; i <= 7; i++) {
    const d = new Date(now.getTime() + i * 86400000)
    if (d.getUTCDay() >= 1 && d.getUTCDay() <= 5) {
      candidates.push(utcOf(d, 1))
      break
    }
  }
  const next = candidates.filter((c) => c > now.getTime()).sort((a, b) => a - b)[0] ?? null
  return { active: false, endAt: null, nextStartAt: next }
}

export function fmtDuration(ms: number): string {
  if (ms <= 0) return '0m'
  const m = Math.floor(ms / 60000)
  const h = Math.floor(m / 60)
  const rem = m % 60
  if (h > 0) return `${h}h ${rem}m`
  return `${rem}m`
}

export function resetFmt(resetsAt: string, now = Date.now()): string {
  const ms = new Date(resetsAt).getTime() - now
  if (ms <= 0) return '0m'
  const m = Math.floor(ms / 60000)
  const d = Math.floor(m / 1440)
  const h = Math.floor((m % 1440) / 60)
  const mm = m % 60
  if (d > 0) return `${d}d ${h}h ${mm}m`
  if (h > 0) return `${h}h ${mm}m`
  return `${mm}m`
}
