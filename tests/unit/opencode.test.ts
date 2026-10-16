import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { jsonResponse, tempDir } from "../helpers/temp";

const w = (percent: number) => ({ status: "ok", percent, resetsAt: "2030-01-01T00:00:00Z" });
const usage = (rolling: number) => ({
  usage: { rolling: w(rolling), weekly: w(1), monthly: w(2) },
});

let cleanup: () => void;
beforeEach(() => {
  const t = tempDir();
  cleanup = t.cleanup;
  vi.stubEnv("USAGE_DATA_DIR", t.dir);
  vi.resetModules();
});
afterEach(() => {
  vi.useRealTimers();
  cleanup();
});

describe("OpenCode getUsage", () => {
  it("keeps the previous snapshot stable across cache hits", async () => {
    vi.useFakeTimers();
    const responses = [usage(1), usage(5)];
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => jsonResponse(responses.shift())),
    );
    const { getUsage, getLastChange } = await import("@/lib/opencode");

    const first = await getUsage("sk-k", "a@x.com");
    expect(first.usage?.rolling.percent).toBe(1);
    expect(first.prev).toBeNull();

    vi.advanceTimersByTime(11_000);
    const second = await getUsage("sk-k", "a@x.com");
    expect(second.usage?.rolling.percent).toBe(5);
    expect(second.prev?.rolling.percent).toBe(1);
    expect(getLastChange("a@x.com")?.detail).toBe("rolling 1% → 5%");

    // Within the TTL the widget sees the same before/after pair, so its ▲ survives.
    const third = await getUsage("sk-k", "a@x.com");
    expect(third.prev?.rolling.percent).toBe(1);
  });

  it("flags an inactive subscription", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => jsonResponse({ error: { type: "EntitlementError" } }, 403)),
    );
    const { getUsage } = await import("@/lib/opencode");
    const r = await getUsage("sk-k", "b@x.com");
    expect(r.subscriptionActive).toBe(false);
    expect(r.usage).toBeNull();
  });

  it("encrypts plaintext keys on read and hands callers the plaintext", async () => {
    const fs = await import("node:fs/promises");
    const { dataFile } = await import("@/lib/paths");
    await fs.mkdir(process.env.USAGE_DATA_DIR!, { recursive: true });
    await fs.writeFile(
      dataFile("accounts.json"),
      JSON.stringify([{ email: "a@x.com", key: "sk-secret" }]),
    );
    const { getAccounts, keyHint, updateAccounts } = await import("@/lib/opencode");

    const accounts = await getAccounts();
    expect(accounts).toEqual([{ email: "a@x.com", key: "sk-secret" }]);
    expect(keyHint(accounts[0].key)).toBe("sk-…cret");

    const sealed = JSON.parse(await fs.readFile(dataFile("accounts.json"), "utf8")) as {
      key: string;
    }[];
    expect(sealed[0].key.startsWith("enc:v1:")).toBe(true);
    expect(sealed[0].key.includes("sk-secret")).toBe(false);

    expect(await getAccounts()).toEqual([{ email: "a@x.com", key: "sk-secret" }]);
    await updateAccounts((list) => [...list, { email: "b@x.com", key: "sk-otherkey" }]);
    const after = await getAccounts();
    expect(after.map((a) => a.key)).toEqual(["sk-secret", "sk-otherkey"]);
    const sealedAgain = JSON.parse(await fs.readFile(dataFile("accounts.json"), "utf8")) as {
      key: string;
    }[];
    expect(sealedAgain.every((a) => a.key.startsWith("enc:v1:"))).toBe(true);
  });

  it("refetches immediately when the key changes", async () => {
    const fetch = vi.fn(async () => jsonResponse(usage(1)));
    vi.stubGlobal("fetch", fetch);
    const { getUsage } = await import("@/lib/opencode");
    await getUsage("sk-old", "c@x.com");
    await getUsage("sk-new", "c@x.com");
    expect(fetch).toHaveBeenCalledTimes(2);
  });
});
