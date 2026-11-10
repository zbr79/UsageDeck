import { describe, expect, it } from "vitest";
import { initialSettingsState, settingsReducer } from "./settings-reducer";

describe("settingsReducer", () => {
  it("starts from the same defaults the settings screen used before", () => {
    expect(initialSettingsState.tab).toBe("opencode");
    expect(initialSettingsState.themeMode).toBe("auto");
    expect(initialSettingsState.panelTheme).toBe("dark");
    expect(initialSettingsState.accounts).toBeNull();
  });

  it("applies a loaded snapshot in one update", () => {
    const next = settingsReducer(initialSettingsState, {
      type: "loaded",
      patch: {
        accounts: [{ email: "a@example.com" }],
        themeMode: "dark",
        cursorEnabled: false,
        disabledAccounts: ["a@example.com"],
      },
    });
    expect(next.accounts).toEqual([{ email: "a@example.com" }]);
    expect(next.themeMode).toBe("dark");
    expect(next.cursorEnabled).toBe(false);
    expect(next.tab).toBe("opencode");
  });

  it("clears the saved flash only for the key that just saved", () => {
    const flashing = { ...initialSettingsState, savedFlash: "cursor" };
    expect(settingsReducer(flashing, { type: "clearSavedFlash", key: "grok" }).savedFlash).toBe(
      "cursor",
    );
    expect(settingsReducer(flashing, { type: "clearSavedFlash", key: "cursor" }).savedFlash).toBe(
      null,
    );
  });
});
