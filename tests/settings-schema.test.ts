// Moved from tests/unit/settings.test.ts (the pure applySettingsPatch part).
import { describe, expect, it } from "vitest";
import {
  applySettingsPatch,
  DEFAULT_SETTINGS,
  moveItem,
  normalizeProviderOrder,
  normalizeSettings,
} from "@/lib/settings-schema";
import { resolvePanelTheme } from "@/settings/_lib/theme";

describe("applySettingsPatch", () => {
  it("applies only valid, known fields", () => {
    const next = applySettingsPatch(DEFAULT_SETTINGS, {
      cursorEnabled: false,
      codexEnabled: "no",
      themeMode: "neon",
      panelTheme: "light",
      evil: true,
    });
    expect(next.cursorEnabled).toBe(false);
    expect(next.opencodeEnabled).toBe(true);
    expect(next.codexEnabled).toBe(true);
    expect(next.themeMode).toBe("auto");
    expect(next.panelTheme).toBe("light");
    expect(next).not.toHaveProperty("evil");
  });

  it("merges nicknames; an empty name removes one", () => {
    const current = { ...DEFAULT_SETTINGS, displayNames: { a: "Alpha", b: "Beta" } };
    const next = applySettingsPatch(current, { displayNames: { b: "  ", c: "  Gamma  " } });
    expect(next.displayNames).toEqual({ a: "Alpha", c: "Gamma" });
  });

  it("accepts the system settings theme and keeps the old values valid", () => {
    const system = applySettingsPatch(DEFAULT_SETTINGS, { panelTheme: "system" });
    expect(system.panelTheme).toBe("system");
    expect(applySettingsPatch(system, { panelTheme: "light" }).panelTheme).toBe("light");
    expect(applySettingsPatch(system, { panelTheme: "dark" }).panelTheme).toBe("dark");
    expect(applySettingsPatch(system, { panelTheme: "auto" }).panelTheme).toBe("system");
    expect(normalizeSettings({ panelTheme: "system" }).panelTheme).toBe("system");
    expect(normalizeSettings({}).panelTheme).toBe("dark");
  });

  it("resolves the system settings theme from the Windows mode", () => {
    expect(resolvePanelTheme("system", true)).toBe("dark");
    expect(resolvePanelTheme("system", false)).toBe("light");
    expect(resolvePanelTheme("light", true)).toBe("light");
    expect(resolvePanelTheme("dark", false)).toBe("dark");
    expect(resolvePanelTheme(undefined, false)).toBe("dark");
  });

  it("migrates the removed white widget theme to light", () => {
    expect(applySettingsPatch(DEFAULT_SETTINGS, { themeMode: "white" }).themeMode).toBe("light");
  });

  it("caps nickname length and ignores non-strings and prototype keys", () => {
    const next = applySettingsPatch(DEFAULT_SETTINGS, {
      displayNames: JSON.parse('{"__proto__":"x","n":42,"long":"' + "x".repeat(100) + '"}'),
    });
    expect(next.displayNames).toEqual({ long: "x".repeat(64) });
    expect(({} as Record<string, unknown>).x).toBeUndefined();
  });

  it("lower-cases and de-duplicates disabled accounts", () => {
    const next = applySettingsPatch(DEFAULT_SETTINGS, {
      disabledAccounts: ["A@x.com", "a@x.com", 3, ""],
    });
    expect(next.disabledAccounts).toEqual(["a@x.com"]);
  });

  it("can hide OpenCode as a whole, and a missing flag stays on", () => {
    expect(applySettingsPatch(DEFAULT_SETTINGS, { opencodeEnabled: false }).opencodeEnabled).toBe(
      false,
    );
    expect(normalizeSettings({ cursorEnabled: false }).opencodeEnabled).toBe(true);
  });

  it("keeps a provider reorder and fills in anything missing", () => {
    expect(moveItem(["opencode", "cursor", "grok"], 2, 0)).toEqual(["grok", "opencode", "cursor"]);
    expect(normalizeProviderOrder(["claude", "cursor", "nope", "cursor", "opencode"])).toEqual([
      "claude",
      "cursor",
      "opencode",
      "grok",
      "codex",
    ]);
    expect(applySettingsPatch(DEFAULT_SETTINGS, { providerOrder: "nope" }).providerOrder).toEqual(
      DEFAULT_SETTINGS.providerOrder,
    );
  });

  it("starts with Windows by default and keeps a boolean change", () => {
    expect(DEFAULT_SETTINGS.startAtLogin).toBe(true);
    expect(normalizeSettings({}).startAtLogin).toBe(true);
    expect(applySettingsPatch(DEFAULT_SETTINGS, { startAtLogin: false }).startAtLogin).toBe(false);
    expect(applySettingsPatch(DEFAULT_SETTINGS, { startAtLogin: "no" }).startAtLogin).toBe(true);
  });

  it("normalises garbage from disk to defaults", () => {
    expect(normalizeSettings(null)).toEqual(DEFAULT_SETTINGS);
    expect(normalizeSettings([1, 2])).toEqual(DEFAULT_SETTINGS);
  });
});

describe("claudeHourlyTarget", () => {
  const patch = (v: unknown, from: number | null = 50) =>
    applySettingsPatch({ ...DEFAULT_SETTINGS, claudeHourlyTarget: from }, { claudeHourlyTarget: v })
      .claudeHourlyTarget;

  it("is off by default", () => {
    expect(DEFAULT_SETTINGS.claudeHourlyTarget).toBeNull();
    expect(normalizeSettings({}).claudeHourlyTarget).toBeNull();
  });
  it("takes a number from 1 to 10000, rounded to a whole dollar", () => {
    expect(patch(100)).toBe(100);
    expect(patch(1)).toBe(1);
    expect(patch(10000)).toBe(10000);
    expect(patch(99.6)).toBe(100);
    expect(patch(12.4)).toBe(12);
  });
  it("null clears it", () => {
    expect(patch(null)).toBeNull();
  });
  it("ignores anything else", () => {
    for (const v of [0, 0.5, -5, 10001, NaN, Infinity, "100", true, {}, []]) {
      expect(patch(v)).toBe(50);
    }
    expect(applySettingsPatch(DEFAULT_SETTINGS, {}).claudeHourlyTarget).toBeNull();
  });
});
