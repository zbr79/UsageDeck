// Client-side access to the local API. Every request carries the shared
// secret injected by the Electron preload (the server rejects it otherwise).

function secret(): string | undefined {
  if (typeof window === 'undefined') return undefined
  return window.widget?.secret ?? window.settingsWindow?.secret
}

export function apiFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const headers = new Headers(init?.headers)
  const s = secret()
  if (s) headers.set('x-usage-secret', s)
  return fetch(input, { cache: 'no-store', ...init, headers })
}

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message)
  }
}

async function parse<T>(res: Response): Promise<T> {
  const body = await res.json().catch(() => null)
  if (!res.ok) {
    const message = typeof body?.error === 'string' ? body.error : `request failed (${res.status})`
    throw new ApiError(res.status, message)
  }
  return body as T
}

export async function getJson<T>(path: string): Promise<T> {
  return parse<T>(await apiFetch(path))
}

export async function sendJson<T>(
  path: string,
  method: 'POST' | 'DELETE',
  body: unknown,
): Promise<T> {
  return parse<T>(
    await apiFetch(path, {
      method,
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    }),
  )
}

// Settings changes are broadcast so the widget updates live, not only when
// the settings window closes.
const CHANNEL = 'usage-monitor:settings'

export function announceSettingsChanged(): void {
  if (typeof BroadcastChannel === 'undefined') return
  const ch = new BroadcastChannel(CHANNEL)
  ch.postMessage('changed')
  ch.close()
}

export function onSettingsChanged(cb: () => void): () => void {
  if (typeof BroadcastChannel === 'undefined') return () => {}
  const ch = new BroadcastChannel(CHANNEL)
  ch.onmessage = () => cb()
  return () => ch.close()
}
