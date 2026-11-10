import { promises as fs } from 'fs'
import os from 'os'
import path from 'path'
import { afterEach, describe, expect, it } from 'vitest'
import { logEvent } from '@/lib/log'
import { createRestartPolicy, appendRotatedLog } from '@/desktop/restart-policy'

let tempDir: string | null = null

afterEach(async () => {
  delete process.env.USAGE_DATA_DIR
  if (tempDir) {
    await fs.rm(tempDir, { recursive: true, force: true })
    tempDir = null
  }
})

describe('restart policy', () => {
  it('backs off exponentially as 1s, 2s, 4s, 8s, 16s and stops after 5 crashes in 5 minutes', () => {
    const policy = createRestartPolicy({
      maxCrashes: 5,
      windowMs: 5 * 60_000,
      baseDelayMs: 1000,
      maxDelayMs: 30_000,
    })
    const t0 = 1_000_000

    expect(policy.recordExit(t0)).toEqual({
      shouldRestart: true,
      delayMs: 1000,
      crashesInWindow: 1,
    })
    expect(policy.recordExit(t0 + 10_000)).toEqual({
      shouldRestart: true,
      delayMs: 2000,
      crashesInWindow: 2,
    })
    expect(policy.recordExit(t0 + 20_000)).toEqual({
      shouldRestart: true,
      delayMs: 4000,
      crashesInWindow: 3,
    })
    expect(policy.recordExit(t0 + 30_000)).toEqual({
      shouldRestart: true,
      delayMs: 8000,
      crashesInWindow: 4,
    })
    expect(policy.recordExit(t0 + 40_000)).toEqual({
      shouldRestart: true,
      delayMs: 16_000,
      crashesInWindow: 5,
    })
    expect(policy.recordExit(t0 + 50_000)).toEqual({
      shouldRestart: false,
      delayMs: null,
      crashesInWindow: 6,
    })
  })

  it('drops crashes older than the 5-minute window and resets on recovery', () => {
    const policy = createRestartPolicy({
      maxCrashes: 5,
      windowMs: 5 * 60_000,
    })
    const t0 = 1_000_000
    for (let i = 0; i < 4; i++) {
      policy.recordExit(t0 + i * 1000)
    }
    const afterWindow = t0 + 5 * 60_000 + 5000
    expect(policy.recordExit(afterWindow)).toEqual({
      shouldRestart: true,
      delayMs: 1000,
      crashesInWindow: 1,
    })

    policy.reset()
    expect(policy.getCrashesInWindow(afterWindow)).toBe(0)
  })
})

describe('log rotation', () => {
  it('rotates desktop log file when it reaches the byte threshold', async () => {
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'usage-log-'))
    const file = path.join(tempDir, 'logs', 'main.log')

    appendRotatedLog(file, 'first-entry-1234567890', 20)
    appendRotatedLog(file, 'second-entry', 20)

    const current = await fs.readFile(file, 'utf8')
    const rotated = await fs.readFile(`${file}.1`, 'utf8')
    expect(current.trim()).toBe('second-entry')
    expect(rotated.trim()).toBe('first-entry-1234567890')
  })

  it('writes server log events under USAGE_DATA_DIR/logs/server.log', async () => {
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'usage-srv-log-'))
    process.env.USAGE_DATA_DIR = tempDir

    await logEvent('warn', 'cursor', 'SQLite busy', new Error('database is locked'))

    const content = await fs.readFile(path.join(tempDir, 'logs', 'server.log'), 'utf8')
    expect(content).toContain('[WARN] [cursor] SQLite busy (database is locked)')
  })
})
