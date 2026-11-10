import path from 'path'

export function dataDir(): string {
  const override = process.env.USAGE_DATA_DIR
  if (override && override.trim()) return path.resolve(override)
  return path.join(process.cwd(), 'data')
}
