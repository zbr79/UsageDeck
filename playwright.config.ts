import { defineConfig } from '@playwright/test'

const PORT = 3199

// E2E runs against a production build in .next-e2e (never the .next the
// desktop widget serves) with the API mocked per test via page.route().
export default defineConfig({
  testDir: 'tests/e2e',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [['github'], ['list']] : 'list',
  expect: { toHaveScreenshot: { maxDiffPixelRatio: 0.01 } },
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    channel: 'msedge',
    trace: 'retain-on-failure',
  },
  webServer: {
    command: `npx next build && npx next start -H 127.0.0.1 -p ${PORT}`,
    url: `http://127.0.0.1:${PORT}/widget`,
    reuseExistingServer: !process.env.CI,
    timeout: 300_000,
    env: { NEXT_DIST_DIR: '.next-e2e', USAGE_DATA_DIR: 'tests/.e2e-data' },
  },
})
