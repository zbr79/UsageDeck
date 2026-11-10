import { describe, expect, it } from "vitest";
import type { ClaudeUsage, TrackedAccount } from "@/lib/types";
import { slotFromAccount } from "@/widget/_lib/useWidgetData";

describe("slotFromAccount", () => {
  it("keeps the cached usage of an account that is not signed in", () => {
    const usage = {
      email: "old@example.com",
      signedIn: true,
      fiveHour: { percent: 12, resetsAt: "2026-01-01T00:00:00Z" },
      weekly: { percent: 70, resetsAt: "2026-01-05T00:00:00Z" },
    } as ClaudeUsage;
    const account: TrackedAccount<ClaudeUsage> = {
      id: "old@example.com",
      label: "old@example.com",
      active: false,
      usage,
      stale: true,
      cachedAt: 1_700_000_000_000,
      authExpired: false,
    };
    expect(slotFromAccount(account)).toEqual({
      data: usage,
      stale: true,
      cachedAt: 1_700_000_000_000,
      authExpired: false,
      error: null,
      loaded: true,
    });
  });

  it("gives an empty slot when there is no account", () => {
    expect(slotFromAccount(null, false)).toEqual({
      data: null,
      stale: false,
      cachedAt: null,
      authExpired: false,
      error: null,
      loaded: false,
    });
  });
});
