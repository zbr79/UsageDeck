export function toIsoTimestamp(value: unknown): string | null {
  if (typeof value === 'number' && Number.isFinite(value)) {
    const ms = value < 1e12 ? value * 1000 : value
    const d = new Date(ms)
    return Number.isNaN(d.getTime()) ? null : d.toISOString()
  }
  if (typeof value === 'string' && value.trim()) {
    const trimmed = value.trim()
    if (/^\d+(\.\d+)?$/.test(trimmed)) return toIsoTimestamp(Number(trimmed))
    const d = new Date(trimmed)
    return Number.isNaN(d.getTime()) ? null : d.toISOString()
  }
  return null
}
