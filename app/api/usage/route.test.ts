import { mkdtemp, rm, writeFile } from 'fs/promises'
import { tmpdir } from 'os'
import path from 'path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { GET } from '@/app/api/usage/route'

let dir: string

const usage = {
  rolling: { status: 'ok', percent: 10, resetsAt: '2026-10-15T01:00:00.000Z' },
  weekly: { status: 'ok', percent: 20, resetsAt: '2026-10-18T00:00:00.000Z' },
  monthly: { status: 'ok', percent: 30, resetsAt: '2026-11-01T00:00:00.000Z' },
}

beforeEach(async () => {
  dir = await mkdtemp(path.join(tmpdir(), 'usage-route-'))
  process.env.USAGE_DATA_DIR = dir
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => new Response(JSON.stringify({ usage }), { status: 200 })),
  )
})

afterEach(async () => {
  vi.unstubAllGlobals()
  delete process.env.USAGE_DATA_DIR
  await rm(dir, { recursive: true, force: true })
})

describe('usage route', () => {
  it('returns parsed usage for a stored account', async () => {
    await writeFile(
      path.join(dir, 'accounts.json'),
      JSON.stringify([{ email: 'a@example.com', key: 'sk-stored' }]),
    )
    await writeFile(
      path.join(dir, 'settings.json'),
      JSON.stringify({ disabledAccounts: [] }),
    )
    const res = await GET()
    const json = await res.json()
    expect(res.status).toBe(200)
    expect(json.accounts).toEqual([
      expect.objectContaining({
        email: 'a@example.com',
        enabled: true,
        error: null,
        usage,
      }),
    ])
  })
})
