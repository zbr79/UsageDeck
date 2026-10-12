import path from 'path'
import os from 'os'
import { readJson } from './fs-atomic'
import { getAppKey, unseal } from './os-crypt'
import { createProvider, type ProviderResult } from './provider-cache'
import { cursorHeaders, readCursorState } from './cursor'
import { numberOrNull, stringOrNull, toIso } from './time'
import type { GrokUsage } from './types'

export type { GrokUsage } from './types'

const GROK_DIR = path.join(
  process.env.APPDATA ?? path.join(os.homedir(), 'AppData', 'Roaming'),
  'Grok Bot',
)
const SECRETS_FILE = path.join(GROK_DIR, 'sand-secrets.json')
const LOCAL_STATE_FILE = path.join(GROK_DIR, 'Local State')
const SAND_URL = 'https://api2.cursor.sh/aiserver.v1.DashboardService/GetSandUsageStatus'

interface GrokAccount {
  token: string
  email: string | null
  name: string | null
  source: GrokUsage['source']
}

/** The account currently selected in the Grok Bot app (read-only). */
async function readGrokBotAccount(): Promise<GrokAccount | null> {
  const secrets = await readJson<Record<string, string>>(SECRETS_FILE)
  let accounts: { active?: string; accounts?: Record<string, Record<string, string>> }
  try {
    accounts = JSON.parse(secrets?.['cursor-accounts'] ?? '')
  } catch {
    return null
  }
  const entry = accounts?.active ? accounts.accounts?.[accounts.active] : undefined
  if (!entry) return null
  const key = await getAppKey(LOCAL_STATE_FILE)
  if (!key) return null
  const token = unseal(key, entry['cursor-access-token'])
  if (!token) return null
  let email: string | null = null
  let name: string | null = null
  const profile = unseal(key, entry['cursor-account-profile'])
  if (profile) {
    try {
      const p = JSON.parse(profile)
      email = stringOrNull(p?.email)
      name = stringOrNull(p?.name)
    } catch {
      // ignore malformed profile
    }
  }
  return { token, email, name, source: 'grok-bot' }
}

/** Grok Bot's own sign-in, falling back to Cursor's (same backend). */
async function readAccount(): Promise<GrokAccount | null> {
  const own = await readGrokBotAccount()
  if (own) return own
  const cursor = await readCursorState()
  return cursor.token
    ? { token: cursor.token, email: cursor.accountName, name: null, source: 'cursor' }
    : null
}

export function parseGrokUsage(json: unknown, account: Omit<GrokAccount, 'token'>): GrokUsage {
  const sj = json as Record<string, unknown> | null
  return {
    email: account.email,
    name: account.name,
    percentUsed: numberOrNull(sj?.usagePercent),
    resetAt: toIso(sj?.nextResetTimestampUtc),
    planLabel: stringOrNull(sj?.grokPlanLabel),
    cursorPlanName: stringOrNull(sj?.cursorPlanName),
    source: account.source,
  }
}

const provider = createProvider<GrokUsage>({
  section: 'grokbot',
  ttlMs: 30_000,
  async fetch() {
    const account = await readAccount()
    if (!account) return { ok: false }
    const res = await fetch(SAND_URL, {
      method: 'POST',
      headers: cursorHeaders(account.token),
      body: '{}',
      cache: 'no-store',
      signal: AbortSignal.timeout(10_000),
    })
    if (res.status === 401) return { ok: false, authExpired: true }
    if (!res.ok) return { ok: false }
    const data = parseGrokUsage(await res.json(), account)
    return data.percentUsed == null ? { ok: false } : { ok: true, data }
  },
})

export function getGrokUsage(): Promise<ProviderResult<GrokUsage>> {
  return provider.get()
}
