import { getClaudeUsage } from '@/lib/claude'

export const dynamic = 'force-dynamic'

export async function GET() {
  const { usage, stale, cachedAt } = await getClaudeUsage()
  return Response.json({ usage, stale, cachedAt, error: usage ? null : 'failed' })
}