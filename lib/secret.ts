import { promises as fs } from 'fs'
import path from 'path'
import crypto from 'crypto'
import { NextRequest } from 'next/server'
import { dataDir } from '@/lib/data-dir'

function secretFile(): string {
  return path.join(dataDir(), '.api-secret')
}

export async function getOrCreateSecret(): Promise<string> {
  const file = secretFile()
  try {
    const existing = (await fs.readFile(file, 'utf8')).trim()
    if (existing) return existing
  } catch {
    // create below
  }
  const secret = crypto.randomBytes(32).toString('hex')
  await fs.mkdir(path.dirname(file), { recursive: true })
  await fs.writeFile(file, secret, { encoding: 'utf8', mode: 0o600 })
  return secret
}

export async function requireLocalSecret(req: NextRequest): Promise<Response | null> {
  const secret = await getOrCreateSecret()
  const provided = req.headers.get('x-usage-secret')
  if (provided && provided === secret) return null
  return Response.json({ error: 'unauthorized' }, { status: 401 })
}
