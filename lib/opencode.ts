import { dataFile } from './paths'
import { readJson, withLock, writeJsonAtomic } from './fs-atomic'
import { createProvider, type ProviderResult } from './provider-cache'
import type { OpenCodeUsage, UsageWindow } from './types'

export type { OpenCodeUsage as Usage, UsageWindow } from './types'

export interface Account {
  email: string
  key: string
}

const ACCOUNTS_FILE = 'accounts.json'
const USAGE_URL = 'https://opencode.ai/zen/go/v1/usage'

function isAccount(value: unknown): value is Account {
  const a = value as Account
  return !!a && typeof a.email === 'string' && typeof a.key === 'string'
}

export async function getAccounts(): Promise<Account[]> {
  const parsed = await readJson(dataFile(ACCOUNTS_FILE))
  return Array.isArray(parsed) ? parsed.filter(isAccount) : []
}

/** Read-modify-write the account list under a lock, written atomically. */
export function updateAccounts(change: (accounts: Account[]) => Account[]): Promise<Account[]> {
  return withLock(ACCOUNTS_FILE, async () => {
    const next = change(await getAccounts())
    await writeJsonAtomic(dataFile(ACCOUNTS_FILE), next, 0o600)
    return next
  })
}

/** "sk-…a1b2" — enough to recognise a key, useless to anyone who sees it. */
export function keyHint(key: string): string {
  return key.length > 8 ? `${key.slice(0, 3)}…${key.slice(-4)}` : '•••'
}

function parseWindow(value: unknown): UsageWindow | null {
  const w = value as Partial<UsageWindow> | null
  if (!w || typeof w.percent !== 'number' || !Number.isFinite(w.percent)) return null
  return {
    status: typeof w.status === 'string' ? w.status : 'ok',
    percent: w.percent,
    resetsAt: typeof w.resetsAt === 'string' ? w.resetsAt : '',
  }
}

/** The `usage` object from /zen/go/v1/usage, or null if malformed. */
export function parseOpenCodeUsage(json: unknown): OpenCodeUsage | null {
  const u = (json as { usage?: Record<string, unknown> } | null)?.usage
  if (!u) return null
  const rolling = parseWindow(u.rolling)
  const weekly = parseWindow(u.weekly)
  const monthly = parseWindow(u.monthly)
  return rolling && weekly && monthly ? { rolling, weekly, monthly } : null
}

export function diffText(prev: OpenCodeUsage, cur: OpenCodeUsage): string {
  const parts: string[] = []
  for (const name of ['rolling', 'weekly', 'monthly'] as const) {
    if (prev[name].percent !== cur[name].percent) {
      parts.push(`${name} ${prev[name].percent}% → ${cur[name].percent}%`)
    }
  }
  return parts.join(', ')
}

// Per-account state, keyed by email.
const keys = new Map<string, string>()
const lastSeen = new Map<string, OpenCodeUsage>()
const prevSnapshot = new Map<string, OpenCodeUsage | null>()
const lastChange = new Map<string, { at: number; detail: string }>()
const subscription = new Map<string, boolean | null>()

const provider = createProvider<OpenCodeUsage>({
  section: 'opencode',
  ttlMs: 10_000,
  failureTtlMs: 15_000,
  async fetch(email) {
    const key = keys.get(email)
    if (!key) return { ok: false }
    const res = await fetch(USAGE_URL, {
      headers: { Authorization: `Bearer ${key}` },
      cache: 'no-store',
      signal: AbortSignal.timeout(10_000),
    })
    if (!res.ok) {
      const err = await res.json().catch(() => null)
      subscription.set(email, err?.error?.type === 'EntitlementError' ? false : null)
      return { ok: false }
    }
    const data = parseOpenCodeUsage(await res.json())
    if (!data) {
      subscription.set(email, null)
      return { ok: false }
    }
    subscription.set(email, true)
    const prev = lastSeen.get(email) ?? null
    if (prev) {
      const detail = diffText(prev, data)
      if (detail) lastChange.set(email, { at: Date.now(), detail })
    }
    // `prev` stays stable across cache hits until the next live fetch, so
    // the widget sees the same before/after pair for the whole TTL.
    prevSnapshot.set(email, prev)
    lastSeen.set(email, data)
    return { ok: true, data }
  },
})

export interface UsageResult extends ProviderResult<OpenCodeUsage> {
  prev: OpenCodeUsage | null
  subscriptionActive: boolean | null
}

export async function getUsage(key: string, email: string): Promise<UsageResult> {
  if (keys.get(email) !== key) {
    keys.set(email, key)
    provider.invalidate(email)
  }
  const result = await provider.get(email)
  return {
    ...result,
    prev: prevSnapshot.get(email) ?? null,
    subscriptionActive: subscription.get(email) ?? null,
  }
}

export function getLastChange(email: string): { at: number; detail: string } | null {
  return lastChange.get(email) ?? null
}
