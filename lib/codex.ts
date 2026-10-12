import { promises as fs } from 'fs'
import path from 'path'
import os from 'os'
import { readCached, writeCached } from './usage-cache'

const AUTH_FILE = path.join(os.homedir(), '.codex', 'auth.json')
const USAGE_URL = 'https://chatgpt.com/backend-api/wham/usage'
const TOKEN_URL = 'https://auth.openai.com/oauth/token'
const CLIENT_ID = 'app_EMoamEEZ73f0CkXaXp7hrann'

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
}

let usageCache: {
  at: number
  data: CodexUsage | null
  stale: boolean
  cachedAt: number | null
} | null = null

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

async function refreshTokens(auth: CodexAuth): Promise<CodexAuth | null> {
  const rt = auth.tokens?.refresh_token
  if (!rt) return null
  try {
    const res = await fetch(TOKEN_URL, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        client_id: CLIENT_ID,
        grant_type: 'refresh_token',
        refresh_token: rt,
        scope: 'openid profile email',
      }),
      signal: AbortSignal.timeout(10_000),
    })
    if (!res.ok) return null
    const json = await res.json()
    if (!json?.access_token) return null
    const next: CodexAuth = {
      ...auth,
      tokens: {
        ...auth.tokens,
        access_token: json.access_token,
        refresh_token: json.refresh_token ?? rt,
        id_token: json.id_token ?? auth.tokens?.id_token,
      },
      last_refresh: new Date().toISOString(),
    }
    await fs.writeFile(AUTH_FILE, JSON.stringify(next, null, 2), 'utf8')
    return next
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
  async function fromCache(): Promise<CodexUsageResult> {
    const cached = await readCached<CodexUsage>('codex', 'default')
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
  let auth = await readAuth()
  const accessToken = auth?.tokens?.access_token
  if (!auth || !accessToken) return await fromCache()
  try {
    let res = await fetchUsage(accessToken, auth.tokens?.account_id)
    if (res.status === 401) {
      const refreshed = await refreshTokens(auth)
      if (!refreshed?.tokens?.access_token) return await fromCache()
      auth = refreshed
      res = await fetchUsage(refreshed.tokens.access_token, refreshed.tokens.account_id)
    }
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
    usageCache = { at: Date.now(), data, stale: false, cachedAt: null }
    return { usage: data, stale: false, cachedAt: null }
  } catch {
    return await fromCache()
  }
}