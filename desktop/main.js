const { app, BrowserWindow, Menu, ipcMain, screen, shell, dialog } = require("electron");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const { appendRotatedLog } = require("./restart-policy");
const { createServerManager } = require("./server-manager");
const { startBgSampler } = require("./bg-sampler");

const APP_ROOT = path.resolve(__dirname, "..");
const WIDGET_URL = "http://127.0.0.1:3100/widget";
const WIDTH = 238;
const CONFIG_PATH = path.join(app.getPath("userData"), "widget-config.json");
const LOG_DIR = path.join(app.getPath("userData"), "logs");
const LOG_PATH = path.join(LOG_DIR, "main.log");
const UPDATE_DELAY_MS = 10 * 1000;
const UPDATE_INTERVAL_MS = 6 * 60 * 60 * 1000;

function packagedMode() {
  const dataDir = path.join(app.getPath("userData"), "data");
  return {
    dataDir,
    iconPath: path.join(process.resourcesPath, "app-icon.ico"),
    autoUpdate: true,
    launch: {
      kind: "packaged",
      execPath: process.execPath,
      serverEntry: path.join(process.resourcesPath, "server", "server.js"),
      dataDir,
    },
  };
}

function devMode() {
  return {
    dataDir: path.join(APP_ROOT, "data"),
    iconPath: path.join(APP_ROOT, "app-icon.ico"),
    autoUpdate: false,
    launch: { kind: "dev", appRoot: APP_ROOT },
  };
}

const mode = app.isPackaged ? packagedMode() : devMode();
const SECRET_PATH = path.join(mode.dataDir, ".api-secret");
const SETTINGS_FILE = path.join(mode.dataDir, "settings.json");

let win = null;
let settingsWin = null;
let dragOffset = null;
let usageSecret = "";
let updater = null;
let updateCheckInFlight = false;

function writeLog(level, message) {
  const line = `${new Date().toISOString()} [${level}] ${message}`;
  appendRotatedLog(LOG_PATH, line);
}

const server = createServerManager({
  launch: mode.launch,
  widgetUrl: WIDGET_URL,
  log: writeLog,
  getWindow: () => win,
});

process.on("uncaughtException", (err) => {
  writeLog("ERROR", `uncaughtException: ${err?.stack || err}`);
});

process.on("unhandledRejection", (reason) => {
  writeLog("ERROR", `unhandledRejection: ${reason?.stack || reason}`);
});

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
    // Expected when widget-config.json has not been created yet
    return null;
  }
}

function savePos() {
  if (!win) return;
  const b = win.getBounds();
  try {
    fs.mkdirSync(path.dirname(CONFIG_PATH), { recursive: true });
    fs.writeFileSync(CONFIG_PATH, JSON.stringify({ x: b.x, y: b.y, width: b.width, height: b.height }));
  } catch (err) {
    writeLog("WARN", `Failed to save window position: ${err?.message || err}`);
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
  } catch (err) {
    writeLog("WARN", `Failed to persist API secret, using in-memory fallback: ${err?.message || err}`);
    usageSecret = crypto.randomBytes(32).toString("hex");
  }
}

function showStartupError(message) {
  writeLog("ERROR", message);
  const errorWin = new BrowserWindow({
    width: 460,
    height: 240,
    title: "Provider Usage Monitor",
    alwaysOnTop: true,
    skipTaskbar: false,
    autoHideMenuBar: true,
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
    },
  });
  const safe = String(message)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
  const html = `<!doctype html><html><head><meta charset="utf-8"><title>Provider Usage Monitor</title></head><body style="margin:0;font-family:Segoe UI,sans-serif;background:#18181b;color:#f4f4f5"><main style="padding:24px"><h1 style="font-size:16px;margin:0 0 12px">The widget did not open</h1><p style="font-size:13px;line-height:1.5;margin:0">${safe}</p></main></body></html>`;
  errorWin.loadURL(`data:text/html;charset=utf-8,${encodeURIComponent(html)}`);
}

async function recoverWindow(reason) {
  if (server.isQuitting() || !win || win.isDestroyed()) return;
  writeLog("WARN", `Recovering widget window after ${reason}`);
  if (!(await server.isUp())) {
    if (!server.hasChild() && !server.hasRestartTimer()) {
      server.scheduleRestart(reason);
      return;
    }
    const ok = await server.waitFor(20000);
    if (!ok) return;
  }
  if (win && !win.isDestroyed()) {
    win.loadURL(WIDGET_URL);
  }
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
    skipTaskbar: false,
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

  win.webContents.on("render-process-gone", (_e, details) => {
    writeLog("ERROR", `Renderer process gone: reason=${details?.reason} exitCode=${details?.exitCode}`);
    setTimeout(() => recoverWindow(`render-process-gone (${details?.reason})`), 1000);
  });

  win.webContents.on("did-fail-load", (_e, errorCode, errorDescription, validatedURL) => {
    if (errorCode === -3) return; // ERR_ABORTED during reload
    writeLog("WARN", `did-fail-load: code=${errorCode} (${errorDescription}) url=${validatedURL}`);
    setTimeout(() => recoverWindow(`did-fail-load (${errorCode})`), 1500);
  });

  win.on("unresponsive", () => {
    writeLog("WARN", "Widget window became unresponsive");
    setTimeout(() => recoverWindow("window-unresponsive"), 2000);
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
    skipTaskbar: false,
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

function openLogFolder() {
  try {
    fs.mkdirSync(LOG_DIR, { recursive: true });
    shell.openPath(LOG_DIR);
  } catch (err) {
    writeLog("WARN", `Failed to open log folder: ${err?.message || err}`);
  }
}

function showMenu() {
  if (!win) return;
  Menu.buildFromTemplate([
    {
      label: "Toggle Expand / Collapse",
      accelerator: "E",
      click: () => win.webContents.send("widget-toggle-expand"),
    },
    {
      label: "Refresh",
      accelerator: "R",
      click: () => recoverWindow("manual-refresh"),
    },
    {
      label: "Settings",
      accelerator: "CmdOrCtrl+,",
      click: () => openSettingsOverlay(),
    },
    {
      label: "Open log folder",
      click: () => openLogFolder(),
    },
    { type: "separator" },
    { label: `UsageDeck v${app.getVersion()}`, enabled: false },
    updater
      ? { label: "Check for Updates…", click: () => checkForUpdatesManually() }
      : { label: "Updates are only available in the installed app", enabled: false },
    { type: "separator" },
    {
      label: "Quit",
      accelerator: "CmdOrCtrl+Q",
      click: () => app.quit(),
    },
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
    writeLog("WARN", `Manual update check failed: ${err?.message || err}`);
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
  autoUpdater.logger = {
    info: (m) => writeLog("INFO", `updater: ${m}`),
    warn: (m) => writeLog("WARN", `updater: ${m}`),
    error: (m) => writeLog("ERROR", `updater: ${m}`),
    debug: () => {},
  };
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
      writeLog("INFO", `Installing update ${info.version}`);
      server.stop();
      autoUpdater.quitAndInstall();
    }
  });
  const check = () =>
    autoUpdater.checkForUpdates().catch((err) => writeLog("WARN", `Update check failed: ${err?.message || err}`));
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
ipcMain.on("widget-open-settings", () => openSettingsOverlay());
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

app.whenReady().then(async () => {
  writeLog("INFO", "Widget starting up");
  ensureSecret();
  const probe = await server.probe();
  if (probe === "other") {
    showStartupError(
      "Port 3100 is already used by another program. Close that program, then open Provider Usage Monitor again.",
    );
    return;
  }
  if (probe === "down") {
    server.start();
  }
  const ok = await server.waitFor();
  if (!usageSecret) ensureSecret();
  if (ok) {
    server.resetPolicy();
    writeLog("INFO", "Server is ready; creating widget window");
    if (mode.autoUpdate) updater = setupUpdates();
    createWindow();
    startBgSampler({
      settingsFile: SETTINGS_FILE,
      log: writeLog,
      getWindow: () => win,
      shouldStop: () => server.isQuitting(),
    });
  } else {
    showStartupError(
      `The local server did not answer on 127.0.0.1:3100 within 60 seconds. The log is in ${LOG_PATH}.`,
    );
  }
});

app.on("before-quit", () => server.stop());
app.on("window-all-closed", () => app.quit());
