import { getCursorUsage } from '@/lib/cursor'

export const dynamic = 'force-dynamic'

export async function GET() {
  const { usage, stale, cachedAt } = await getCursorUsage()
  return Response.json({ usage, error: usage ? null : 'failed', stale, cachedAt })
}
