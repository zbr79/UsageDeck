import { dataFile } from './paths'
import { readJson, withLock, writeJsonAtomic } from './fs-atomic'

/**
 * Last-known usage per provider/account, persisted so the widget can show
 * something (marked "cached") across restarts and outages.
 */

interface CacheEntry<T> {
  savedAt: number
  data: T
}

type CacheState = Record<string, Record<string, CacheEntry<unknown>>>

const FILE_NAME = 'usage-cache.json'

let state: CacheState | null = null
let loading: Promise<CacheState> | null = null
let writeTimer: ReturnType<typeof setTimeout> | null = null

function isState(value: unknown): value is CacheState {
  return !!value && typeof value === 'object' && !Array.isArray(value)
}

async function readDisk(): Promise<CacheState> {
  const parsed = await readJson(dataFile(FILE_NAME))
  return isState(parsed) ? parsed : {}
}

async function load(): Promise<CacheState> {
  if (state) return state
  loading ??= readDisk()
  const loaded = await loading
  state ??= loaded
  loading = null
  return state
}

function scheduleWrite(): void {
  if (writeTimer) clearTimeout(writeTimer)
  writeTimer = setTimeout(() => {
    writeTimer = null
    void flush()
  }, 500)
}

/**
 * Merge with what's on disk (newest entry wins) before writing, so another
 * module instance or process writing other sections is never clobbered.
 */
export function flush(): Promise<void> {
  return withLock(FILE_NAME, async () => {
    const mine = state
    if (!mine) return
    const merged = await readDisk()
    for (const [section, entries] of Object.entries(mine)) {
      merged[section] ??= {}
      for (const [key, entry] of Object.entries(entries)) {
        const theirs = merged[section][key]
        if (!theirs || (theirs.savedAt ?? 0) <= entry.savedAt) merged[section][key] = entry
      }
    }
    try {
      await writeJsonAtomic(dataFile(FILE_NAME), merged)
    } catch {
      // cache is best-effort
    }
  })
}

export async function readCached<T>(
  section: string,
  key: string,
): Promise<{ data: T; savedAt: number } | null> {
  const s = await load()
  const entry = s[section]?.[key] as CacheEntry<T> | undefined
  if (!entry || entry.data === undefined || entry.data === null) return null
  return { data: entry.data, savedAt: entry.savedAt }
}

export async function writeCached(section: string, key: string, data: unknown): Promise<void> {
  const s = await load()
  s[section] ??= {}
  s[section][key] = { savedAt: Date.now(), data }
  scheduleWrite()
}

/** Test hook: forget in-memory state so the next read comes from disk. */
export function resetUsageCacheForTests(): void {
  state = null
  loading = null
  if (writeTimer) clearTimeout(writeTimer)
  writeTimer = null
}
