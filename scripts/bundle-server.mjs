import { cpSync, existsSync, mkdirSync, rmSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const target = path.join(root, 'desktop', 'server')

const copies = [
  ['.next/standalone', ''],
  ['.next/static', '.next/static'],
  ['public', 'public'],
]

if (!existsSync(path.join(root, '.next', 'standalone'))) {
  console.error('Missing .next/standalone. Run "next build" first.')
  process.exit(1)
}

rmSync(target, { recursive: true, force: true })
mkdirSync(target, { recursive: true })
for (const [from, to] of copies) {
  cpSync(path.join(root, from), path.join(target, to), { recursive: true })
}
console.log(`Server bundle written to ${path.relative(root, target)}`)
