const fs = require("fs");
const { screen, desktopCapturer } = require("electron");

function startBgSampler({ settingsFile, log, getWindow, shouldStop }) {
  let settingsCache = { mtimeMs: -1, mode: "auto" };

  function readThemeMode() {
    try {
      const stat = fs.statSync(settingsFile);
      if (stat.mtimeMs === settingsCache.mtimeMs) return settingsCache.mode;
      const saved = JSON.parse(fs.readFileSync(settingsFile, "utf8"));
      const mode =
        saved?.themeMode === "light" || saved?.themeMode === "dark" || saved?.themeMode === "white"
          ? saved.themeMode
          : "auto";
      settingsCache = { mtimeMs: stat.mtimeMs, mode };
      return mode;
    } catch {
      // Expected when settings.json has not been saved yet; default to auto
      return "auto";
    }
  }

  async function loop() {
    while (!shouldStop()) {
      const mode = readThemeMode();
      if (mode === "auto") {
        try {
          const win = getWindow();
          if (win && !win.isDestroyed() && !win.isMinimized() && win.isVisible()) {
            const display = screen.getPrimaryDisplay();
            const sources = await desktopCapturer.getSources({
              types: ["screen"],
              thumbnailSize: { width: 160, height: 90 },
            });
            const src = sources.find((s) => s.display_id === String(display.id)) ?? sources[0];
            if (src) {
              const buffer = src.thumbnail.toBitmap();
              let sum = 0;
              let count = 0;
              for (let i = 0; i + 3 < buffer.length; i += 12) {
                sum += buffer[i] + buffer[i + 1] + buffer[i + 2];
                count += 3;
              }
              const avg = count ? sum / count / 255 : 0.5;
              win.webContents.send("widget-bg", avg);
            }
          }
        } catch (err) {
          log("WARN", `Screen brightness sample failed: ${err?.message || err}`);
        }
        await new Promise((r) => setTimeout(r, 3000));
      } else {
        await new Promise((r) => setTimeout(r, 5000));
      }
    }
  }

  loop();
}

module.exports = { startBgSampler };
