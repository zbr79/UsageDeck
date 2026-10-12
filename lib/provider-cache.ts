import { readCached, writeCached } from './usage-cache'

export interface ProviderResult<T> {
  usage: T | null
  /** true when `usage` is last-known data from disk, not a live fetch */
  stale: boolean
  cachedAt: number | null
  /** the provider rejected or we found only an expired sign-in */
  authExpired: boolean
}

export type FetchOutcome<T> =
  | { ok: true; data: T }
  | {
      ok: false
      authExpired?: boolean
      /** shown when there is no cached data either (never persisted) */
      fallback?: T | null
    }

export interface ProviderOptions<T> {
  /** section name in usage-cache.json */
  section: string
  /** how long a successful result is reused */
  ttlMs: number
  /** how long a failed result is reused before retrying (default ttlMs) */
  failureTtlMs?: number
  fetch: (key: string) => Promise<FetchOutcome<T>>
}

export interface Provider<T> {
  get(key?: string): Promise<ProviderResult<T>>
  invalidate(key?: string): void
}

/**
 * The one caching policy every provider shares:
 *  - successful results are reused for `ttlMs`, failures for `failureTtlMs`;
 *  - concurrent callers share a single in-flight fetch (the widget and the
 *    settings window polling together never double-hit a provider);
 *  - on failure, fall back to the last-known data on disk, marked stale.
 */
export function createProvider<T>(opts: ProviderOptions<T>): Provider<T> {
  const memo = new Map<string, { at: number; ok: boolean; result: ProviderResult<T> }>()
  const inflight = new Map<string, Promise<ProviderResult<T>>>()

  async function resolve(key: string): Promise<ProviderResult<T>> {
    let outcome: FetchOutcome<T>
    try {
      outcome = await opts.fetch(key)
    } catch {
      outcome = { ok: false }
    }
    let result: ProviderResult<T>
    if (outcome.ok) {
      await writeCached(opts.section, key, outcome.data)
      result = { usage: outcome.data, stale: false, cachedAt: null, authExpired: false }
    } else {
      const authExpired = outcome.authExpired === true
      const cached = await readCached<T>(opts.section, key)
      result = cached
        ? { usage: cached.data, stale: true, cachedAt: cached.savedAt, authExpired }
        : { usage: outcome.fallback ?? null, stale: false, cachedAt: null, authExpired }
    }
    memo.set(key, { at: Date.now(), ok: outcome.ok, result })
    return result
  }

  return {
    get(key = 'default') {
      const hit = memo.get(key)
      if (hit) {
        const ttl = hit.ok ? opts.ttlMs : (opts.failureTtlMs ?? opts.ttlMs)
        if (Date.now() - hit.at < ttl) return Promise.resolve(hit.result)
      }
      let pending = inflight.get(key)
      if (!pending) {
        pending = resolve(key).finally(() => inflight.delete(key))
        inflight.set(key, pending)
      }
      return pending
    },
    invalidate(key) {
      if (key === undefined) memo.clear()
      else memo.delete(key)
    },
  }
}
