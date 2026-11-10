import { isOpenCodeKey, OPENCODE_KEY_MAX_LENGTH, OPENCODE_KEY_PREFIXES } from "@/lib/opencode-key";

/**
 * The inline error for an OpenCode key field, or null when it is valid or still empty (the
 * primary button stays disabled while empty). The field's value is trimmed before saving.
 */
export function keyError(raw: string): string | null {
  const key = raw.trim();
  if (!key || isOpenCodeKey(key)) return null;
  if (key.length > OPENCODE_KEY_MAX_LENGTH) return "This key is too long.";
  if (/\s/.test(key)) return "Remove the spaces from the key.";
  if (OPENCODE_KEY_PREFIXES.some((p) => key === p)) return "The key is incomplete.";
  return "OpenCode keys start with oc_sk_ (or an older sk-).";
}
