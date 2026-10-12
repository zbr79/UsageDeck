import { applySettingsPatch, getSettings, updateSettings } from '@/lib/settings'
import { readJsonBody } from '@/lib/http'
import { requireLocalSecret } from '@/lib/secret'
import { NextRequest } from 'next/server'

export const dynamic = 'force-dynamic'

export async function GET(req: NextRequest) {
  const denied = await requireLocalSecret(req)
  if (denied) return denied
  return Response.json({ settings: await getSettings() })
}

/** Partial update: only valid, known fields in the body are applied. */
export async function POST(req: NextRequest) {
  const denied = await requireLocalSecret(req)
  if (denied) return denied
  const parsed = await readJsonBody(req)
  if (!parsed.ok) return parsed.response
  const settings = await updateSettings((current) => applySettingsPatch(current, parsed.body))
  return Response.json({ ok: true, settings })
}
