import { promises as fs } from 'fs'
import path from 'path'
import { dataDir } from '@/lib/data-dir'
import { decryptKey, encryptKey, isEncryptedKey } from '@/lib/key-crypto'
import { logWarn } from '@/lib/log'
import { getOrCreateSecret } from '@/lib/secret'

export interface Account {
  email: string
  key: string
}

export interface UsageWindow {
  status: string
  percent: number
  resetsAt: string
}

export interface Usage {
  rolling: UsageWindow
  weekly: UsageWindow
  monthly: UsageWindow
}

const USAGE_URL = 'https://opencode.ai/zen/go/v1/usage'

function accountsFile(): string {
  return path.join(dataDir(), 'accounts.json')
}

function isUsageWindow(value: unknown): value is UsageWindow {
  if (!value || typeof value !== 'object') return false
  const window = value as UsageWindow
  return (
    typeof window.status === 'string' &&
    typeof window.percent === 'number' &&
    typeof window.resetsAt === 'string'
  )
}

export function parseOpenCodeUsage(json: unknown): Usage | null {
  const usage = (json as { usage?: unknown } | null)?.usage
  if (!isUsageWindow((usage as Usage | undefined)?.rolling)) return null
  if (!isUsageWindow((usage as Usage | undefined)?.weekly)) return null
  if (!isUsageWindow((usage as Usage | undefined)?.monthly)) return null
  const parsed = usage as Usage
  return {
    rolling: parsed.rolling,
    weekly: parsed.weekly,
    monthly: parsed.monthly,
  }
}

async function writeAccounts(accounts: Account[], secret: string): Promise<void> {
  const file = accountsFile()
  await fs.mkdir(path.dirname(file), { recursive: true })
  const stored = accounts.map((account) => ({
    email: account.email,
    key: isEncryptedKey(account.key) ? account.key : encryptKey(account.key, secret),
  }))
  await fs.writeFile(file, JSON.stringify(stored, null, 2), { encoding: 'utf8', mode: 0o600 })
}

export async function getAccounts(): Promise<Account[]> {
  let parsed: unknown
  try {
    parsed = JSON.parse(await fs.readFile(accountsFile(), 'utf8'))
  } catch {
    // Expected when accounts.json has not been created yet
    return []
  }
  if (!Array.isArray(parsed)) return []
  const secret = await getOrCreateSecret()
  let needsMigration = false
  const accounts: Account[] = []
  for (const row of parsed) {
    const email = String(row?.email ?? '').trim()
    const storedKey = String(row?.key ?? '')
    if (!email || !storedKey) continue
    if (!isEncryptedKey(storedKey)) needsMigration = true
    try {
      accounts.push({ email, key: decryptKey(storedKey, secret) })
    } catch (err) {
      logWarn('opencode', `Could not decrypt the API key for ${email}`, err)
    }
  }
  if (needsMigration) await writeAccounts(accounts, secret)
  return accounts
}

export async function saveAccounts(accounts: Account[]): Promise<void> {
  await writeAccounts(accounts, await getOrCreateSecret())
}

const usageCache = new Map<string, { at: number; data: Usage | null }>()
const CACHE_MS = 10_000
const FAILURE_CACHE_MS = 15_000

const lastSeen = new Map<string, Usage>()
const lastChange = new Map<string, { at: number; detail: string }>()

function diffText(prev: Usage, cur: Usage): string {
  const parts: string[] = []
  const windows: [string, UsageWindow, UsageWindow][] = [
    ['rolling', prev.rolling, cur.rolling],
    ['weekly', prev.weekly, cur.weekly],
    ['monthly', prev.monthly, cur.monthly],
  ]
  for (const [name, p, c] of windows) {
    if (p.percent !== c.percent) parts.push(`${name} ${p.percent}% → ${c.percent}%`)
  }
  return parts.join(', ')
}

export function getLastChange(email: string): { at: number; detail: string } | null {
  return lastChange.get(email) ?? null
}

export async function getUsage(
  key: string,
  email: string,
): Promise<{ usage: Usage | null; prev: Usage | null }> {
  const hit = usageCache.get(email)
  if (hit && Date.now() - hit.at < (hit.data ? CACHE_MS : FAILURE_CACHE_MS)) {
    return { usage: hit.data, prev: lastSeen.get(email) ?? null }
  }
  try {
    const res = await fetch(USAGE_URL, {
      headers: { Authorization: `Bearer ${key}` },
      cache: 'no-store',
      signal: AbortSignal.timeout(10_000),
    })
    let data: Usage | null = null
    const prev = lastSeen.get(email) ?? null
    if (res.ok) {
      data = parseOpenCodeUsage(await res.json())
      if (data) {
        if (prev) {
          const detail = diffText(prev, data)
          if (detail) lastChange.set(email, { at: Date.now(), detail })
        }
        lastSeen.set(email, data)
      }
    } else {
      logWarn('opencode', `Usage request for ${email} returned HTTP ${res.status}`)
    }
    usageCache.set(email, { at: Date.now(), data })
    return { usage: data, prev }
  } catch (err) {
    logWarn('opencode', `Usage request failed for ${email}`, err)
    usageCache.set(email, { at: Date.now(), data: null })
    return { usage: null, prev: lastSeen.get(email) ?? null }
  }
}