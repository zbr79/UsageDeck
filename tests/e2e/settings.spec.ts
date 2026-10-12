import AxeBuilder from '@axe-core/playwright'
import { expect, test, type Page } from '@playwright/test'
import { mockApi, setupWidget } from './fixtures'

test.use({ viewport: { width: 1000, height: 760 } })

async function openSettings(page: Page, overrides: Parameters<typeof mockApi>[1] = {}) {
  await setupWidget(page)
  const seen = await mockApi(page, overrides)
  await page.goto('/settings')
  const dialog = page.getByRole('dialog', { name: 'Settings' })
  await expect(dialog).toBeVisible()
  await expect(dialog.getByText('main@example.com')).toBeVisible()
  return { seen, dialog }
}

test('tabs follow the WAI-ARIA pattern', async ({ page }) => {
  const { dialog } = await openSettings(page)
  const accountsTab = dialog.getByRole('tab', { name: 'Accounts' })
  await expect(accountsTab).toHaveAttribute('aria-selected', 'true')
  await accountsTab.focus()
  await page.keyboard.press('ArrowLeft')
  await expect(dialog.getByRole('tab', { name: 'General' })).toBeFocused()
  await expect(dialog.getByRole('tabpanel')).toContainText('Panel theme')
})

test('switches expose state and send only the changed field', async ({ page }) => {
  const { seen, dialog } = await openSettings(page)
  await dialog.getByRole('tab', { name: 'General' }).click()
  const sw = dialog.getByRole('switch', { name: 'Show provider names' })
  await expect(sw).toHaveAttribute('aria-checked', 'false')
  await sw.click()
  await expect(sw).toHaveAttribute('aria-checked', 'true')
  const post = seen.find((r) => r.method === 'POST' && r.path === '/api/settings')
  expect(post?.body).toEqual({ showProviderNames: true })
})

test('theme choices are a radio group', async ({ page }) => {
  const { seen, dialog } = await openSettings(page)
  await dialog.getByRole('tab', { name: 'General' }).click()
  const group = dialog.getByRole('radiogroup', { name: 'Widget theme' })
  await group.getByRole('radio', { name: 'white' }).click()
  await expect(group.getByRole('radio', { name: 'white' })).toHaveAttribute('aria-checked', 'true')
  expect(seen.some((r) => r.method === 'POST' && JSON.stringify(r.body) === '{"themeMode":"white"}')).toBe(true)
})

test('account details show a key hint and never fetch the full key', async ({ page }) => {
  const { seen, dialog } = await openSettings(page)
  await dialog.getByRole('button', { name: 'Details for main@example.com' }).click()
  const detail = page.getByRole('dialog', { name: 'OpenCode' })
  await expect(detail.getByText('sk-…ab12')).toBeVisible()
  expect(seen.some((r) => r.path.startsWith('/api/key'))).toBe(false)
})

test('Escape closes only the topmost dialog', async ({ page }) => {
  const { dialog } = await openSettings(page)
  await dialog.getByRole('button', { name: 'Details for main@example.com' }).click()
  await expect(page.getByRole('dialog', { name: 'OpenCode' })).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(page.getByRole('dialog', { name: 'OpenCode' })).toHaveCount(0)
  await expect(dialog).toBeVisible()
  expect(await page.evaluate(() => (window as unknown as { __closed?: boolean }).__closed)).toBeUndefined()
})

test('nickname saves once on Enter, not on every blur', async ({ page }) => {
  const { seen, dialog } = await openSettings(page)
  await dialog.getByRole('button', { name: 'Details for main@example.com' }).click()
  const detail = page.getByRole('dialog', { name: 'OpenCode' })
  const field = detail.getByLabel('Nickname')
  await field.focus()
  await field.blur() // unchanged: no request
  await field.fill('Primary')
  await field.press('Enter')
  await expect(detail.getByText('✓ saved')).toBeVisible()
  const posts = seen.filter((r) => r.method === 'POST' && r.path === '/api/settings')
  expect(posts.map((p) => p.body)).toEqual([{ displayNames: { 'main@example.com': 'Primary' } }])
})

test('adding an account validates the key and posts it', async ({ page }) => {
  const { seen, dialog } = await openSettings(page)
  await dialog.getByRole('button', { name: 'Add', exact: true }).click()
  await dialog.getByLabel('Email or label').fill('new@example.com')
  await dialog.getByLabel('API key').fill('not-a-key')
  await dialog.getByRole('button', { name: 'Add account' }).click()
  expect(seen.some((r) => r.method === 'POST' && r.path === '/api/accounts')).toBe(false)
  await dialog.getByLabel('API key').fill('sk-new-key')
  await dialog.getByRole('button', { name: 'Add account' }).click()
  await expect(page.getByText('Account added')).toBeVisible()
  expect(seen.find((r) => r.method === 'POST' && r.path === '/api/accounts')?.body).toEqual({
    email: 'new@example.com',
    key: 'sk-new-key',
  })
})

test('removing an account asks for confirmation', async ({ page }) => {
  const { seen, dialog } = await openSettings(page)
  await dialog.getByRole('button', { name: 'Remove spare@example.com' }).click()
  expect(seen.some((r) => r.method === 'DELETE')).toBe(false)
  await dialog.getByRole('button', { name: 'Remove', exact: true }).click()
  await expect(page.getByText('Account removed')).toBeVisible()
  expect(seen.find((r) => r.method === 'DELETE')?.body).toEqual({ email: 'spare@example.com' })
})

for (const panelTheme of ['dark', 'light'] as const) {
  test(`${panelTheme} panel passes axe (WCAG 2.2 AA) on every tab`, async ({ page }) => {
    const { dialog } = await openSettings(page, { settings: { panelTheme } })
    const axe = () =>
      new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa']).analyze()
    const check = async (label: string) => {
      const { violations } = await axe()
      expect(violations.map((v) => `${label} ${v.id}: ${v.nodes.map((n) => n.target).join(' ')}`)).toEqual([])
    }
    await check('accounts/opencode')
    await expect(dialog).toHaveScreenshot(`settings-accounts-${panelTheme}.png`, { animations: 'disabled' })
    for (const tab of ['Cursor', 'Grok Bot', 'Codex', 'Claude']) {
      await dialog.getByRole('tab', { name: tab }).click()
      await check(`accounts/${tab}`)
    }
    await dialog.getByRole('tab', { name: 'General' }).click()
    await check('general')
    await expect(dialog).toHaveScreenshot(`settings-${panelTheme}.png`, { animations: 'disabled' })
  })
}
