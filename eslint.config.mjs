import { defineConfig, globalIgnores } from 'eslint/config'
import nextVitals from 'eslint-config-next/core-web-vitals'
import nextTs from 'eslint-config-next/typescript'

export default defineConfig([
  ...nextVitals,
  ...nextTs,
  {
    // Electron main/preload are CommonJS Node scripts.
    files: ['desktop/**/*.js'],
    rules: { '@typescript-eslint/no-require-imports': 'off' },
  },
  {
    // Provider payloads are untyped JSON; parsers narrow them explicitly.
    files: ['lib/cursor.ts', 'lib/codex.ts'],
    rules: { '@typescript-eslint/no-explicit-any': 'off' },
  },
  globalIgnores([
    '.next/**',
    '.next-e2e/**',
    'out/**',
    'build/**',
    'next-env.d.ts',
    'desktop/node_modules/**',
    'test-results/**',
    'playwright-report/**',
  ]),
])
