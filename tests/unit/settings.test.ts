import { readFileSync } from "node:fs";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { tempDir } from "../helpers/temp";
import {
  applySettingsPatch,
  DEFAULT_SETTINGS,
  moveItem,
  normalizeProviderOrder,
  normalizeSettings,
} from "@/lib/settings-schema";
import { forgetAccount, getSettings, updateSettings } from "@/lib/settings";

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

  it("normalises garbage from disk to defaults", () => {
    expect(normalizeSettings(null)).toEqual(DEFAULT_SETTINGS);
    expect(normalizeSettings([1, 2])).toEqual(DEFAULT_SETTINGS);
  });
});

describe("updateSettings", () => {
  let dir: string;
  let cleanup: () => void;
  beforeEach(() => {
    ({ dir, cleanup } = tempDir());
    vi.stubEnv("USAGE_DATA_DIR", dir);
  });
  afterEach(() => cleanup());

  it("never loses an update when saves race", async () => {
    await Promise.all([
      updateSettings((s) => applySettingsPatch(s, { themeMode: "dark" })),
      updateSettings((s) => applySettingsPatch(s, { showProviderNames: true })),
      updateSettings((s) => applySettingsPatch(s, { displayNames: { a: "A" } })),
      updateSettings((s) => applySettingsPatch(s, { displayNames: { b: "B" } })),
    ]);
    const s = await getSettings();
    expect(s.themeMode).toBe("dark");
    expect(s.showProviderNames).toBe(true);
    expect(s.displayNames).toEqual({ a: "A", b: "B" });
    expect(JSON.parse(readFileSync(path.join(dir, "settings.json"), "utf8")).themeMode).toBe(
      "dark",
    );
  });

  it("forgetAccount drops the nickname and disabled flag", async () => {
    await updateSettings((s) =>
      applySettingsPatch(s, {
        displayNames: { "Me@x.com": "Me", cursor: "C" },
        disabledAccounts: ["me@x.com", "other@x.com"],
      }),
    );
    const s = await forgetAccount("me@x.com");
    expect(s.displayNames).toEqual({ cursor: "C" });
    expect(s.disabledAccounts).toEqual(["other@x.com"]);
  });
});
