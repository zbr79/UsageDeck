import { describe, expect, it } from "vitest";
import { errorHint, isAuthRejected } from "@/lib/errors";
import { staleAge } from "@/lib/format";
import type { ProviderErrorKind, TrackedAccount } from "@/lib/types";
import { slotFromAccount } from "@/widget/_lib/useWidgetData";

const H = 3_600_000;

describe("errorHint", () => {
  it.each([
    ["rate-limited", "rate limited · retrying"],
    ["schema-changed", "reply format changed"],
    ["network", "offline"],
    ["http", "service error"],
    ["locked", "busy"],
    ["missing-credentials", "not signed in"],
  ] as [ProviderErrorKind, string][])("%s → %s", (kind, short) => {
    const hint = errorHint(kind, "open Cursor");
    expect(hint?.short).toBe(short);
    // the tooltip explains more than the header text
    expect(hint!.long.length).toBeGreaterThan(short.length);
  });

  it("names how to sign in for missing credentials", () => {
    expect(errorHint("missing-credentials", "open Cursor")?.long).toContain("open Cursor");
  });

  it("leaves auth-rejected to the re-auth hint, and no error to nothing", () => {
    expect(errorHint("auth-rejected")).toBeNull();
    expect(errorHint(null)).toBeNull();
    expect(errorHint(undefined)).toBeNull();
  });

  it("treats the old flag or the new kind as a rejected sign-in", () => {
    expect(isAuthRejected(true, null)).toBe(true);
    expect(isAuthRejected(false, "auth-rejected")).toBe(true);
    expect(isAuthRejected(undefined, "network")).toBe(false);
  });
});

describe("slotFromAccount error", () => {
  const base: TrackedAccount<number> = {
    id: "a",
    label: null,
    active: true,
    usage: 1,
    stale: true,
    cachedAt: 5,
    authExpired: false,
  };

  it("carries the error kind into the slot", () => {
    expect(slotFromAccount({ ...base, error: "rate-limited" }).error).toBe("rate-limited");
  });

  it("treats a missing field (older backend) as null", () => {
    expect(slotFromAccount(base).error).toBeNull();
    expect(slotFromAccount(null).error).toBeNull();
  });
});

describe("staleAge", () => {
  it("shows the age only for stale cached data", () => {
    const now = 10 * H;
    expect(staleAge(true, now - 2 * H, now)).toBe("updated 2h ago");
    expect(staleAge(false, now - 2 * H, now)).toBeNull();
    expect(staleAge(true, null, now)).toBeNull();
  });
});
