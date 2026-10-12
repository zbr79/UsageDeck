import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { jsonResponse, tempDir } from '../helpers/temp'

// The app must never modify credential files owned by Claude Code / Codex:
// refreshing a token there rotates the refresh token and logs the owner out.

let home: string
let cleanup: () => void
const seeded = new Map<string, string>()

function seed(rel: string, value: unknown) {
  const file = path.join(home, rel)
  mkdirSync(path.dirname(file), { recursive: true })
  const text = JSON.stringify(value, null, 2)
  writeFileSync(file, text)
  seeded.set(file, text)
}

function expectUntouched() {
  for (const [file, text] of seeded) {
    expect(readFileSync(file, 'utf8'), file).toBe(text)
  }
}

function stubFetch(status: number) {
  const fetch = vi.fn(async (_url: string | URL | Request) => jsonResponse({ error: 'nope' }, status))
  vi.stubGlobal('fetch', fetch)
  return fetch
}

const calledUrls = (fetch: ReturnType<typeof stubFetch>) =>
  fetch.mock.calls.map(([u]) => String(u instanceof Request ? u.url : u))

beforeEach(() => {
  ;({ dir: home, cleanup } = tempDir('usage-home-'))
  seeded.clear()
  // Point every credential location at the temp "home" (paths are resolved
  // at module load, hence resetModules + dynamic import below).
  vi.stubEnv('USERPROFILE', home)
  vi.stubEnv('HOME', home)
  vi.stubEnv('APPDATA', path.join(home, 'AppData', 'Roaming'))
  vi.stubEnv('LOCALAPPDATA', path.join(home, 'AppData', 'Local'))
  vi.stubEnv('USAGE_DATA_DIR', path.join(home, 'data'))
  vi.resetModules()
})
afterEach(() => cleanup())

describe('Claude credentials', () => {
  it('an expired CLI token is reported, not refreshed', async () => {
    seed('.claude/.credentials.json', {
      claudeAiOauth: {
        accessToken: 'at',
        refreshToken: 'rt',
        expiresAt: Date.now() - 60_000,
        subscriptionType: 'pro',
      },
      mcpOAuth: { server: { accessToken: 'mcp' } },
    })
    const fetch = stubFetch(200)
    const { getClaudeUsage } = await import('@/lib/claude')
    const r = await getClaudeUsage()
    expect(r.authExpired).toBe(true)
    expect(r.usage?.planName).toBe('Pro')
    expect(calledUrls(fetch)).toEqual([]) // no usage call, no token endpoint
    expectUntouched()
  })

  it('a rejected live token is reported, not refreshed', async () => {
    seed('.claude/.credentials.json', {
      claudeAiOauth: { accessToken: 'at', refreshToken: 'rt', expiresAt: Date.now() + 3_600_000 },
    })
    const fetch = stubFetch(401)
    const { getClaudeUsage } = await import('@/lib/claude')
    const r = await getClaudeUsage()
    expect(r.authExpired).toBe(true)
    expect(calledUrls(fetch)).toEqual(['https://api.anthropic.com/api/oauth/usage'])
    expectUntouched()
  })
})

describe('Codex credentials', () => {
  it('a 401 is reported and the refresh token is never spent', async () => {
    seed('.codex/auth.json', {
      tokens: { access_token: 'at', refresh_token: 'rt', account_id: 'acc' },
      last_refresh: '2026-01-01T00:00:00Z',
    })
    const fetch = stubFetch(401)
    const { getCodexUsage } = await import('@/lib/codex')
    const r = await getCodexUsage()
    expect(r.authExpired).toBe(true)
    expect(calledUrls(fetch)).toEqual(['https://chatgpt.com/backend-api/wham/usage'])
    expectUntouched()
  })
})
