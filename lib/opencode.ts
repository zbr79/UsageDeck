import { promises as fs } from 'fs'
import path from 'path'
import { readCached, writeCached } from './usage-cache'

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

const DATA_DIR = path.join(process.cwd(), 'data')
const ACCOUNTS_FILE = path.join(DATA_DIR, 'accounts.json')
const USAGE_URL = 'https://opencode.ai/zen/go/v1/usage'

export async function getAccounts(): Promise<Account[]> {
  try {
    const raw = await fs.readFile(ACCOUNTS_FILE, 'utf8')
    const parsed = JSON.parse(raw)
    return Array.isArray(parsed) ? parsed : []
  } catch {
    return []
  }
}

export async function saveAccounts(accounts: Account[]): Promise<void> {
  await fs.mkdir(DATA_DIR, { recursive: true })
  await fs.writeFile(ACCOUNTS_FILE, JSON.stringify(accounts, null, 2), 'utf8')
}

interface UsageCacheEntry {
  at: number
  data: Usage | null
  stale: boolean
  cachedAt: number | null
  subscriptionActive: boolean | null
}

export interface UsageResult {
  usage: Usage | null
  prev: Usage | null
  stale: boolean
  cachedAt: number | null
  subscriptionActive: boolean | null
}

const usageCache = new Map<string, UsageCacheEntry>()
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

export async function getUsage(key: string, email: string): Promise<UsageResult> {
  async function fromCache(
    subscriptionActive: boolean | null = null,
  ): Promise<UsageResult> {
    const cached = await readCached<Usage>('opencode', email)
    const entry: UsageCacheEntry = {
      at: Date.now(),
      data: cached?.data ?? null,
      stale: cached !== null,
      cachedAt: cached?.savedAt ?? null,
      subscriptionActive,
    }
    usageCache.set(email, entry)
    return {
      usage: entry.data,
      prev: lastSeen.get(email) ?? null,
      stale: entry.stale,
      cachedAt: entry.cachedAt,
      subscriptionActive: entry.subscriptionActive,
    }
  }

  const hit = usageCache.get(email)
  if (hit && Date.now() - hit.at < (hit.data ? CACHE_MS : FAILURE_CACHE_MS)) {
    return {
      usage: hit.data,
      prev: lastSeen.get(email) ?? null,
      stale: hit.stale,
      cachedAt: hit.cachedAt,
      subscriptionActive: hit.subscriptionActive,
    }
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
      const json = await res.json()
      data = (json?.usage as Usage) ?? null
      if (data) {
        if (prev) {
          const detail = diffText(prev, data)
          if (detail) lastChange.set(email, { at: Date.now(), detail })
        }
        lastSeen.set(email, data)
        writeCached('opencode', email, data)
        usageCache.set(email, {
          at: Date.now(),
          data,
          stale: false,
          cachedAt: null,
          subscriptionActive: true,
        })
        return { usage: data, prev, stale: false, cachedAt: null, subscriptionActive: true }
      }
    }
    let subscriptionActive: boolean | null = null
    if (!res.ok) {
      try {
        const err = await res.json()
        if (err?.error?.type === 'EntitlementError') subscriptionActive = false
      } catch {
        // non-JSON error body
      }
    }
    return await fromCache(subscriptionActive)
  } catch {
    return await fromCache(null)
  }
}