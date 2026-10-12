import { getCodexUsage } from '@/lib/codex'

export const dynamic = 'force-dynamic'

export async function GET() {
  const { usage, stale, cachedAt, authExpired } = await getCodexUsage()
  return Response.json({ usage, stale, cachedAt, authExpired, error: usage ? null : 'failed' })
}
