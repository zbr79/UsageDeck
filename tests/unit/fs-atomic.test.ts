import { readdirSync, readFileSync } from 'node:fs'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { tempDir } from '../helpers/temp'
import { withLock, writeJsonAtomic } from '@/lib/fs-atomic'

let dir: string
let cleanup: () => void
beforeEach(() => ({ dir, cleanup } = tempDir()))
afterEach(() => cleanup())

describe('writeJsonAtomic', () => {
  it('writes the file and leaves no temp files behind', async () => {
    const file = path.join(dir, 'nested', 'x.json')
    await writeJsonAtomic(file, { a: 1 })
    await writeJsonAtomic(file, { a: 2 })
    expect(JSON.parse(readFileSync(file, 'utf8'))).toEqual({ a: 2 })
    expect(readdirSync(path.dirname(file))).toEqual(['x.json'])
  })
})

describe('withLock', () => {
  it('runs critical sections one at a time, in order', async () => {
    const log: string[] = []
    const task = (name: string, ms: number) =>
      withLock('k', async () => {
        log.push(`start ${name}`)
        await new Promise((r) => setTimeout(r, ms))
        log.push(`end ${name}`)
      })
    await Promise.all([task('a', 20), task('b', 1), task('c', 5)])
    expect(log).toEqual(['start a', 'end a', 'start b', 'end b', 'start c', 'end c'])
  })

  it('keeps going after a failed section', async () => {
    await expect(withLock('k2', async () => Promise.reject(new Error('boom')))).rejects.toThrow('boom')
    await expect(withLock('k2', async () => 'ok')).resolves.toBe('ok')
  })
})
