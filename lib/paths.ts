import path from 'path'

/**
 * Where the app keeps its own state (accounts, settings, cache, API secret).
 * Overridable so tests and packaged builds can point it elsewhere.
 */
export function dataDir(): string {
  return (
    process.env.USAGE_DATA_DIR || path.join(/*turbopackIgnore: true*/ process.cwd(), 'data')
  )
}

export function dataFile(name: string): string {
  return path.join(/*turbopackIgnore: true*/ dataDir(), name)
}
