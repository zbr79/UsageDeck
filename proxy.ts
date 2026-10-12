import { NextResponse, type NextRequest } from 'next/server'
import { isAllowedHost } from '@/lib/host'
import { getOrCreateSecret, secretsMatch, SECRET_HEADER } from '@/lib/secret'

/**
 * Network boundary for the local server:
 *  1. only loopback Host headers are served (blocks DNS rebinding);
 *  2. every /api request must carry the shared secret the Electron shell
 *     injects, so other local pages can't read usage, emails or keys.
 */
export async function proxy(req: NextRequest) {
  if (!isAllowedHost(req.headers.get('host'))) {
    return new NextResponse('Forbidden', { status: 403 })
  }
  if (req.nextUrl.pathname.startsWith('/api/')) {
    const secret = await getOrCreateSecret()
    if (!secretsMatch(req.headers.get(SECRET_HEADER), secret)) {
      return NextResponse.json({ error: 'unauthorized' }, { status: 401 })
    }
  }
  return NextResponse.next()
}

export const config = {
  matcher: ['/((?!_next/static|_next/image).*)'],
}
