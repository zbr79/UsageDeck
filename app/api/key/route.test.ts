import { mkdtemp, rm, writeFile } from 'fs/promises'
import { tmpdir } from 'os'
import path from 'path'
import { NextRequest } from 'next/server'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { GET } from '@/app/api/key/route'
import { getOrCreateSecret } from '@/lib/secret'

let dir: string

beforeEach(async () => {
  dir = await mkdtemp(path.join(tmpdir(), 'usage-key-'))
  process.env.USAGE_DATA_DIR = dir
})

afterEach(async () => {
  delete process.env.USAGE_DATA_DIR
  await rm(dir, { recursive: true, force: true })
})

function request(email: string, secret?: string) {
  const headers = new Headers()
  if (secret) headers.set('x-usage-secret', secret)
  return new NextRequest(`http://127.0.0.1:3100/api/key?email=${encodeURIComponent(email)}`, {
    headers,
  })
}

describe('key route', () => {
  it('requires the local secret', async () => {
    const res = await GET(request('a@example.com'))
    expect(res.status).toBe(401)
  })

  it('returns 404 for an unknown account', async () => {
    const secret = await getOrCreateSecret()
    const res = await GET(request('missing@example.com', secret))
    expect(res.status).toBe(404)
  })

  it('returns the decrypted key for a stored account', async () => {
    const secret = await getOrCreateSecret()
    await writeFile(
      path.join(dir, 'accounts.json'),
      JSON.stringify([{ email: 'a@example.com', key: 'sk-stored' }]),
    )
    const res = await GET(request('a@example.com', secret))
    expect(res.status).toBe(200)
    expect(await res.json()).toEqual({ key: 'sk-stored' })
    const stored = await import('fs/promises').then((fs) =>
      fs.readFile(path.join(dir, 'accounts.json'), 'utf8'),
    )
    expect(stored).not.toContain('sk-stored')
    expect(stored).toContain('enc:v1:')
  })
})
