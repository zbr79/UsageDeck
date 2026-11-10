import { promises as fs } from 'fs'
import path from 'path'
import { DATA_DIR } from './paths'
import crypto from 'crypto'
import { NextRequest } from 'next/server'

const FILE = path.join(DATA_DIR, '.api-secret')

export async function getOrCreateSecret(): Promise<string> {
  try {
    const existing = (await fs.readFile(FILE, 'utf8')).trim()
    if (existing) return existing
  } catch {
    // create below
  }
  const secret = crypto.randomBytes(32).toString('hex')
  await fs.mkdir(path.dirname(FILE), { recursive: true })
  await fs.writeFile(FILE, secret, { encoding: 'utf8', mode: 0o600 })
  return secret
}

export async function requireLocalSecret(req: NextRequest): Promise<Response | null> {
  const secret = await getOrCreateSecret()
  const provided = req.headers.get('x-usage-secret')
  if (provided && provided === secret) return null
  return Response.json({ error: 'unauthorized' }, { status: 401 })
}
