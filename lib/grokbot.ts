import { promises as fs } from 'fs'
import path from 'path'
import os from 'os'
import crypto from 'crypto'
import { execFile } from 'child_process'
import { readCached, writeCached } from './usage-cache'

const GROK_DIR = path.join(
  process.env.APPDATA ?? path.join(os.homedir(), 'AppData', 'Roaming'),
  'Grok Bot',
)
const SECRETS_FILE = path.join(GROK_DIR, 'sand-secrets.json')
const LOCAL_STATE_FILE = path.join(GROK_DIR, 'Local State')
const SAND_URL =
  'https://api2.cursor.sh/aiserver.v1.DashboardService/GetSandUsageStatus'

export interface GrokUsage {
  email: string | null
  name: string | null
  percentUsed: number | null
  resetAt: string | null
  planLabel: string | null
  cursorPlanName: string | null
}

export interface GrokUsageResult {
  usage: GrokUsage | null
  stale: boolean
  cachedAt: number | null
}

let keyCache: Buffer | null = null
let usageCache: {
  at: number
  data: GrokUsage | null
  stale: boolean
  cachedAt: number | null
} | null = null

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
      {
        env: { ...process.env, OS_KEY: encryptedKeyB64 },
        windowsHide: true,
        timeout: 10_000,
        maxBuffer: 1024 * 1024,
      },
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

async function getOsCryptKey(): Promise<Buffer | null> {
  if (keyCache) return keyCache
  try {
    const ls = JSON.parse(await fs.readFile(LOCAL_STATE_FILE, 'utf8'))
    const encryptedKey = ls?.os_crypt?.encrypted_key
    if (typeof encryptedKey !== 'string' || !encryptedKey) return null
    const key = await dpapiUnprotect(encryptedKey)
    if (key.length !== 32) return null
    keyCache = key
    return key
  } catch {
    return null
  }
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

interface GrokAccount {
  token: string | null
  email: string | null
  name: string | null
}

async function readActiveAccount(): Promise<GrokAccount | null> {
  let raw: string
  try {
    raw = await fs.readFile(SECRETS_FILE, 'utf8')
  } catch {
    return null
  }
  let accounts: { active?: string; accounts?: Record<string, Record<string, string>> }
  try {
    const secrets = JSON.parse(raw)
    accounts = JSON.parse(secrets['cursor-accounts'])
  } catch {
    return null
  }
  const active = accounts?.active
  const entry = active ? accounts?.accounts?.[active] : undefined
  if (!entry) return null
  let key = await getOsCryptKey()
  if (!key) return null
  let token = unseal(key, entry['cursor-access-token'])
  if (!token) {
    keyCache = null
    key = await getOsCryptKey()
    token = key ? unseal(key, entry['cursor-access-token']) : null
  }
  if (!token) return null
  let email: string | null = null
  let name: string | null = null
  const profileRaw = key ? unseal(key, entry['cursor-account-profile']) : null
  if (profileRaw) {
    try {
      const p = JSON.parse(profileRaw)
      email = typeof p?.email === 'string' ? p.email : null
      name = typeof p?.name === 'string' ? p.name : null
    } catch {
      // ignore malformed profile
    }
  }
  return { token, email, name }
}

export async function getGrokUsage(): Promise<GrokUsageResult> {
  async function fromCache(): Promise<GrokUsageResult> {
    const cached = await readCached<GrokUsage>('grokbot', 'default')
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

  const account = await readActiveAccount()
  if (!account?.token) return await fromCache()

  try {
    const res = await fetch(SAND_URL, {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${account.token}`,
        'Content-Type': 'application/json',
        Accept: 'application/json',
        'User-Agent': 'Mozilla/5.0',
      },
      body: '{}',
      cache: 'no-store',
      signal: AbortSignal.timeout(10_000),
    })
    if (!res.ok) return await fromCache()
    const sj = await res.json()
    const data: GrokUsage = {
      email: account.email,
      name: account.name,
      percentUsed: typeof sj?.usagePercent === 'number' ? sj.usagePercent : null,
      resetAt:
        typeof sj?.nextResetTimestampUtc === 'string'
          ? new Date(sj.nextResetTimestampUtc).toISOString()
          : null,
      planLabel: typeof sj?.grokPlanLabel === 'string' ? sj.grokPlanLabel : null,
      cursorPlanName: typeof sj?.cursorPlanName === 'string' ? sj.cursorPlanName : null,
    }
    writeCached('grokbot', 'default', data)
    usageCache = { at: Date.now(), data, stale: false, cachedAt: null }
    return { usage: data, stale: false, cachedAt: null }
  } catch {
    return await fromCache()
  }
}
