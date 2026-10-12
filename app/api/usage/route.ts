import { getAccounts, getUsage, getLastChange } from '@/lib/opencode'
import { getSettings } from '@/lib/settings'
import { requireLocalSecret } from '@/lib/secret'
import { NextRequest } from 'next/server'

export const dynamic = 'force-dynamic'

export async function GET(req: NextRequest) {
  const denied = await requireLocalSecret(req)
  if (denied) return denied
  const [accounts, settings] = await Promise.all([getAccounts(), getSettings()])
  const results = await Promise.all(
    accounts.map(async (acc) => {
      const enabled = !settings.disabledAccounts.includes(acc.email.toLowerCase())
      const { usage, prev, stale, cachedAt, subscriptionActive } = await getUsage(acc.key, acc.email)
      const change = getLastChange(acc.email)
      return {
        email: acc.email,
        usage,
        prev,
        enabled,
        error: usage ? null : 'failed',
        stale,
        cachedAt,
        subscriptionActive,
        lastChangeAt: change?.at ?? null,
        lastChangeDetail: change?.detail ?? null,
      }
    }),
  )
  return Response.json({ accounts: results })
}