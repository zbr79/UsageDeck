import { describe, expect, it } from "vitest";
import { decryptKey, encryptKey, isEncryptedKey } from "@/lib/key-crypto";

describe("key crypto", () => {
  it("round-trips a key and leaves plaintext untouched", () => {
    const stored = encryptKey("sk-secret", "local-secret");
    expect(isEncryptedKey(stored)).toBe(true);
    expect(stored.includes("sk-secret")).toBe(false);
    expect(decryptKey(stored, "local-secret")).toBe("sk-secret");
    expect(decryptKey("sk-plain", "local-secret")).toBe("sk-plain");
  });

  it("rejects a tampered ciphertext", () => {
    const stored = encryptKey("sk-secret", "local-secret");
    const flipped = `${stored.slice(0, -1)}${stored.endsWith("a") ? "b" : "a"}`;
    expect(() => decryptKey(flipped, "local-secret")).toThrow();
    expect(() => decryptKey(stored, "other-secret")).toThrow();
    expect(() => decryptKey("enc:v1:not-valid", "local-secret")).toThrow();
  });
});
