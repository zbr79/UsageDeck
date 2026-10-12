import type { ProviderResult } from './provider-cache'
import type { ProviderResponse } from './types'

/** Parse a JSON request body; malformed JSON is a 400, not a 500. */
export async function readJsonBody(
  req: Request,
): Promise<{ ok: true; body: unknown } | { ok: false; response: Response }> {
  try {
    return { ok: true, body: await req.json() }
  } catch {
    return { ok: false, response: Response.json({ error: 'invalid JSON body' }, { status: 400 }) }
  }
}

export function providerResponse<T>(result: ProviderResult<T>): Response {
  const body: ProviderResponse<T> = {
    usage: result.usage,
    stale: result.stale,
    cachedAt: result.cachedAt,
    authExpired: result.authExpired,
    error: result.usage ? null : 'failed',
  }
  return Response.json(body)
}
