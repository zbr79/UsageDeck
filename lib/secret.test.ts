import { mkdtemp, rm } from 'fs/promises'
import { tmpdir } from 'os'
import path from 'path'
import { NextRequest } from 'next/server'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { getOrCreateSecret, requireLocalSecret } from '@/lib/secret'

let dir: string

beforeEach(async () => {
  dir = await mkdtemp(path.join(tmpdir(), 'usage-secret-'))
  process.env.USAGE_DATA_DIR = dir
})

afterEach(async () => {
  delete process.env.USAGE_DATA_DIR
  await rm(dir, { recursive: true, force: true })
})

function request(secret?: string) {
  const headers = new Headers()
  if (secret !== undefined) headers.set('x-usage-secret', secret)
  return new NextRequest('http://127.0.0.1:3100/api/accounts', { headers })
}

describe('requireLocalSecret', () => {
  it('rejects a missing or mismatched header', async () => {
    const secret = await getOrCreateSecret()
    const missing = await requireLocalSecret(request())
    const mismatch = await requireLocalSecret(request('not-the-secret'))
    expect(missing?.status).toBe(401)
    expect(mismatch?.status).toBe(401)
    expect(secret).not.toBe('not-the-secret')
  })

  it('accepts the local secret', async () => {
    const secret = await getOrCreateSecret()
    await expect(requireLocalSecret(request(secret))).resolves.toBeNull()
  })
})
