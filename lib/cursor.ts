import path from 'path'
import os from 'os'
import { toIsoTimestamp } from '@/lib/time'
import { logWarn } from '@/lib/log'

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
}

export function parseCursorUsage(
  usageJson: Record<string, unknown> | null | undefined,
  planJson?: Record<string, unknown> | null,
  sandJson?: Record<string, unknown> | null,
  accountName: string | null = null,
): CursorUsage {
  const pu = (usageJson?.planUsage ?? null) as Record<string, unknown> | null
  const planInfo = (planJson?.planInfo ?? null) as Record<string, unknown> | null
  const planName = typeof planInfo?.planName === 'string' ? planInfo.planName : null
  const grokPercentUsed =
    typeof sandJson?.usagePercent === 'number' ? sandJson.usagePercent : null
  let grokResetAt: string | null = null
  if (typeof sandJson?.nextResetTimestampUtc === 'string') {
    const parsed = new Date(sandJson.nextResetTimestampUtc)
    if (!Number.isNaN(parsed.getTime())) {
      grokResetAt = parsed.toISOString()
    }
  }
  return {
    autoPercentUsed: typeof pu?.autoPercentUsed === 'number' ? pu.autoPercentUsed : null,
    apiPercentUsed: typeof pu?.apiPercentUsed === 'number' ? pu.apiPercentUsed : null,
    totalPercentUsed: typeof pu?.totalPercentUsed === 'number' ? pu.totalPercentUsed : null,
    billingCycleEnd: toIsoTimestamp(usageJson?.billingCycleEnd),
    totalSpend: typeof pu?.totalSpend === 'number' ? pu.totalSpend : null,
    limit: typeof pu?.limit === 'number' ? pu.limit : null,
    bonusSpend: typeof pu?.bonusSpend === 'number' ? pu.bonusSpend : null,
    accountName,
    planName,
    grokPercentUsed,
    grokResetAt,
  }
}

let tokenCache: { at: number; token: string } | null = null
let usageCache: { at: number; data: CursorUsage | null } | null = null

interface CursorStateDb {
  prepare: (sql: string) => {
    get: (key: string) => { value?: string } | undefined
  }
  close: () => void
}

async function readState(fn: (db: CursorStateDb) => string | null): Promise<string | null> {
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
  } catch (err) {
    logWarn('cursor', 'Could not read the Cursor auth token from state.vscdb', err)
  }
  return null
}

export async function getCursorUsage(): Promise<CursorUsage | null> {
  if (usageCache && Date.now() - usageCache.at < 30_000) return usageCache.data
  try {
    const token = await getToken()
    if (!token) {
      usageCache = { at: Date.now(), data: null }
      return null
    }
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
    if (!res.ok) {
      logWarn('cursor', `GetCurrentPeriodUsage returned HTTP ${res.status}`)
      usageCache = { at: Date.now(), data: null }
      return null
    }
    const json = await res.json()
    let planJson: Record<string, unknown> | null = null
    let sandJson: Record<string, unknown> | null = null
    const headers = {
      Authorization: `Bearer ${token}`,
      'Content-Type': 'application/json',
      Accept: 'application/json',
      'User-Agent': 'Mozilla/5.0',
    }
    const [planRes, sandRes] = await Promise.all([
      fetch(PLAN_URL, { method: 'POST', headers, body: '{}', cache: 'no-store', signal: AbortSignal.timeout(10_000) }).catch(() => null),
      fetch(SAND_URL, { method: 'POST', headers, body: '{}', cache: 'no-store', signal: AbortSignal.timeout(10_000) }).catch(() => null),
    ])
    if (planRes?.ok) {
      planJson = await planRes.json()
    }
    if (sandRes?.ok) {
      sandJson = await sandRes.json()
    }
    const data = parseCursorUsage(json, planJson, sandJson, await getAccountName())
    usageCache = { at: Date.now(), data }
    return data
  } catch (err) {
    logWarn('cursor', 'Failed to fetch Cursor usage', err)
    usageCache = { at: Date.now(), data: null }
    return null
  }
}
