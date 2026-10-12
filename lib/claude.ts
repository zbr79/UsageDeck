import { promises as fs } from 'fs'
import path from 'path'
import os from 'os'
import crypto from 'crypto'
import { execFile } from 'child_process'
import { readCached, writeCached } from './usage-cache'

const CLAUDE_JSON = path.join(os.homedir(), '.claude.json')
const CLI_CREDENTIALS = path.join(os.homedir(), '.claude', '.credentials.json')
const USAGE_URL = 'https://api.anthropic.com/api/oauth/usage'
const TOKEN_URL = 'https://console.anthropic.com/v1/oauth/token'
const OAUTH_CLIENT_ID = '9d1c250a-e61b-44d9-88ed-5944d1962f5e'

export interface ClaudeWindow {
  percent: number | null
  resetsAt: string | null
}

export interface ClaudeUsage {
  email: string | null
  displayName: string | null
  signedIn: boolean
  subscribed: boolean
  subscriptionType: string | null
  planName: string | null
  fiveHour: ClaudeWindow
  weekly: ClaudeWindow
  weeklyOpus: ClaudeWindow | null
  weeklySonnet: ClaudeWindow | null
  resetCredits: number | null
  billingType: string | null
  rateLimitTier: string | null
  orgUuid: string | null
}

export interface ClaudeUsageResult {
  usage: ClaudeUsage | null
  stale: boolean
  cachedAt: number | null
}

let cache: { at: number; data: ClaudeUsage | null; stale: boolean; cachedAt: number | null } | null = null

function dpapiUnprotect(encryptedKeyB64: string): Promise<Buffer> {
  const script = [
    'Add-Type -AssemblyName System.Security',
    '$enc = [Convert]::FromBase64String($env:OS_KEY)',
    '$blob = $enc[5..($enc.Length - 1)]',
    '$key = [System.Security.Cryptography.ProtectedData]::Unprotect($blob, $null, [System.Security.Cryptography.DataProtectionScope]::CurrentUser)',
    '[Convert]::ToBase64String($key)',
  ].join('\n')
  return new Promise((resolve, reject) => {
    execFile(
      'powershell.exe',
      ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-Command', script],
      { env: { ...process.env, OS_KEY: encryptedKeyB64 }, windowsHide: true, timeout: 10_000, maxBuffer: 1024 * 1024 },
      (err, stdout) => {
        if (err) return reject(err)
        const lines = stdout.trim().split(/\r?\n/).filter(Boolean)
        const b64 = lines[lines.length - 1]?.trim()
        if (!b64) return reject(new Error('empty dpapi output'))
        resolve(Buffer.from(b64, 'base64'))
      },
    )
  })
}

function unseal(key: Buffer, sealedB64: string): string | null {
  try {
    const b = Buffer.from(sealedB64, 'base64')
    if (b.length < 3 + 12 + 16) return null
    if (b.subarray(0, 3).toString('latin1') !== 'v10') return null
    const nonce = b.subarray(3, 15)
    const tag = b.subarray(b.length - 16)
    const ct = b.subarray(15, b.length - 16)
    const d = crypto.createDecipheriv('aes-256-gcm', key, nonce)
    d.setAuthTag(tag)
    return Buffer.concat([d.update(ct), d.final()]).toString('utf8')
  } catch {
    return null
  }
}

interface CachedToken {
  token: string
  subscriptionType: string | null
  rateLimitTier: string | null
  expiresAt: number | null
}

async function desktopConfigDirs(): Promise<string[]> {
  const dirs: string[] = []
  if (process.env.LOCALAPPDATA) {
    const packages = path.join(process.env.LOCALAPPDATA, 'Packages')
    try {
      for (const entry of await fs.readdir(packages, { withFileTypes: true })) {
        if (entry.isDirectory() && /^Claude_/i.test(entry.name)) {
          dirs.push(path.join(packages, entry.name, 'LocalCache', 'Roaming', 'Claude'))
        }
      }
    } catch {
      // no packages dir
    }
  }
  if (process.env.APPDATA) dirs.push(path.join(process.env.APPDATA, 'Claude'))
  return dirs
}

async function readDesktopToken(): Promise<CachedToken | null> {
  for (const dir of await desktopConfigDirs()) {
    try {
      const config = JSON.parse(await fs.readFile(path.join(dir, 'config.json'), 'utf8'))
      const sealed = config['oauth:tokenCacheV2'] ?? config['oauth:tokenCache']
      if (typeof sealed !== 'string') continue
      const localState = JSON.parse(await fs.readFile(path.join(dir, 'Local State'), 'utf8'))
      const encKey = localState?.os_crypt?.encrypted_key
      if (typeof encKey !== 'string') continue
      const key = await dpapiUnprotect(encKey)
      if (key.length !== 32) continue
      const raw = unseal(key, sealed)
      if (!raw) continue
      const entries = JSON.parse(raw) as Record<string, {
        token?: string
        expiresAt?: number
        subscriptionType?: string
        rateLimitTier?: string
      }>
      let claudeCode: CachedToken | null = null
      let fallback: CachedToken | null = null
      for (const [name, entry] of Object.entries(entries)) {
        if (!entry?.token) continue
        if (typeof entry.expiresAt === 'number' && entry.expiresAt < Date.now()) continue
        const candidate: CachedToken = {
          token: entry.token,
          subscriptionType: entry.subscriptionType ?? null,
          rateLimitTier: entry.rateLimitTier ?? null,
          expiresAt: typeof entry.expiresAt === 'number' ? entry.expiresAt : null,
        }
        if (name.includes('claude_code')) claudeCode = candidate
        else if (!fallback) fallback = candidate
      }
      if (claudeCode) return claudeCode
      if (fallback) return fallback
    } catch {
      // try next dir
    }
  }
  return null
}

async function readCliToken(): Promise<CachedToken | null> {
  let creds: any
  try {
    creds = JSON.parse(await fs.readFile(CLI_CREDENTIALS, 'utf8'))
  } catch {
    return null
  }
  const oauth = creds?.claudeAiOauth
  if (!oauth?.accessToken) return null
  const expiresAt = typeof oauth.expiresAt === 'number' ? oauth.expiresAt : null
  const notExpired = expiresAt == null || expiresAt > Date.now()
  if (notExpired) {
    return {
      token: oauth.accessToken,
      subscriptionType: oauth.subscriptionType ?? null,
      rateLimitTier: oauth.rateLimitTier ?? null,
      expiresAt,
    }
  }
  const refreshed = await refreshCliToken(oauth)
  return refreshed
}

async function refreshCliToken(oauth: any): Promise<CachedToken | null> {
  if (!oauth?.refreshToken) return null
  try {
    const res = await fetch(TOKEN_URL, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        grant_type: 'refresh_token',
        refresh_token: oauth.refreshToken,
        client_id: OAUTH_CLIENT_ID,
      }),
      signal: AbortSignal.timeout(10_000),
    })
    if (!res.ok) return null
    const json = await res.json()
    if (!json?.access_token) return null
    const next = {
      ...oauth,
      accessToken: json.access_token,
      refreshToken: json.refresh_token ?? oauth.refreshToken,
      expiresAt: Date.now() + (typeof json.expires_in === 'number' ? json.expires_in * 1000 : 3600_000),
    }
    try {
      await fs.writeFile(CLI_CREDENTIALS, JSON.stringify(next, null, 2), 'utf8')
    } catch {
      // best-effort persistence
    }
    return {
      token: next.accessToken,
      subscriptionType: next.subscriptionType ?? null,
      rateLimitTier: next.rateLimitTier ?? null,
      expiresAt: next.expiresAt,
    }
  } catch {
    return null
  }
}

async function readAccount(): Promise<{
  email: string | null
  displayName: string | null
  billingType: string | null
  organizationType: string | null
  seatTier: string | null
  orgUuid: string | null
}> {
  try {
    const main = JSON.parse(await fs.readFile(CLAUDE_JSON, 'utf8'))
    const oauth = main?.oauthAccount ?? {}
    return {
      email: typeof oauth.emailAddress === 'string' ? oauth.emailAddress : null,
      displayName: typeof oauth.displayName === 'string' ? oauth.displayName : null,
      billingType: typeof oauth.billingType === 'string' ? oauth.billingType : null,
      organizationType: typeof oauth.organizationType === 'string' ? oauth.organizationType : null,
      seatTier: typeof oauth.seatTier === 'string' ? oauth.seatTier : null,
      orgUuid: typeof oauth.organizationUuid === 'string' ? oauth.organizationUuid : null,
    }
  } catch {
    return { email: null, displayName: null, billingType: null, organizationType: null, seatTier: null, orgUuid: null }
  }
}

function planNameOf(subscriptionType: string | null, organizationType: string | null): string | null {
  const raw = (subscriptionType ?? organizationType ?? '').toLowerCase()
  if (!raw) return null
  if (raw.includes('max')) return 'Max'
  if (raw.includes('pro')) return 'Pro'
  if (raw.includes('free')) return 'Free'
  if (raw.includes('team')) return 'Team'
  if (raw.includes('enterprise')) return 'Enterprise'
  return subscriptionType ?? organizationType
}

function windowOf(value: any): ClaudeWindow {
  if (!value || typeof value !== 'object') return { percent: null, resetsAt: null }
  const percent =
    typeof value.utilization === 'number'
      ? value.utilization
      : typeof value.percent === 'number'
        ? value.percent
        : null
  const rawReset = value.resets_at ?? value.resetsAt
  let resetsAt: string | null = null
  if (typeof rawReset === 'number') resetsAt = new Date(rawReset * 1000).toISOString()
  else if (typeof rawReset === 'string') {
    const d = new Date(rawReset)
    if (!Number.isNaN(d.getTime())) resetsAt = d.toISOString()
  }
  return { percent, resetsAt }
}

function resetCreditsOf(json: any): number | null {
  if (!json || typeof json !== 'object') return null
  for (const key of ['cedar_ember', 'juniper_tide', 'iguana_necktie', 'amber_gauge']) {
    const v = json[key]
    if (!v || typeof v !== 'object') continue
    if (v.eligible === false) continue
    if (Array.isArray(v.grants)) return v.grants.length
    if (typeof v.available === 'boolean') return v.available ? 1 : 0
  }
  return null
}

export async function getClaudeUsage(): Promise<ClaudeUsageResult> {
  async function fromCache(): Promise<ClaudeUsageResult> {
    const cached = await readCached<ClaudeUsage>('claude', 'default')
    cache = {
      at: Date.now(),
      data: cached?.data ?? null,
      stale: cached !== null,
      cachedAt: cached?.savedAt ?? null,
    }
    return { usage: cache.data, stale: cache.stale, cachedAt: cache.cachedAt }
  }

  if (cache && Date.now() - cache.at < 60_000) {
    return { usage: cache.data, stale: cache.stale, cachedAt: cache.cachedAt }
  }

  const account = await readAccount()
  const token = (await readDesktopToken()) ?? (await readCliToken())
  const signedIn = token != null
  const subscriptionType = token?.subscriptionType ?? null
  const rateLimitTier = token?.rateLimitTier ?? null

  let fiveHour: ClaudeWindow = { percent: null, resetsAt: null }
  let weekly: ClaudeWindow = { percent: null, resetsAt: null }
  let weeklyOpus: ClaudeWindow | null = null
  let weeklySonnet: ClaudeWindow | null = null
  let resetCredits: number | null = null
  let live = false

  if (token) {
    try {
      const res = await fetch(USAGE_URL, {
        headers: {
          Authorization: `Bearer ${token.token}`,
          'anthropic-beta': 'oauth-2025-04-20',
          'anthropic-version': '2023-06-01',
          'User-Agent': 'claude-cli/2.1.293',
          Accept: 'application/json',
        },
        cache: 'no-store',
        signal: AbortSignal.timeout(10_000),
      })
      if (res.ok) {
        const json = await res.json()
        fiveHour = windowOf(json?.five_hour)
        weekly = windowOf(json?.seven_day)
        if (json?.seven_day_opus) weeklyOpus = windowOf(json.seven_day_opus)
        if (json?.seven_day_sonnet) weeklySonnet = windowOf(json.seven_day_sonnet)
        resetCredits = resetCreditsOf(json)
        live = true
      }
    } catch {
      // fall back to cache below
    }
  }

  if (!live && !signedIn && !account.email) return await fromCache()

  const subscribed =
    subscriptionType != null ||
    (account.billingType != null && account.billingType !== 'none') ||
    account.seatTier != null
  const data: ClaudeUsage = {
    email: account.email,
    displayName: account.displayName,
    signedIn,
    subscribed,
    subscriptionType,
    planName: planNameOf(subscriptionType, account.organizationType),
    fiveHour,
    weekly,
    weeklyOpus,
    weeklySonnet,
    resetCredits,
    billingType: account.billingType,
    rateLimitTier,
    orgUuid: account.orgUuid,
  }

  if (!live) {
    const cached = await readCached<ClaudeUsage>('claude', 'default')
    if (cached?.data) {
      cache = { at: Date.now(), data: cached.data, stale: true, cachedAt: cached.savedAt }
      return { usage: cached.data, stale: true, cachedAt: cached.savedAt }
    }
  }

  writeCached('claude', 'default', data)
  cache = { at: Date.now(), data, stale: false, cachedAt: null }
  return { usage: data, stale: false, cachedAt: null }
}
