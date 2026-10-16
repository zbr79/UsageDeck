const {
  app,
  BrowserWindow,
  ipcMain,
  screen,
  desktopCapturer,
  dialog,
  session,
} = require("electron");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const { appendRotatedLog } = require("./log");
const { createServerManager } = require("./server-manager");

const APP_ROOT = path.resolve(__dirname, "..");
const ORIGIN = "http://127.0.0.1:3100";
const WIDGET_URL = `${ORIGIN}/widget`;
const SETTINGS_URL = `${ORIGIN}/settings`;
const WIDTH = 238;
const CONFIG_PATH = path.join(app.getPath("userData"), "widget-config.json");
const DATA_DIR = process.env.USAGE_DATA_DIR || path.join(APP_ROOT, "data");
const SECRET_PATH = path.join(DATA_DIR, ".api-secret");
const SETTINGS_FILE = path.join(DATA_DIR, "settings.json");

let win = null;
let settingsWin = null;
let dragOffset = null;
let usageSecret = "";
let logFile = "";
let stopServer = () => {};

// One widget at a time: a second launch just surfaces the existing one
// (otherwise quitting either instance would kill the shared server).
const gotLock = app.requestSingleInstanceLock();
if (!gotLock) app.quit();
app.on("second-instance", () => {
  if (win && !win.isDestroyed()) win.showInactive();
});

function isAppUrl(url) {
  try {
    return new URL(url).origin === ORIGIN;
  } catch {
    return false;
  }
}

function fromApp(event) {
  return !!event.senderFrame && isAppUrl(event.senderFrame.url);
}

/** Keep every window on our own origin: no navigation away, no popups. */
function lockDown(w) {
  w.webContents.setWindowOpenHandler(() => ({ action: "deny" }));
  const guard = (e, url) => {
    if (!isAppUrl(url)) e.preventDefault();
  };
  w.webContents.on("will-navigate", guard);
  w.webContents.on("will-redirect", guard);
}

const secureWebPreferences = (preload) => ({
  preload: path.join(__dirname, preload),
  contextIsolation: true,
  nodeIntegration: false,
  sandbox: true,
  webviewTag: false,
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
    return null;
  }
}

function savePos() {
  if (!win || win.isDestroyed()) return;
  const b = win.getBounds();
  try {
    fs.mkdirSync(path.dirname(CONFIG_PATH), { recursive: true });
    fs.writeFileSync(CONFIG_PATH, JSON.stringify({ x: b.x, y: b.y }));
  } catch {
    // ignore
  }
}

function ensureSecret() {
  try {
    fs.mkdirSync(DATA_DIR, { recursive: true });
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
    icon: path.join(APP_ROOT, "app-icon.ico"),
    webPreferences: secureWebPreferences("preload.js"),
  });
  lockDown(win);

  win.setMinimumSize(WIDTH, 40);
  win.setMaximumSize(WIDTH, 8000);

  win.setAlwaysOnTop(true, "screen-saver");
  win.setVisibleOnAllWorkspaces(true, { visibleOnFullScreen: true });
  win.loadURL(WIDGET_URL);

  let saveTimer = null;
  win.on("move", () => {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(savePos, 300);
  });

  win.webContents.on("context-menu", (e) => e.preventDefault());

  // If the renderer dies (crash / OOM), bring it back instead of leaving a
  // blank transparent window on screen.
  win.webContents.on("render-process-gone", () => {
    setTimeout(() => {
      if (win && !win.isDestroyed()) win.loadURL(WIDGET_URL);
    }, 1000);
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
  const disp =
    win && !win.isDestroyed()
      ? screen.getDisplayMatching(win.getBounds())
      : screen.getPrimaryDisplay();
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
    webPreferences: secureWebPreferences("settings-preload.js"),
  });
  lockDown(settingsWin);
  settingsWin.setAlwaysOnTop(true, "screen-saver");
  settingsWin.loadURL(SETTINGS_URL);
  settingsWin.on("closed", () => {
    settingsWin = null;
    if (win && !win.isDestroyed()) win.webContents.send("widget-settings-changed");
  });
}

ipcMain.on("get-usage-secret", (e) => {
  e.returnValue = fromApp(e) ? usageSecret : "";
});
ipcMain.on("settings-close", (e) => {
  if (!fromApp(e)) return;
  if (settingsWin && !settingsWin.isDestroyed()) settingsWin.close();
});
ipcMain.on("widget-menu-action", (e, action) => {
  if (!fromApp(e)) return;
  if (action === "settings") openSettingsOverlay();
  else if (action === "quit") app.quit();
});
ipcMain.on("widget-drag-start", (e, sx, sy) => {
  if (!fromApp(e) || !win) return;
  const [x, y] = win.getPosition();
  dragOffset = { dx: sx - x, dy: sy - y };
});
ipcMain.on("widget-drag-move", (e, sx, sy) => {
  if (!fromApp(e) || !win || !dragOffset) return;
  win.setPosition(Math.round(sx - dragOffset.dx), Math.round(sy - dragOffset.dy));
});
ipcMain.on("widget-drag-end", (e) => {
  if (!fromApp(e)) return;
  dragOffset = null;
  savePos();
});
ipcMain.on("widget-resize", (e, _width, height) => {
  if (!fromApp(e) || !win || !Number.isFinite(height)) return;
  const disp = screen.getDisplayMatching(win.getBounds());
  const maxH = disp.workArea.height - 8;
  const h = Math.max(40, Math.min(Math.round(height), maxH));
  win.setSize(WIDTH, h);
});

function readThemeMode() {
  try {
    const s = JSON.parse(fs.readFileSync(SETTINGS_FILE, "utf8"));
    return typeof s?.themeMode === "string" ? s.themeMode : "auto";
  } catch {
    return "auto"; // no settings file yet
  }
}

/**
 * Average brightness (0..1) of the screen just around the widget, on the
 * display the widget is actually on. The widget's own pixels are excluded.
 */
async function sampleBackground() {
  const bounds = win.getBounds();
  const display = screen.getDisplayMatching(bounds);
  const db = display.bounds;
  const thumbW = 320;
  const sources = await desktopCapturer.getSources({
    types: ["screen"],
    thumbnailSize: { width: thumbW, height: Math.round((thumbW * db.height) / db.width) },
  });
  const src = sources.find((s) => s.display_id === String(display.id));
  if (!src || src.thumbnail.isEmpty()) return null;
  const size = src.thumbnail.getSize();
  const bmp = src.thumbnail.toBitmap(); // BGRA
  const sx = size.width / db.width;
  const sy = size.height / db.height;
  const clampX = (v) => Math.max(0, Math.min(size.width, Math.round(v)));
  const clampY = (v) => Math.max(0, Math.min(size.height, Math.round(v)));
  const margin = 48;
  const outer = {
    x0: clampX((bounds.x - db.x - margin) * sx),
    y0: clampY((bounds.y - db.y - margin) * sy),
    x1: clampX((bounds.x - db.x + bounds.width + margin) * sx),
    y1: clampY((bounds.y - db.y + bounds.height + margin) * sy),
  };
  const inner = {
    x0: clampX((bounds.x - db.x) * sx),
    y0: clampY((bounds.y - db.y) * sy),
    x1: clampX((bounds.x - db.x + bounds.width) * sx),
    y1: clampY((bounds.y - db.y + bounds.height) * sy),
  };
  let sum = 0;
  let count = 0;
  for (let y = outer.y0; y < outer.y1; y++) {
    for (let x = outer.x0; x < outer.x1; x++) {
      if (x >= inner.x0 && x < inner.x1 && y >= inner.y0 && y < inner.y1) continue;
      const i = (y * size.width + x) * 4;
      sum += (0.0722 * bmp[i] + 0.7152 * bmp[i + 1] + 0.2126 * bmp[i + 2]) / 255;
      count++;
    }
  }
  return count ? sum / count : null;
}

function startBgSampler() {
  const tick = async () => {
    let delay = 5000;
    // Only the "auto" theme needs screen sampling; light/white/dark are static.
    if (readThemeMode() === "auto" && win && !win.isDestroyed() && win.isVisible()) {
      delay = 3000;
      try {
        const avg = await sampleBackground();
        if (avg != null) win.webContents.send("widget-bg", avg);
      } catch {
        // ignore sampling errors
      }
    }
    setTimeout(tick, delay);
  };
  tick();
}

app.on("web-contents-created", (_e, contents) => {
  contents.on("will-attach-webview", (e) => e.preventDefault());
});

app.whenReady().then(async () => {
  if (!gotLock) return;
  // Only clipboard writes (copy key) are needed; deny every other permission.
  session.defaultSession.setPermissionRequestHandler((wc, permission, cb) => {
    cb(permission === "clipboard-sanitized-write" && isAppUrl(wc.getURL()));
  });
  ensureSecret();
  logFile = path.join(app.getPath("userData"), "logs", "main.log");
  const log = (level, message) =>
    appendRotatedLog(logFile, `${new Date().toISOString()} [${level}] ${message}`);
  const server = createServerManager({
    appRoot: APP_ROOT,
    widgetUrl: WIDGET_URL,
    log,
    getWindow: () => win,
  });
  stopServer = () => server.stop();
  log("INFO", "Widget starting");

  const probe = await server.probe();
  if (probe === "other") {
    dialog.showErrorBox(
      "Provider Usage Monitor",
      "Port 3100 is already used by another program. Close that program, then open Provider Usage Monitor again.",
    );
    app.quit();
    return;
  }
  if (probe === "down") server.start();
  if (await server.waitFor()) {
    createWindow();
    startBgSampler();
  } else {
    server.stop();
    dialog.showErrorBox(
      "Provider Usage Monitor",
      `The local server did not start on ${ORIGIN} within 60 seconds.\n\n` +
        "Run `npm run build` in the project folder, then launch the widget again.\n" +
        `The log is in ${path.dirname(logFile)}.`,
    );
    app.quit();
  }
});

app.on("before-quit", () => stopServer());
app.on("window-all-closed", () => app.quit());
