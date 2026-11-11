import { describe, expect, it } from "vitest";
import appIcon from "../src-tauri/icons/128x128@2x.png?inline";
import installerIcon from "../installer/ui/icon.png?inline";

// The installer window's frontend is a static folder (installer/tauri.conf.json frontendDist), so
// it carries its own copy of the app icon.
describe("installer assets", () => {
  it("shows the app's own icon", () => {
    expect(installerIcon.startsWith("data:image/png;base64,")).toBe(true);
    expect(installerIcon).toBe(appIcon);
  });
});
