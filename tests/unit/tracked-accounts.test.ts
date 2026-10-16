import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { tempDir } from "../helpers/temp";
import { accountKey, isAccountHidden } from "@/lib/account-key";
import { emailFromJwt } from "@/lib/codex";
import { collectAccounts } from "@/lib/tracked-accounts";
import { resetUsageCacheForTests, writeCached } from "@/lib/usage-cache";
import type { ProviderResult } from "@/lib/provider-cache";

let cleanup: () => void;

const result = <T>(data: T | null, extra: Partial<ProviderResult<T>> = {}): ProviderResult<T> => ({
  usage: data,
  stale: false,
  cachedAt: null,
  authExpired: false,
  ...extra,
});

beforeEach(() => {
  const t = tempDir();
  cleanup = t.cleanup;
  vi.stubEnv("USAGE_DATA_DIR", t.dir);
  resetUsageCacheForTests();
});

afterEach(() => {
  resetUsageCacheForTests();
  cleanup();
});

describe("account keys", () => {
  it("prefixes and lowercases so a provider account cannot hide an OpenCode email", () => {
    expect(accountKey("cursor", " A@X.com ")).toBe("cursor:a@x.com");
    expect(isAccountHidden(["a@x.com"], "cursor", "a@x.com")).toBe(false);
    expect(isAccountHidden(["cursor:a@x.com"], "cursor", "A@x.com")).toBe(true);
  });
});

describe("emailFromJwt", () => {
  it("reads the email claim and ignores a token with no payload", () => {
    const payload = Buffer.from(JSON.stringify({ email: " A@x.com " })).toString("base64url");
    expect(emailFromJwt(`hdr.${payload}.sig`)).toBe("A@x.com");
    expect(emailFromJwt("not-a-jwt")).toBeNull();
    expect(emailFromJwt(undefined)).toBeNull();
  });
});

describe("collectAccounts", () => {
  it("keeps an account after the app switches away and serves its cached reading", async () => {
    await collectAccounts("cursor", "cursor", [
      {
        id: "old@x.com",
        label: "old@x.com",
        active: true,
        load: async () => {
          await writeCached("cursor", "old@x.com", { n: 1 });
          return result({ n: 1 });
        },
      },
    ]);
    const rows = await collectAccounts("cursor", "cursor", [
      {
        id: "new@x.com",
        label: "new@x.com",
        active: true,
        load: async () => result({ n: 2 }),
      },
    ]);
    expect(rows.map((r) => [r.id, r.active, r.usage, r.stale])).toEqual([
      ["new@x.com", true, { n: 2 }, false],
      ["old@x.com", false, { n: 1 }, true],
    ]);
  });

  it("uses the legacy single-account cache only while one account is known", async () => {
    await writeCached("cursor", "default", { n: 7 });
    const [only] = await collectAccounts("cursor", "cursor", [
      {
        id: "a@x.com",
        label: "a@x.com",
        active: true,
        load: async () => result(null),
      },
    ]);
    expect(only).toMatchObject({ id: "a@x.com", usage: { n: 7 }, stale: true, active: true });
  });
});
