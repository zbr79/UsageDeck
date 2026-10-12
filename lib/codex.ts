import { promises as fs } from 'fs'
import path from 'path'
import os from 'os'
import { readCached, writeCached } from './usage-cache'

// auth.json belongs to the Codex CLI and is only ever read here. Refreshing
// would rotate the single-use refresh token and log the CLI out.
const AUTH_FILE = path.join(os.homedir(), '.codex', 'auth.json')
const USAGE_URL = 'https://chatgpt.com/backend-api/wham/usage'

export interface CodexUsage {
  email: string | null
  planType: string | null
  usedPercent: number | null
  limitWindowSeconds: number | null
  resetAt: string | null
  creditsBalance: number | null
  resetCredits: number | null
  limitReached: boolean
}

export interface CodexUsageResult {
  usage: CodexUsage | null
  stale: boolean
  cachedAt: number | null
  authExpired: boolean
}

let usageCache: { at: number; result: CodexUsageResult } | null = null

interface CodexAuth {
  auth_mode?: string
  tokens?: {
    access_token?: string
    refresh_token?: string
    id_token?: string
    account_id?: string
  }
  last_refresh?: string
}

async function readAuth(): Promise<CodexAuth | null> {
  try {
    return JSON.parse(await fs.readFile(AUTH_FILE, 'utf8'))
  } catch {
    return null
  }
}

async function fetchUsage(accessToken: string, accountId: string | undefined) {
  return fetch(USAGE_URL, {
    headers: {
      Authorization: `Bearer ${accessToken}`,
      ...(accountId ? { 'chatgpt-account-id': accountId } : {}),
      'User-Agent': 'codex-cli/0.44.0',
    },
    cache: 'no-store',
    signal: AbortSignal.timeout(10_000),
  })
}

export async function getCodexUsage(): Promise<CodexUsageResult> {
  async function fromCache(authExpired = false): Promise<CodexUsageResult> {
    const cached = await readCached<CodexUsage>('codex', 'default')
    const result: CodexUsageResult = {
      usage: cached?.data ?? null,
      stale: cached !== null,
      cachedAt: cached?.savedAt ?? null,
      authExpired,
    }
    usageCache = { at: Date.now(), result }
    return result
  }

  if (usageCache && Date.now() - usageCache.at < 30_000) return usageCache.result
  const auth = await readAuth()
  const accessToken = auth?.tokens?.access_token
  if (!auth || !accessToken) return await fromCache()
  try {
    const res = await fetchUsage(accessToken, auth.tokens?.account_id)
    if (res.status === 401 || res.status === 403) return await fromCache(true)
    if (!res.ok) return await fromCache()
    const json = await res.json()
    const rl = json?.rate_limit
    const primary = rl?.primary_window
    const data: CodexUsage = {
      email: typeof json?.email === 'string' ? json.email : null,
      planType: typeof json?.plan_type === 'string' ? json.plan_type : null,
      usedPercent: typeof primary?.used_percent === 'number' ? primary.used_percent : null,
      limitWindowSeconds:
        typeof primary?.limit_window_seconds === 'number' ? primary.limit_window_seconds : null,
      resetAt:
        typeof primary?.reset_at === 'number'
          ? new Date(primary.reset_at * 1000).toISOString()
          : null,
      creditsBalance:
        json?.credits?.balance != null && !Number.isNaN(Number(json.credits.balance))
          ? Number(json.credits.balance)
          : null,
      resetCredits:
        typeof json?.rate_limit_reset_credits?.available_count === 'number'
          ? json.rate_limit_reset_credits.available_count
          : null,
      limitReached: rl?.limit_reached === true,
    }
    writeCached('codex', 'default', data)
    const result: CodexUsageResult = { usage: data, stale: false, cachedAt: null, authExpired: false }
    usageCache = { at: Date.now(), result }
    return result
  } catch {
    return await fromCache()
  }
}
