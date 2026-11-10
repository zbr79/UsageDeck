import { promises as fs } from 'fs'
import path from 'path'
import { dataDir } from '@/lib/data-dir'

const MAX_LOG_BYTES = 1024 * 1024

function logFile(): string {
  return path.join(dataDir(), 'logs', 'server.log')
}

function formatError(err: unknown): string {
  if (err instanceof Error) return err.message
  if (typeof err === 'string') return err
  try {
    return JSON.stringify(err)
  } catch {
    return String(err)
  }
}

export async function logEvent(
  level: 'info' | 'warn' | 'error',
  scope: string,
  message: string,
  err?: unknown,
): Promise<void> {
  try {
    const file = logFile()
    await fs.mkdir(path.dirname(file), { recursive: true })
    try {
      const stat = await fs.stat(file)
      if (stat.size >= MAX_LOG_BYTES) {
        await fs.rename(file, `${file}.1`).catch(() => {})
      }
    } catch {
      // log file does not exist yet
    }
    const suffix = err !== undefined ? ` (${formatError(err)})` : ''
    const line = `${new Date().toISOString()} [${level.toUpperCase()}] [${scope}] ${message}${suffix}\n`
    await fs.appendFile(file, line, 'utf8')
  } catch {
    // logging is best-effort and must never break provider polling
  }
}

export function logWarn(scope: string, message: string, err?: unknown): void {
  void logEvent('warn', scope, message, err)
}

export function logError(scope: string, message: string, err?: unknown): void {
  void logEvent('error', scope, message, err)
}
