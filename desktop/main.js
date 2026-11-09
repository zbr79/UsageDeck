const { app, BrowserWindow, Menu, ipcMain, screen, desktopCapturer, dialog } = require("electron");
const { spawn } = require("child_process");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const APP_ROOT = path.resolve(__dirname, "..");
const WIDGET_URL = "http://127.0.0.1:3100/widget";
const WIDTH = 238;
const CONFIG_PATH = path.join(app.getPath("userData"), "widget-config.json");
const UPDATE_DELAY_MS = 10 * 1000;
const UPDATE_INTERVAL_MS = 6 * 60 * 60 * 1000;

function createDevMode() {
  let child = null;
  return {
    dataDir: path.join(APP_ROOT, "data"),
    iconPath: path.join(APP_ROOT, "app-icon.ico"),
    startServer() {
      child = spawn("cmd.exe", ["/c", "npm run start"], {
        cwd: APP_ROOT,
        stdio: "ignore",
        windowsHide: true,
      });
    },
    stopServer() {
      const pid = child?.pid;
      child = null;
      if (!pid) return;
      try {
        spawn("taskkill", ["/pid", String(pid), "/f", "/t"], {
          windowsHide: true,
          stdio: "ignore",
        });
      } catch {
        // ignore
      }
    },
  };
}

function createPackagedMode() {
  const dataDir = path.join(app.getPath("userData"), "data");
  let child = null;
  return {
    dataDir,
    iconPath: path.join(process.resourcesPath, "app-icon.ico"),
    startServer() {
      child = spawn(process.execPath, [path.join(process.resourcesPath, "server", "server.js")], {
        env: {
          ...process.env,
          ELECTRON_RUN_AS_NODE: "1",
          PORT: "3100",
          HOSTNAME: "127.0.0.1",
          USAGEDECK_DATA_DIR: dataDir,
        },
        stdio: "ignore",
        windowsHide: true,
      });
    },
    stopServer() {
      child?.kill();
      child = null;
    },
  };
}

const mode = app.isPackaged ? createPackagedMode() : createDevMode();
const SECRET_PATH = path.join(mode.dataDir, ".api-secret");

let win = null;
let settingsWin = null;
let dragOffset = null;
let spawnedServer = false;
let usageSecret = "";
let updater = null;
let updateCheckInFlight = false;

function loadPos() {
  try {
    const c = JSON.parse(fs.readFileSync(CONFIG_PATH, "utf8"));
    if (typeof c.x !== "number" || typeof c.y !== "number") return null;
    const visible = screen.getAllDisplays().some((d) => {
      const a = d.workArea;
      return (
        c.x + 40 < a.x + a.width &&
        c.x + WIDTH - 40 > a.x &&
        c.y + 40 < a.y + a.height &&
        c.y + 40 > a.y
      );
    });
    return visible ? { x: c.x, y: c.y } : null;
  } catch {
    return null;
  }
}

function savePos() {
  if (!win) return;
  const b = win.getBounds();
  try {
    fs.mkdirSync(path.dirname(CONFIG_PATH), { recursive: true });
    fs.writeFileSync(CONFIG_PATH, JSON.stringify({ x: b.x, y: b.y, width: b.width, height: b.height }));
  } catch {
    // ignore
  }
}

function ensureSecret() {
  try {
    fs.mkdirSync(path.dirname(SECRET_PATH), { recursive: true });
    if (fs.existsSync(SECRET_PATH)) {
      const existing = fs.readFileSync(SECRET_PATH, "utf8").trim();
      if (existing) {
        usageSecret = existing;
        return;
      }
    }
    usageSecret = crypto.randomBytes(32).toString("hex");
    fs.writeFileSync(SECRET_PATH, usageSecret, { encoding: "utf8", mode: 0o600 });
  } catch {
    usageSecret = crypto.randomBytes(32).toString("hex");
  }
}

async function serverIsUp() {
  try {
    const res = await fetch("http://127.0.0.1:3100/widget", {
      signal: AbortSignal.timeout(2000),
    });
    return res.ok;
  } catch {
    return false;
  }
}

function startServer() {
  mode.startServer();
  spawnedServer = true;
}

function stopServer() {
  if (!spawnedServer) return;
  spawnedServer = false;
  mode.stopServer();
}

async function waitForServer(timeoutMs = 60000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await serverIsUp()) return true;
    await new Promise((r) => setTimeout(r, 1000));
  }
  return false;
}

function createWindow() {
  const pos = loadPos();
  win = new BrowserWindow({
    x: pos?.x,
    y: pos?.y,
    width: WIDTH,
    height: 150,
    frame: false,
    transparent: true,
    resizable: true,
    alwaysOnTop: true,
    skipTaskbar: true,
    hasShadow: false,
    backgroundColor: "#00000000",
    icon: mode.iconPath,
    webPreferences: {
      preload: path.join(__dirname, "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
    },
  });

  win.setMinimumSize(WIDTH, 150);
  win.setMaximumSize(WIDTH, 8000);

  win.setAlwaysOnTop(true, "screen-saver");
  win.setVisibleOnAllWorkspaces(true, { visibleOnFullScreen: true });
  win.loadURL(WIDGET_URL);

  let saveTimer = null;
  win.on("move", () => {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(savePos, 300);
  });

  win.webContents.on("context-menu", (e) => {
    e.preventDefault();
    showMenu();
  });

  win.on("closed", () => {
    win = null;
  });
}

function openSettingsOverlay() {
  if (settingsWin && !settingsWin.isDestroyed()) {
    settingsWin.focus();
    return;
  }
  const disp = win && !win.isDestroyed() ? screen.getDisplayMatching(win.getBounds()) : screen.getPrimaryDisplay();
  const wa = disp.workArea;
  settingsWin = new BrowserWindow({
    x: wa.x,
    y: wa.y,
    width: wa.width,
    height: wa.height,
    frame: false,
    transparent: true,
    alwaysOnTop: true,
    skipTaskbar: true,
    hasShadow: false,
    backgroundColor: "#00000000",
    webPreferences: {
      preload: path.join(__dirname, "settings-preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
    },
  });
  settingsWin.setAlwaysOnTop(true, "screen-saver");
  settingsWin.loadURL("http://127.0.0.1:3100/settings");
  settingsWin.on("closed", () => {
    settingsWin = null;
    if (win && !win.isDestroyed()) win.webContents.send("widget-settings-changed");
  });
}

function showMenu() {
  if (!win) return;
  Menu.buildFromTemplate([
    {
      label: "Toggle Expand / Collapse",
      click: () => win.webContents.send("widget-toggle-expand"),
    },
    { label: "Refresh", click: () => win.reload() },
    {
      label: "Settings",
      click: () => openSettingsOverlay(),
    },
    { type: "separator" },
    { label: `UsageDeck v${app.getVersion()}`, enabled: false },
    updater
      ? { label: "Check for Updates…", click: () => checkForUpdatesManually() }
      : { label: "Updates are only available in the installed app", enabled: false },
    { type: "separator" },
    { label: "Quit", click: () => app.quit() },
  ]).popup({ window: win });
}

function showMessage(options) {
  const parent = win && !win.isDestroyed() ? win : undefined;
  return dialog.showMessageBox(parent, { type: "info", title: "UsageDeck", ...options });
}

async function checkForUpdatesManually() {
  if (updateCheckInFlight) return;
  updateCheckInFlight = true;
  try {
    const result = await updater.checkForUpdates();
    if (result?.isUpdateAvailable) {
      await showMessage({
        message: `Version ${result.updateInfo.version} is available.`,
        detail: "It is downloading now. You will be asked to restart when it is ready.",
      });
    } else {
      await showMessage({ message: "UsageDeck is up to date.", detail: `Version ${app.getVersion()}` });
    }
  } catch (err) {
    await showMessage({
      type: "error",
      message: "Could not check for updates.",
      detail: String(err?.message ?? err),
    });
  } finally {
    updateCheckInFlight = false;
  }
}

function setupUpdates() {
  const { autoUpdater } = require("electron-updater");
  autoUpdater.autoDownload = true;
  autoUpdater.on("update-downloaded", async (info) => {
    const { response } = await showMessage({
      message: `UsageDeck ${info.version} is ready to install.`,
      detail: "Restart now to finish updating.",
      buttons: ["Restart now", "Later"],
      defaultId: 0,
      cancelId: 1,
    });
    if (response === 0) {
      stopServer();
      autoUpdater.quitAndInstall();
    }
  });
  const check = () => autoUpdater.checkForUpdates().catch(() => {});
  setTimeout(check, UPDATE_DELAY_MS);
  setInterval(check, UPDATE_INTERVAL_MS);
  return autoUpdater;
}

ipcMain.on("settings-close", () => {
  if (settingsWin && !settingsWin.isDestroyed()) settingsWin.close();
});
ipcMain.on("get-usage-secret", (e) => {
  e.returnValue = usageSecret;
});

ipcMain.on("widget-close", () => app.quit());
ipcMain.on("widget-minimize", () => {
  if (win) win.minimize();
});
ipcMain.on("widget-show-menu", () => showMenu());
ipcMain.on("widget-drag-start", (_e, sx, sy) => {
  if (!win) return;
  const [x, y] = win.getPosition();
  dragOffset = { dx: sx - x, dy: sy - y };
});
ipcMain.on("widget-drag-move", (_e, sx, sy) => {
  if (!win || !dragOffset) return;
  win.setPosition(Math.round(sx - dragOffset.dx), Math.round(sy - dragOffset.dy));
});
ipcMain.on("widget-drag-end", () => {
  dragOffset = null;
  savePos();
});
ipcMain.on("widget-resize", (_e, width, height) => {
  if (!win) return;
  const disp = screen.getDisplayMatching(win.getBounds());
  const maxH = disp.workArea.height - 8;
  const h = Math.max(100, Math.min(Math.round(height), maxH));
  win.setSize(Math.round(width), h);
});

function startBgSampler() {
  const SETTINGS_FILE = path.join(mode.dataDir, "settings.json");
  async function loop() {
    while (true) {
      let theme = "auto";
      try {
        const s = JSON.parse(fs.readFileSync(SETTINGS_FILE, "utf8"));
        if (s?.themeMode === "light" || s?.themeMode === "dark") theme = s.themeMode;
      } catch {
        // no settings file yet → auto
      }
      if (theme === "auto") {
        try {
          if (win && !win.isDestroyed()) {
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
        } catch {
          // ignore sampling errors
        }
        await new Promise((r) => setTimeout(r, 3000));
      } else {
        await new Promise((r) => setTimeout(r, 5000));
      }
    }
  }
  loop();
}

app.whenReady().then(async () => {
  ensureSecret();
  if (!(await serverIsUp())) {
    startServer();
  }
  const ok = await waitForServer();
  if (!usageSecret) ensureSecret();
  if (ok) {
    if (app.isPackaged) updater = setupUpdates();
    createWindow();
    startBgSampler();
  } else {
    console.error("Server did not start in time");
    app.quit();
  }
});

app.on("before-quit", () => stopServer());
app.on("window-all-closed", () => app.quit());