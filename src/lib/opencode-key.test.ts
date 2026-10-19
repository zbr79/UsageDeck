import { describe, expect, it } from "vitest";
import {
  isOpenCodeKey,
  OPENCODE_KEY_MAX_LENGTH,
  OPENCODE_KEY_PATTERN,
  OPENCODE_KEY_PLACEHOLDER,
} from "./opencode-key";

// made-up keys only, never real ones
const NEW_KEY = "oc_sk_000000000000_AAAAAAAAAAAAAAAAAAAAAAAAAAAA1234";
const OLD_KEY = "sk-0000000000005678";
const BAD = [
  "",
  "oc_sk_",
  "sk-",
  "oc_sk_000000000000_AAAA AAAA",
  "oc_sk_000000000000_AAAA\tAAAA",
  " oc_sk_000000000000_AAAA",
  "pk-0000000000005678",
  "oc-sk-000000000000_AAAA",
  "OC_SK_000000000000_AAAA",
  "sk_0000000000005678",
  "xoc_sk_000000000000_AAAA",
];

/** How the browser applies an input's `pattern` attribute. */
const patternMatches = (value: string) =>
  new RegExp(`^(?:${OPENCODE_KEY_PATTERN})$`, "v").test(value);

describe("OpenCode API key format", () => {
  it("accepts the new oc_sk_ keys and the older sk- keys", () => {
    for (const key of [NEW_KEY, OLD_KEY]) {
      expect(isOpenCodeKey(key)).toBe(true);
      expect(patternMatches(key)).toBe(true);
    }
  });

  it("rejects whitespace and other prefixes", () => {
    for (const key of BAD) {
      expect(isOpenCodeKey(key), JSON.stringify(key)).toBe(false);
      // an empty value is caught by `required`, not by the pattern
      if (key) expect(patternMatches(key), JSON.stringify(key)).toBe(false);
    }
  });

  it("caps the length like the Rust check", () => {
    const at = "oc_sk_" + "A".repeat(OPENCODE_KEY_MAX_LENGTH - 6);
    expect(isOpenCodeKey(at)).toBe(true);
    expect(isOpenCodeKey(at + "A")).toBe(false);
  });

  it("shows the new format as the placeholder", () => {
    expect(OPENCODE_KEY_PLACEHOLDER.startsWith("oc_sk_")).toBe(true);
  });
});
