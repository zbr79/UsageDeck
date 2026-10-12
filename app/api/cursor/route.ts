import { getCursorUsage } from '@/lib/cursor'
import { providerResponse } from '@/lib/http'
import { requireLocalSecret } from '@/lib/secret'
import { NextRequest } from 'next/server'

export const dynamic = 'force-dynamic'

export async function GET(req: NextRequest) {
  const denied = await requireLocalSecret(req)
  if (denied) return denied
  return providerResponse(await getCursorUsage())
}
