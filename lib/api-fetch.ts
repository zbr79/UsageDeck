import { settingsWindow, usageWidget } from '@/lib/desktop-bridge'

export function apiFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const secret = usageWidget()?.secret ?? settingsWindow()?.secret
  const headers = new Headers(init?.headers)
  if (secret) headers.set('x-usage-secret', secret)
  return fetch(input, { ...init, headers })
}
