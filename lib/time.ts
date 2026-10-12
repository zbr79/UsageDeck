/**
 * Normalise a provider timestamp (ISO string, epoch seconds or epoch ms,
 * number or numeric string) to ISO-8601. Returns null instead of throwing
 * on anything unparseable.
 */
export function toIso(value: unknown): string | null {
  if (typeof value === "number" && Number.isFinite(value)) {
    const ms = value < 1e12 ? value * 1000 : value;
    const d = new Date(ms);
    return Number.isNaN(d.getTime()) ? null : d.toISOString();
  }
  if (typeof value === "string" && value.trim()) {
    const trimmed = value.trim();
    if (/^\d+(\.\d+)?$/.test(trimmed)) return toIso(Number(trimmed));
    const d = new Date(trimmed);
    return Number.isNaN(d.getTime()) ? null : d.toISOString();
  }
  return null;
}

export function numberOrNull(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

export function stringOrNull(value: unknown): string | null {
  return typeof value === "string" && value ? value : null;
}
