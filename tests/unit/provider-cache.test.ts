import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { tempDir } from "../helpers/temp";
import { createProvider, type FetchOutcome } from "@/lib/provider-cache";
import { flush, readCached, resetUsageCacheForTests, writeCached } from "@/lib/usage-cache";

let cleanup: () => void;

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

describe("createProvider", () => {
  it("shares one in-flight fetch between concurrent callers", async () => {
    let resolveFetch!: (o: FetchOutcome<number>) => void;
    const fetch = vi.fn(() => new Promise<FetchOutcome<number>>((r) => (resolveFetch = r)));
    const p = createProvider<number>({ section: "t", ttlMs: 1000, fetch });
    const a = p.get();
    const b = p.get();
    resolveFetch({ ok: true, data: 42 });
    expect(await a).toEqual(await b);
    expect(fetch).toHaveBeenCalledTimes(1);
  });

  it("reuses a success for ttlMs, then refetches", async () => {
    vi.useFakeTimers();
    try {
      const fetch = vi.fn(async (): Promise<FetchOutcome<number>> => ({ ok: true, data: 1 }));
      const p = createProvider<number>({ section: "t", ttlMs: 1000, fetch });
      await p.get();
      await p.get();
      expect(fetch).toHaveBeenCalledTimes(1);
      vi.advanceTimersByTime(1001);
      await p.get();
      expect(fetch).toHaveBeenCalledTimes(2);
    } finally {
      vi.useRealTimers();
    }
  });

  it("falls back to last-known disk data, marked stale", async () => {
    await writeCached("t", "default", { n: 7 });
    const p = createProvider<{ n: number }>({
      section: "t",
      ttlMs: 1000,
      fetch: async () => ({ ok: false, authExpired: true }),
    });
    const r = await p.get();
    expect(r.usage).toEqual({ n: 7 });
    expect(r.stale).toBe(true);
    expect(r.cachedAt).toBeTypeOf("number");
    expect(r.authExpired).toBe(true);
  });

  it("uses the fallback (unpersisted) when there is no cache", async () => {
    const p = createProvider<{ n: number }>({
      section: "t",
      ttlMs: 1000,
      fetch: async () => ({ ok: false, fallback: { n: 0 } }),
    });
    const r = await p.get();
    expect(r).toEqual({ usage: { n: 0 }, stale: false, cachedAt: null, authExpired: false });
    expect(await readCached("t", "default")).toBeNull();
  });

  it("treats a thrown fetch as a failure, not a crash", async () => {
    const p = createProvider<number>({
      section: "t",
      ttlMs: 1000,
      fetch: async () => {
        throw new Error("network down");
      },
    });
    expect(await p.get()).toEqual({
      usage: null,
      stale: false,
      cachedAt: null,
      authExpired: false,
    });
  });

  it("persists successes so a fresh process can read them", async () => {
    const p = createProvider<number>({
      section: "t",
      ttlMs: 1000,
      fetch: async () => ({ ok: true, data: 5 }),
    });
    await p.get("acct");
    await flush();
    resetUsageCacheForTests();
    expect((await readCached<number>("t", "acct"))?.data).toBe(5);
  });
});
