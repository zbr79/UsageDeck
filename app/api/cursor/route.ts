import { getCursorUsage } from '@/lib/cursor'
import { requireLocalSecret } from '@/lib/secret'
import { NextRequest } from 'next/server'

export const dynamic = 'force-dynamic'

export async function GET(req: NextRequest) {
  const denied = await requireLocalSecret(req)
  if (denied) return denied
  const { usage, stale, cachedAt } = await getCursorUsage()
  return Response.json({ usage, stale, cachedAt, error: usage ? null : 'failed' })
}
