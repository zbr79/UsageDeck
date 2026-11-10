import { describe, expect, it } from "vitest";
import { accountLabels, accountNote, providerLabels, providerStatus, resetParts } from "./account-labels";
import type { AccountWindow } from "./types";

describe("account labels", () => {
  it("uses the nickname as the title and the email as the subtitle", () => {
    expect(accountLabels("zhibinren79@gmail.com", "Main")).toEqual({
      title: "Main",
      subtitle: "zhibinren79@gmail.com",
    });
  });

  it("shows only the email when there is no nickname", () => {
    expect(accountLabels("zhibinren79@gmail.com", "  ")).toEqual({
      title: "zhibinren79@gmail.com",
      subtitle: null,
    });
  });

  it("names a provider from the nickname, then the account, then the provider", () => {
    expect(providerLabels("Cursor", "dev@cursor.com", "crsr-pro").title).toBe("crsr-pro");
    expect(providerLabels("Cursor", "dev@cursor.com", "").title).toBe("dev@cursor.com");
    expect(providerLabels("Cursor", null, "").title).toBe("Cursor");
  });

  it("writes week and month time remaining, and marks a full window", () => {
    const now = Date.parse("2026-10-15T00:00:00.000Z");
    const weekly: AccountWindow = {
      resetsAt: "2026-10-18T00:00:00.000Z",
      percent: 40,
      exhausted: false,
    };
    const monthly: AccountWindow = {
      resetsAt: "2026-11-02T00:00:00.000Z",
      percent: 100,
      exhausted: true,
    };
    expect(resetParts(weekly, monthly, now)).toEqual([
      { text: "Week 3d", full: false },
      { text: "Month full, 18d", full: true },
    ]);
  });

  it("keeps hidden, cached, and inactive as separate facts", () => {
    expect(accountNote(true, true, false)).toBe("Hidden · Cached");
    expect(accountNote(false, false, true)).toBe("Inactive");
    expect(accountNote(false, true, true)).toBe("Cached");
    expect(accountNote(false, false, false)).toBeNull();
  });

  it("keeps one status and treats a missing sign-in as inactive", () => {
    expect(providerStatus("dev@cursor.com", "Pro")).toBe("Pro");
    expect(providerStatus(null, "Pro")).toBe("Inactive");
    expect(providerStatus("dev@cursor.com", "not signed in")).toBe("Inactive");
  });
});