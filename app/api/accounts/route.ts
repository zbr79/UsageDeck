import { getAccounts, saveAccounts } from '@/lib/opencode'
import { requireLocalSecret } from '@/lib/secret'
import { NextRequest } from 'next/server'

export const dynamic = 'force-dynamic'

/** "sk-…a1b2" — enough to recognise a key, useless to anyone who sees it. */
function keyHint(key: string): string {
  return key.length > 8 ? `${key.slice(0, 3)}…${key.slice(-4)}` : '•••'
}

export async function GET(req: NextRequest) {
  const denied = await requireLocalSecret(req)
  if (denied) return denied
  const accounts = await getAccounts()
  return Response.json({
    accounts: accounts.map((a) => ({ email: a.email, keyHint: keyHint(a.key) })),
  })
}

export async function POST(req: NextRequest) {
  const denied = await requireLocalSecret(req)
  if (denied) return denied
  const body = await req.json()
  const email = String(body?.email ?? '').trim()
  const key = String(body?.key ?? '').trim()
  if (!email || !key.startsWith('sk-')) {
    return Response.json({ error: 'email and a valid sk- key are required' }, { status: 400 })
  }
  const accounts = await getAccounts()
  const existing = accounts.find((a) => a.email.toLowerCase() === email.toLowerCase())
  if (existing) {
    existing.key = key
  } else {
    accounts.push({ email, key })
  }
  await saveAccounts(accounts)
  return Response.json({ ok: true })
}

export async function DELETE(req: NextRequest) {
  const denied = await requireLocalSecret(req)
  if (denied) return denied
  const body = await req.json()
  const email = String(body?.email ?? '').trim()
  if (!email) {
    return Response.json({ error: 'email is required' }, { status: 400 })
  }
  const accounts = await getAccounts()
  await saveAccounts(accounts.filter((a) => a.email.toLowerCase() !== email.toLowerCase()))
  return Response.json({ ok: true })
}
