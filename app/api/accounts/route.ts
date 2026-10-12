import { getAccounts, keyHint, updateAccounts } from '@/lib/opencode'
import { forgetAccount } from '@/lib/settings'
import { readJsonBody } from '@/lib/http'
import { requireLocalSecret } from '@/lib/secret'
import type { OpenCodeAccountSummary } from '@/lib/types'
import { NextRequest } from 'next/server'

export const dynamic = 'force-dynamic'

const MAX_EMAIL_LENGTH = 320
const MAX_KEY_LENGTH = 512

export async function GET(req: NextRequest) {
  const denied = await requireLocalSecret(req)
  if (denied) return denied
  const accounts: OpenCodeAccountSummary[] = (await getAccounts()).map((a) => ({
    email: a.email,
    keyHint: keyHint(a.key),
  }))
  return Response.json({ accounts })
}

export async function POST(req: NextRequest) {
  const denied = await requireLocalSecret(req)
  if (denied) return denied
  const parsed = await readJsonBody(req)
  if (!parsed.ok) return parsed.response
  const body = parsed.body as { email?: unknown; key?: unknown } | null
  const email = String(body?.email ?? '').trim()
  const key = String(body?.key ?? '').trim()
  if (!email || email.length > MAX_EMAIL_LENGTH) {
    return Response.json({ error: 'an email or label is required' }, { status: 400 })
  }
  if (!key.startsWith('sk-') || key.length > MAX_KEY_LENGTH || /\s/.test(key)) {
    return Response.json({ error: 'a valid sk- key is required' }, { status: 400 })
  }
  await updateAccounts((accounts) => {
    const existing = accounts.find((a) => a.email.toLowerCase() === email.toLowerCase())
    if (existing) {
      existing.key = key
      return accounts
    }
    return [...accounts, { email, key }]
  })
  return Response.json({ ok: true })
}

export async function DELETE(req: NextRequest) {
  const denied = await requireLocalSecret(req)
  if (denied) return denied
  const parsed = await readJsonBody(req)
  if (!parsed.ok) return parsed.response
  const email = String((parsed.body as { email?: unknown } | null)?.email ?? '').trim()
  if (!email) {
    return Response.json({ error: 'email is required' }, { status: 400 })
  }
  await updateAccounts((accounts) =>
    accounts.filter((a) => a.email.toLowerCase() !== email.toLowerCase()),
  )
  await forgetAccount(email)
  return Response.json({ ok: true })
}
