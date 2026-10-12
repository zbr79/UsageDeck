import { promises as fs } from 'fs'
import path from 'path'
import os from 'os'
import crypto from 'crypto'
import { execFile } from 'child_process'
import { readCached, writeCached } from './usage-cache'

// Credentials belong to Claude Code / Claude desktop. This module only ever
// reads them: refreshing would rotate the refresh token out from under the
// owning app and log it out.
const CLAUDE_JSON = path.join(os.homedir(), '.claude.json')
const CLI_CREDENTIALS = path.join(os.homedir(), '.claude', '.credentials.json')
const USAGE_URL = 'https://api.anthropic.com/api/oauth/usage'

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
  authExpired: boolean
}

let cache: { at: number; result: ClaudeUsageResult } | null = null

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
  expired: boolean
}

function isExpired(expiresAt: number | null): boolean {
  return expiresAt != null && expiresAt <= Date.now()
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
      // Preference: live Claude Code token > any live token > expired ones
      // (an expired token still tells us the plan, just not the usage).
      const candidates: { rank: number; token: CachedToken }[] = []
      for (const [name, entry] of Object.entries(entries)) {
        if (!entry?.token) continue
        const expiresAt = typeof entry.expiresAt === 'number' ? entry.expiresAt : null
        const expired = isExpired(expiresAt)
        candidates.push({
          rank: (expired ? 2 : 0) + (name.includes('claude_code') ? 0 : 1),
          token: {
            token: entry.token,
            subscriptionType: entry.subscriptionType ?? null,
            rateLimitTier: entry.rateLimitTier ?? null,
            expiresAt,
            expired,
          },
        })
      }
      candidates.sort((a, b) => a.rank - b.rank)
      if (candidates[0]) return candidates[0].token
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
  return {
    token: oauth.accessToken,
    subscriptionType: oauth.subscriptionType ?? null,
    rateLimitTier: oauth.rateLimitTier ?? null,
    expiresAt,
    expired: isExpired(expiresAt),
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
  if (cache && Date.now() - cache.at < 60_000) return cache.result

  const account = await readAccount()
  const desktop = await readDesktopToken()
  const cli = desktop && !desktop.expired ? null : await readCliToken()
  const token =
    [desktop, cli].find((t) => t && !t.expired) ?? desktop ?? cli ?? null
  const signedIn = token != null
  const subscriptionType = token?.subscriptionType ?? null
  const rateLimitTier = token?.rateLimitTier ?? null
  let authExpired = token?.expired === true

  let fiveHour: ClaudeWindow = { percent: null, resetsAt: null }
  let weekly: ClaudeWindow = { percent: null, resetsAt: null }
  let weeklyOpus: ClaudeWindow | null = null
  let weeklySonnet: ClaudeWindow | null = null
  let resetCredits: number | null = null
  let live = false

  if (token && !token.expired) {
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
      } else if (res.status === 401) {
        authExpired = true
      }
    } catch {
      // fall back to cache below
    }
  }

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

  let result: ClaudeUsageResult
  if (live) {
    writeCached('claude', 'default', data)
    result = { usage: data, stale: false, cachedAt: null, authExpired: false }
  } else {
    // Prefer the last known numbers; otherwise show what we know locally
    // (account + plan) without persisting a usage-less snapshot.
    const cached = await readCached<ClaudeUsage>('claude', 'default')
    if (cached?.data) {
      result = { usage: cached.data, stale: true, cachedAt: cached.savedAt, authExpired }
    } else if (signedIn || account.email) {
      result = { usage: data, stale: false, cachedAt: null, authExpired }
    } else {
      result = { usage: null, stale: false, cachedAt: null, authExpired: false }
    }
  }
  cache = { at: Date.now(), result }
  return result
}
