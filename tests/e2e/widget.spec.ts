import AxeBuilder from '@axe-core/playwright'
import { expect, test, type Page } from '@playwright/test'
import { mockApi, setupWidget } from './fixtures'

test.use({ viewport: { width: 238, height: 700 } })

async function openWidget(page: Page, overrides: Parameters<typeof mockApi>[1] = {}) {
  await setupWidget(page)
  const seen = await mockApi(page, overrides)
  await page.goto('/widget')
  await expect(page.getByRole('region', { name: /^Cursor/ })).toBeVisible()
  return seen
}

const expand = (page: Page) => page.evaluate(() => (window as unknown as { __emit: (n: string) => void }).__emit('expand'))

test('shows the top account and every enabled provider, with the secret on each call', async ({ page }) => {
  const seen = await openWidget(page)
  const main = page.getByRole('region', { name: 'OpenCode account Main' })
  await expect(main).toBeVisible()
  await expect(main.getByRole('meter', { name: 'Monthly usage' })).toHaveAttribute('aria-valuenow', '85')
  await expect(page.getByRole('region', { name: /^OpenCode account/ })).toHaveCount(1)
  for (const name of [/^Cursor/, /^Grok Bot/, /^Codex/, /^Claude/]) {
    await expect(page.getByRole('region', { name })).toBeVisible()
  }
  await expect(main.getByText('Peak starts in 3m')).toBeVisible()
  expect(seen.every((r) => r.secret === 'test-secret')).toBe(true)
})

test('cards never touch: collapsed and expanded both keep a gap', async ({ page }) => {
  await openWidget(page)
  for (const state of ['collapsed', 'expanded']) {
    if (state === 'expanded') await expand(page)
    const boxes = await page.getByRole('region').evaluateAll((els) =>
      els.map((el) => el.getBoundingClientRect()).map((r) => ({ top: r.top, bottom: r.bottom })),
    )
    for (let i = 1; i < boxes.length; i++) {
      expect(boxes[i].top - boxes[i - 1].bottom, `${state} gap ${i}`).toBeGreaterThanOrEqual(5)
    }
  }
})

test('expanded view hides disabled and lapsed accounts, exhausted ones sink', async ({ page }) => {
  await openWidget(page)
  await expand(page)
  const names = await page.getByRole('region', { name: /^OpenCode account/ }).evaluateAll((els) =>
    els.map((el) => el.getAttribute('aria-label')),
  )
  expect(names).toEqual(['OpenCode account Main', 'OpenCode account Spare'])
})

test('cached, re-auth and exhausted states are shown consistently', async ({ page }) => {
  await openWidget(page)
  await expect(page.getByRole('region', { name: /^Grok Bot/ }).getByText('cached')).toBeVisible()
  await expect(page.getByRole('region', { name: /^Claude/ }).getByText('re-auth')).toBeVisible()
  const codex = page.getByRole('region', { name: /^Codex/ })
  const ring = await codex.evaluate((el) => getComputedStyle(el).borderTopColor)
  expect(ring).toBe('rgb(255, 100, 103)') // ringDanger (dark)
})

test('one failing provider does not blank the others', async ({ page }) => {
  await openWidget(page, { fail: ['cursor'] })
  await expect(page.getByRole('region', { name: /^Cursor/ }).getByText('Not signed in or unavailable')).toBeVisible()
  await expect(page.getByRole('region', { name: /^Claude/ }).getByRole('meter', { name: 'Weekly usage' })).toBeVisible()
  await expect(page.getByRole('status')).toHaveCount(0)
})

test('an unreachable server is reported, with a working retry', async ({ page }) => {
  await setupWidget(page)
  await page.route('**/api/**', (route) => route.abort())
  await page.goto('/widget')
  await expect(page.getByRole('status')).toContainText('Server unreachable')
  await page.unrouteAll()
  await mockApi(page)
  await page.getByRole('button', { name: 'Retry' }).click()
  await expect(page.getByRole('status')).toHaveCount(0)
  await expect(page.getByRole('region', { name: 'OpenCode account Main' })).toBeVisible()
})

test('copy-key button copies the key', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  const seen = await openWidget(page)
  await page.getByRole('button', { name: 'Copy API key for Main' }).click()
  await expect(page.getByText('Key copied')).toBeVisible()
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('sk-test-key-123')
  expect(seen.some((r) => r.path === '/api/key?email=main%40example.com')).toBe(true)
})

for (const themeMode of ['dark', 'light', 'white'] as const) {
  test(`${themeMode} theme passes axe (WCAG 2.2 AA) and matches the snapshot`, async ({ page }) => {
    await openWidget(page, { settings: { themeMode } })
    await expand(page)
    const results = await new AxeBuilder({ page })
      .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
      .analyze()
    expect(results.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target).join(' ')}`)).toEqual([])
    await expect(page.locator('main')).toHaveScreenshot(`widget-${themeMode}.png`, {
      animations: 'disabled',
    })
  })
}
