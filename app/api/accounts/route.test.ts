import { mkdtemp, readFile, rm } from 'fs/promises'
import { tmpdir } from 'os'
import path from 'path'
import { NextRequest } from 'next/server'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { DELETE, POST } from '@/app/api/accounts/route'
import { getOrCreateSecret } from '@/lib/secret'

let dir: string

beforeEach(async () => {
  dir = await mkdtemp(path.join(tmpdir(), 'usage-accounts-'))
  process.env.USAGE_DATA_DIR = dir
})

afterEach(async () => {
  delete process.env.USAGE_DATA_DIR
  await rm(dir, { recursive: true, force: true })
})

function request(method: string, body: unknown, secret?: string) {
  const headers = new Headers({ 'content-type': 'application/json' })
  if (secret) headers.set('x-usage-secret', secret)
  return new NextRequest('http://127.0.0.1:3100/api/accounts', {
    method,
    headers,
    body: JSON.stringify(body),
  })
}

async function accountsFile() {
  return readFile(path.join(dir, 'accounts.json'), 'utf8').catch(() => '')
}

describe('account routes', () => {
  it('requires the local secret on POST and DELETE', async () => {
    const post = await POST(request('POST', { email: 'a@example.com', key: 'sk-test' }))
    const del = await DELETE(request('DELETE', { email: 'a@example.com' }))
    expect(post.status).toBe(401)
    expect(del.status).toBe(401)
    expect(await accountsFile()).toBe('')
  })

  it('rejects a key that does not start with sk-', async () => {
    const secret = await getOrCreateSecret()
    const res = await POST(request('POST', { email: 'a@example.com', key: 'pk-nope' }, secret))
    expect(res.status).toBe(400)
    expect(await accountsFile()).toBe('')
  })

  it('rejects DELETE without an email', async () => {
    const secret = await getOrCreateSecret()
    const res = await DELETE(request('DELETE', { email: '  ' }, secret))
    expect(res.status).toBe(400)
  })
})
