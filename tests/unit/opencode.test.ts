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

  it("refetches immediately when the key changes", async () => {
    const fetch = vi.fn(async () => jsonResponse(usage(1)));
    vi.stubGlobal("fetch", fetch);
    const { getUsage } = await import("@/lib/opencode");
    await getUsage("sk-old", "c@x.com");
    await getUsage("sk-new", "c@x.com");
    expect(fetch).toHaveBeenCalledTimes(2);
  });
});
