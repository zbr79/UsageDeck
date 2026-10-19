/**
 * OpenCode API key formats: the current `oc_sk_<12 hex>_<32 url-safe>` keys and the older `sk-…`
 * ones. Mirrors `valid_api_key` in src-tauri/src/api.rs, which is the check that counts; this one
 * only lets the form say what is wrong before it is sent. The key is sent exactly as entered.
 */
export const OPENCODE_KEY_PREFIXES = ["oc_sk_", "sk-"] as const;

/** Same limit as MAX_KEY_LENGTH in api.rs (UTF-16 units, like the input's maxLength). */
export const OPENCODE_KEY_MAX_LENGTH = 512;

/**
 * For the input's `pattern` attribute (the browser anchors it and compiles it with the `v` flag):
 * a known prefix, then at least one more character and no whitespace.
 */
export const OPENCODE_KEY_PATTERN = "(?:oc_sk_|sk-)\\S+";

export const OPENCODE_KEY_PLACEHOLDER = "oc_sk_…";

export const OPENCODE_KEY_TITLE = "An OpenCode API key starting with oc_sk_ (or an older sk- key)";

export function isOpenCodeKey(key: string): boolean {
  return (
    key.length <= OPENCODE_KEY_MAX_LENGTH &&
    OPENCODE_KEY_PREFIXES.some((p) => key.startsWith(p) && key.length > p.length) &&
    !/\s/.test(key)
  );
}
