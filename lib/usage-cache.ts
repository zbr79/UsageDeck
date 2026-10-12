import { promises as fs } from 'fs'
import path from 'path'

interface CacheEntry<T> {
  savedAt: number
  data: T
}

type CacheState = Record<string, Record<string, CacheEntry<unknown>>>

const FILE = path.join(process.cwd(), 'data', 'usage-cache.json')

let state: CacheState | null = null
let loading: Promise<CacheState> | null = null
let writeTimer: ReturnType<typeof setTimeout> | null = null

async function load(): Promise<CacheState> {
  if (state) return state
  if (!loading) {
    loading = (async () => {
      try {
        const parsed = JSON.parse(await fs.readFile(FILE, 'utf8'))
        return parsed && typeof parsed === 'object' ? (parsed as CacheState) : {}
      } catch {
        return {}
      }
    })()
  }
  state = await loading
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

async function flush(): Promise<void> {
  const snapshot = state
  if (!snapshot) return
  try {
    await fs.mkdir(path.dirname(FILE), { recursive: true })
    await fs.writeFile(FILE, JSON.stringify(snapshot, null, 2), 'utf8')
  } catch {
    // ignore cache write failures
  }
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

export function writeCached(section: string, key: string, data: unknown): void {
  void load().then((s) => {
    if (!s[section]) s[section] = {}
    s[section][key] = { savedAt: Date.now(), data }
    scheduleWrite()
  })
}
