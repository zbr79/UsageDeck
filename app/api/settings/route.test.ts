import { mkdtemp, rm } from 'fs/promises'
import { tmpdir } from 'os'
import path from 'path'
import { NextRequest } from 'next/server'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { GET, POST } from '@/app/api/settings/route'
import { getOrCreateSecret } from '@/lib/secret'

let dir: string

beforeEach(async () => {
  dir = await mkdtemp(path.join(tmpdir(), 'usage-settings-'))
  process.env.USAGE_DATA_DIR = dir
})

afterEach(async () => {
  delete process.env.USAGE_DATA_DIR
  await rm(dir, { recursive: true, force: true })
})

function request(body: unknown, secret?: string) {
  const headers = new Headers({ 'content-type': 'application/json' })
  if (secret) headers.set('x-usage-secret', secret)
  return new NextRequest('http://127.0.0.1:3100/api/settings', {
    method: 'POST',
    headers,
    body: JSON.stringify(body),
  })
}

describe('settings routes', () => {
  it('returns defaults before anything is saved', async () => {
    const res = await GET()
    const json = await res.json()
    expect(res.status).toBe(200)
    expect(json.settings.themeMode).toBe('auto')
    expect(json.settings.cursorEnabled).toBe(true)
  })

  it('requires the local secret before writing', async () => {
    const res = await POST(request({ themeMode: 'light' }))
    expect(res.status).toBe(401)
  })

  it('saves a theme change', async () => {
    const secret = await getOrCreateSecret()
    const saved = await POST(request({ themeMode: 'dark', panelTheme: 'light' }, secret))
    expect(saved.status).toBe(200)
    const json = await (await GET()).json()
    expect(json.settings.themeMode).toBe('dark')
    expect(json.settings.panelTheme).toBe('light')
  })
})
