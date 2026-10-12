const { contextBridge, ipcRenderer } = require("electron");

const secret = ipcRenderer.sendSync("get-usage-secret") ?? "";

function listen(channel, cb) {
  const listener = (_e, ...args) => cb(...args);
  ipcRenderer.on(channel, listener);
  return () => ipcRenderer.removeListener(channel, listener);
}

contextBridge.exposeInMainWorld("widget", {
  secret,
  resize: (width, height) => ipcRenderer.send("widget-resize", width, height),
  onBg: (cb) => listen("widget-bg", cb),
  onToggleExpand: (cb) => listen("widget-toggle-expand", () => cb()),
  onRefresh: (cb) => listen("widget-refresh", () => cb()),
  onSettingsChanged: (cb) => listen("widget-settings-changed", () => cb()),
  showMenu: () => ipcRenderer.send("widget-show-menu"),
  startDrag: (sx, sy) => ipcRenderer.send("widget-drag-start", sx, sy),
  moveDrag: (sx, sy) => ipcRenderer.send("widget-drag-move", sx, sy),
  endDrag: () => ipcRenderer.send("widget-drag-end"),
});
