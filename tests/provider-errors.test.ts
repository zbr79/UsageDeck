import { describe, expect, it } from "vitest";
import { cardErrorKind, errorHint, isAuthRejected } from "@/lib/errors";
import type { ProviderErrorKind, TrackedAccount } from "@/lib/types";
import { slotFromAccount } from "@/widget/_lib/useWidgetData";

describe("errorHint", () => {
  it.each(["rate-limited", "network", "http", "locked", "schema-changed"] as ProviderErrorKind[])(
    "shows nothing for the transient kind %s",
    (kind) => {
      expect(errorHint(kind, "open Cursor")).toBeNull();
    },
  );

  it("shows a hint for the actionable missing-credentials kind", () => {
    const hint = errorHint("missing-credentials", "open Cursor");
    expect(hint?.short).toBe("not signed in");
    // the tooltip explains more than the header text
    expect(hint!.long.length).toBeGreaterThan(hint!.short.length);
  });

  it("keeps auth-rejected actionable through the re-auth flag", () => {
    expect(isAuthRejected(undefined, "auth-rejected")).toBe(true);
  });

  it("names a missing sign-in only for the account the app reads", () => {
    expect(cardErrorKind("missing-credentials", true)).toBe("missing-credentials");
    expect(cardErrorKind("missing-credentials", false)).toBeNull();
    expect(cardErrorKind("auth-rejected", false)).toBe("auth-rejected");
    expect(cardErrorKind(undefined, true)).toBeNull();
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
