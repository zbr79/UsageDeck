import { promises as fs } from 'fs'
import path from 'path'
import crypto from 'crypto'

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))

/**
 * Write via temp file + rename so a crash mid-write can never leave a
 * truncated file behind. Rename is retried briefly because Windows can
 * hold the target open (antivirus, indexer) for a moment.
 */
export async function writeFileAtomic(file: string, data: string, mode?: number): Promise<void> {
  await fs.mkdir(path.dirname(file), { recursive: true })
  const tmp = `${file}.${process.pid}.${crypto.randomBytes(4).toString('hex')}.tmp`
  await fs.writeFile(tmp, data, { encoding: 'utf8', mode })
  for (let attempt = 0; ; attempt++) {
    try {
      await fs.rename(tmp, file)
      return
    } catch (err) {
      const code = (err as NodeJS.ErrnoException).code
      if (attempt >= 5 || (code !== 'EPERM' && code !== 'EBUSY' && code !== 'EACCES')) {
        await fs.rm(tmp, { force: true })
        throw err
      }
      await sleep(25 * (attempt + 1))
    }
  }
}

export async function writeJsonAtomic(file: string, value: unknown, mode?: number): Promise<void> {
  await writeFileAtomic(file, JSON.stringify(value, null, 2), mode)
}

export async function readJson<T = unknown>(file: string): Promise<T | null> {
  try {
    return JSON.parse(await fs.readFile(file, 'utf8')) as T
  } catch {
    return null
  }
}

const queues = new Map<string, Promise<unknown>>()

/**
 * Serialise read-modify-write cycles on one resource so concurrent requests
 * can't overwrite each other's changes (lost updates).
 */
export function withLock<T>(key: string, fn: () => Promise<T>): Promise<T> {
  const prev = queues.get(key) ?? Promise.resolve()
  const run = prev.then(fn, fn)
  const settled = run.then(
    () => undefined,
    () => undefined,
  )
  queues.set(key, settled)
  void settled.then(() => {
    if (queues.get(key) === settled) queues.delete(key)
  })
  return run
}
