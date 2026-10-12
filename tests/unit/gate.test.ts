import { writeFileSync } from "node:fs";
import path from "node:path";
import { NextRequest } from "next/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { tempDir } from "../helpers/temp";
import { isAllowedHost } from "@/lib/host";
import { secretsMatch } from "@/lib/secret";

describe("isAllowedHost", () => {
  it.each([
    ["127.0.0.1:3100", true],
    ["localhost:3100", true],
    ["LOCALHOST", true],
    ["[::1]:3100", true],
    ["attacker.example:3100", false],
    ["127.0.0.1.attacker.example", false],
    ["localhost.evil.test:3100", false],
    ["", false],
    [null, false],
  ])("%s → %s", (host, allowed) => {
    expect(isAllowedHost(host)).toBe(allowed);
  });
});

describe("secretsMatch", () => {
  it("requires an exact match", () => {
    expect(secretsMatch("abc", "abc")).toBe(true);
    expect(secretsMatch("abd", "abc")).toBe(false);
    expect(secretsMatch("ab", "abc")).toBe(false);
    expect(secretsMatch(null, "abc")).toBe(false);
    expect(secretsMatch("", "abc")).toBe(false);
  });
});

describe("proxy gate", () => {
  const SECRET = "a".repeat(64);
  let cleanup: () => void;
  beforeEach(() => {
    const t = tempDir();
    cleanup = t.cleanup;
    writeFileSync(path.join(t.dir, ".api-secret"), SECRET);
    vi.stubEnv("USAGE_DATA_DIR", t.dir);
  });
  afterEach(() => cleanup());

  const request = (url: string, headers: Record<string, string> = {}) =>
    new NextRequest(url, { headers: { host: new URL(url).host, ...headers } });

  it("serves pages on loopback hosts", async () => {
    const { proxy } = await import("@/proxy");
    const res = await proxy(request("http://127.0.0.1:3100/widget"));
    expect(res.status).toBe(200);
  });

  it("rejects a DNS-rebound host, even with the secret", async () => {
    const { proxy } = await import("@/proxy");
    const res = await proxy(
      request("http://127.0.0.1:3100/api/usage", {
        host: "evil.test:3100",
        "x-usage-secret": SECRET,
      }),
    );
    expect(res.status).toBe(403);
  });

  it("requires the secret on every API route", async () => {
    const { proxy } = await import("@/proxy");
    for (const route of ["usage", "claude", "accounts", "settings", "key"]) {
      const res = await proxy(request(`http://127.0.0.1:3100/api/${route}`));
      expect(res.status, route).toBe(401);
    }
    const wrong = await proxy(
      request("http://127.0.0.1:3100/api/usage", { "x-usage-secret": "nope" }),
    );
    expect(wrong.status).toBe(401);
    const ok = await proxy(
      request("http://127.0.0.1:3100/api/usage", { "x-usage-secret": SECRET }),
    );
    expect(ok.status).toBe(200);
  });
});
