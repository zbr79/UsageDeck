import { getCodexUsage } from '@/lib/codex'
import { requireLocalSecret } from '@/lib/secret'
import { NextRequest } from 'next/server'

export const dynamic = 'force-dynamic'

export async function GET(req: NextRequest) {
  const denied = await requireLocalSecret(req)
  if (denied) return denied
  const { usage, stale, cachedAt, authExpired } = await getCodexUsage()
  return Response.json({ usage, stale, cachedAt, authExpired, error: usage ? null : 'failed' })
}
