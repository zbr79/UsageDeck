import { describe, expect, it } from "vitest";
import type { UpdatePhase, UpdateStatus } from "@/lib/updates";
import { bannerText } from "@/widget/_components/UpdateBanner";

const status = (phase: UpdatePhase): UpdateStatus => ({
  ...phase,
  lastChecked: null,
  currentVersion: "1.4.0",
});

describe("widget update banner", () => {
  it("shows nothing when idle or before the first status", () => {
    expect(bannerText(null)).toBeNull();
    expect(bannerText(status({ state: "idle" }))).toBeNull();
  });

  it("offers a ready update until the user puts it off", () => {
    const ready = status({ state: "ready", version: "1.5.0", dismissed: false });
    expect(bannerText(ready)).toBe("Update 1.5.0 is ready");
    expect(bannerText(ready, true)).toBe("Restarting…");
    expect(bannerText(status({ state: "ready", version: "1.5.0", dismissed: true }))).toBeNull();
  });

  it("follows a manual check through to its result", () => {
    expect(bannerText(status({ state: "checking" }))).toBe("Checking for updates…");
    expect(bannerText(status({ state: "downloading", version: "1.5.0" }))).toBe(
      "Downloading 1.5.0…",
    );
    expect(bannerText(status({ state: "upToDate" }))).toBe("You're up to date · v1.4.0");
    expect(bannerText(status({ state: "failed", message: "network error: offline" }))).toBe(
      "Couldn't check for updates",
    );
  });
});
