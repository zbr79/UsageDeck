import { promises as fs } from 'fs'
import path from 'path'
import os from 'os'
import { readCached, writeCached } from './usage-cache'
import { getGrokUsage } from './grokbot'

const STATE_DB = path.join(
  process.env.APPDATA ?? path.join(os.homedir(), 'AppData', 'Roaming'),
  'Cursor',
  'User',
  'globalStorage',
  'state.vscdb',
)
const USAGE_URL =
  'https://api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage'
const PLAN_URL = 'https://api2.cursor.sh/aiserver.v1.DashboardService/GetPlanInfo'
const SAND_URL =
  'https://api2.cursor.sh/aiserver.v1.DashboardService/GetSandUsageStatus'

export interface CursorUsage {
  autoPercentUsed: number | null
  apiPercentUsed: number | null
  totalPercentUsed: number | null
  billingCycleEnd: string | null
  totalSpend: number | null
  limit: number | null
  bonusSpend: number | null
  accountName: string | null
  planName: string | null
  grokPercentUsed: number | null
  grokResetAt: string | null
  grokAccount: string | null
  grokPlanLabel: string | null
}

export interface CursorUsageResult {
  usage: CursorUsage | null
  stale: boolean
  cachedAt: number | null
}

let tokenCache: { at: number; token: string } | null = null
let usageCache: {
  at: number
  data: CursorUsage | null
  stale: boolean
  cachedAt: number | null
} | null = null

function toIsoTimestamp(value: unknown): string | null {
  if (typeof value === 'number' && Number.isFinite(value)) {
    const ms = value < 1e12 ? value * 1000 : value
    const d = new Date(ms)
    return Number.isNaN(d.getTime()) ? null : d.toISOString()
  }
  if (typeof value === 'string' && value.trim()) {
    const trimmed = value.trim()
    if (/^\d+(\.\d+)?$/.test(trimmed)) return toIsoTimestamp(Number(trimmed))
    const d = new Date(trimmed)
    return Number.isNaN(d.getTime()) ? null : d.toISOString()
  }
  return null
}

async function readState(fn: (db: any) => string | null): Promise<string | null> {
  try {
    // @ts-expect-error node:sqlite types not in @types/node 20
    const mod = await import('node:sqlite')
    const db = new mod.DatabaseSync(STATE_DB, { readOnly: true })
    const value = fn(db)
    db.close()
    return value
  } catch {
    return null
  }
}

async function getAccountName(): Promise<string | null> {
  const email = await readState((db) => {
    const row = db
      .prepare('SELECT value FROM ItemTable WHERE key = ?')
      .get('cursorAuth/cachedEmail') as { value: string } | undefined
    return row?.value ?? null
  })
  if (email) return email
  const profile = await readState((db) => {
    const row = db
      .prepare('SELECT value FROM ItemTable WHERE key = ?')
      .get('cursorAuth/cachedScopedProfile') as { value: string } | undefined
    try {
      return row?.value ? (JSON.parse(row.value)?.displayName ?? null) : null
    } catch {
      return null
    }
  })
  return profile
}

async function getToken(): Promise<string | null> {
  if (tokenCache && Date.now() - tokenCache.at < 60_000) return tokenCache.token
  try {
    // @ts-expect-error node:sqlite types not in @types/node 20
    const mod = await import('node:sqlite')
    const db = new mod.DatabaseSync(STATE_DB, { readOnly: true })
    const row = db
      .prepare('SELECT value FROM ItemTable WHERE key = ?')
      .get('cursorAuth/accessToken') as { value: string } | undefined
    db.close()
    if (row?.value) {
      tokenCache = { at: Date.now(), token: row.value }
      return row.value
    }
  } catch {
    // fall through to raw scan
  }
  try {
    const raw = await fs.readFile(STATE_DB, 'utf8')
    const m = raw.match(/cursorAuth\/accessToken[^"]*"([A-Za-z0-9._-]{40,})"/)
    if (m?.[1]) {
      tokenCache = { at: Date.now(), token: m[1] }
      return m[1]
    }
  } catch {
    // no cursor install / locked db
  }
  return null
}

export async function getCursorUsage(): Promise<CursorUsageResult> {
  async function fromCache(): Promise<CursorUsageResult> {
    const cached = await readCached<CursorUsage>('cursor', 'default')
    usageCache = {
      at: Date.now(),
      data: cached?.data ?? null,
      stale: cached !== null,
      cachedAt: cached?.savedAt ?? null,
    }
    return {
      usage: usageCache.data,
      stale: usageCache.stale,
      cachedAt: usageCache.cachedAt,
    }
  }

  if (usageCache && Date.now() - usageCache.at < 30_000) {
    return { usage: usageCache.data, stale: usageCache.stale, cachedAt: usageCache.cachedAt }
  }
  try {
    const token = await getToken()
    if (!token) return await fromCache()
    const res = await fetch(USAGE_URL, {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${token}`,
        'Content-Type': 'application/json',
        Accept: 'application/json',
        'User-Agent': 'Mozilla/5.0',
      },
      body: '{}',
      cache: 'no-store',
      signal: AbortSignal.timeout(10_000),
    })
    if (!res.ok) return await fromCache()
    const json = await res.json()
    const pu = json?.planUsage
    let planName: string | null = null
    let grokPercentUsed: number | null = null
    let grokResetAt: string | null = null
    let grokAccount: string | null = null
    let grokPlanLabel: string | null = null
    const headers = {
      Authorization: `Bearer ${token}`,
      'Content-Type': 'application/json',
      Accept: 'application/json',
      'User-Agent': 'Mozilla/5.0',
    }
    const [planRes, grok] = await Promise.all([
      fetch(PLAN_URL, { method: 'POST', headers, body: '{}', cache: 'no-store', signal: AbortSignal.timeout(10_000) }).catch(() => null),
      getGrokUsage().catch(() => null),
    ])
    if (planRes?.ok) {
      const pj = await planRes.json()
      planName = pj?.planInfo?.planName ?? null
    }
    // Grok Bot has its own account switcher (separate from Cursor). Prefer the
    // account currently selected in the Grok Bot app; fall back to Cursor's token.
    if (grok?.usage && grok.usage.percentUsed != null) {
      grokPercentUsed = grok.usage.percentUsed
      grokResetAt = grok.usage.resetAt
      grokAccount = grok.usage.email
      grokPlanLabel = grok.usage.planLabel
    } else {
      const sandRes = await fetch(SAND_URL, {
        method: 'POST',
        headers,
        body: '{}',
        cache: 'no-store',
        signal: AbortSignal.timeout(10_000),
      }).catch(() => null)
      if (sandRes?.ok) {
        const sj = await sandRes.json()
        grokPercentUsed = typeof sj?.usagePercent === 'number' ? sj.usagePercent : null
        grokResetAt =
          typeof sj?.nextResetTimestampUtc === 'string'
            ? new Date(sj.nextResetTimestampUtc).toISOString()
            : null
        grokPlanLabel = typeof sj?.grokPlanLabel === 'string' ? sj.grokPlanLabel : null
      }
    }
    const data: CursorUsage = {
      autoPercentUsed: typeof pu?.autoPercentUsed === 'number' ? pu.autoPercentUsed : null,
      apiPercentUsed: typeof pu?.apiPercentUsed === 'number' ? pu.apiPercentUsed : null,
      totalPercentUsed: typeof pu?.totalPercentUsed === 'number' ? pu.totalPercentUsed : null,
      billingCycleEnd: toIsoTimestamp(json?.billingCycleEnd),
      totalSpend: typeof pu?.totalSpend === 'number' ? pu.totalSpend : null,
      limit: typeof pu?.limit === 'number' ? pu.limit : null,
      bonusSpend: typeof pu?.bonusSpend === 'number' ? pu.bonusSpend : null,
      accountName: await getAccountName(),
      planName,
      grokPercentUsed,
      grokResetAt,
      grokAccount,
      grokPlanLabel,
    }
    writeCached('cursor', 'default', data)
    usageCache = { at: Date.now(), data, stale: false, cachedAt: null }
    return { usage: data, stale: false, cachedAt: null }
  } catch {
    return await fromCache()
  }
}