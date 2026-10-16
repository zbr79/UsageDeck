const { spawn } = require("child_process");
const { createRestartPolicy } = require("./restart-policy");

const WIDGET_MARKER = "Provider Usage Monitor";

/** "ours" is this app, "other" is some different program on the port, "down" is nothing. */
function classifyProbe(ok, html) {
  if (!ok) return "down";
  return String(html).includes(WIDGET_MARKER) ? "ours" : "other";
}

function createServerManager({ appRoot, widgetUrl, log, getWindow, fetchImpl, spawnImpl }) {
  const doFetch = fetchImpl ?? fetch;
  const doSpawn = spawnImpl ?? spawn;
  let serverChild = null;
  let spawnedServer = false;
  let quitting = false;
  let restartTimer = null;
  const restartPolicy = createRestartPolicy();

  function notify(payload) {
    const win = getWindow();
    if (win && !win.isDestroyed()) {
      win.webContents.send("widget-server-status", payload);
    }
  }

  async function probe() {
    try {
      const res = await doFetch(widgetUrl, { signal: AbortSignal.timeout(2000) });
      if (!res.ok) return "down";
      return classifyProbe(true, await res.text());
    } catch {
      return "down";
    }
  }

  function pipeStreamToLog(stream, label) {
    if (!stream) return;
    stream.on("data", (chunk) => {
      const text = String(chunk).trim();
      if (!text) return;
      for (const line of text.split(/\r?\n/)) log(label, line);
    });
  }

  function start() {
    log("INFO", "Spawning local Next.js server on 127.0.0.1:3100");
    const isWin = process.platform === "win32";
    const child = isWin
      ? doSpawn("cmd.exe", ["/c", "npm run start"], {
          cwd: appRoot,
          stdio: ["ignore", "pipe", "pipe"],
          windowsHide: true,
        })
      : doSpawn("npm", ["run", "start"], {
          cwd: appRoot,
          stdio: ["ignore", "pipe", "pipe"],
          detached: true,
        });
    serverChild = child;
    spawnedServer = true;
    pipeStreamToLog(child.stdout, "SERVER");
    pipeStreamToLog(child.stderr, "SERVER_ERR");

    child.on("error", (err) => {
      log("ERROR", `Failed to spawn server process: ${err?.message || err}`);
    });

    child.on("exit", (code, signal) => {
      if (serverChild === child) {
        serverChild = null;
        spawnedServer = false;
      }
      if (quitting) return;
      scheduleRestart(`code=${code ?? "null"} signal=${signal ?? "null"}`);
    });
  }

  function scheduleRestart(reason) {
    if (quitting || restartTimer) return;
    const decision = restartPolicy.recordExit();
    if (!decision.shouldRestart) {
      log(
        "ERROR",
        `Server exited (${reason}); ${decision.crashesInWindow} crashes in 5m, giving up automatic restart`,
      );
      notify({ state: "stopped", crashes: decision.crashesInWindow });
      return;
    }
    log(
      "WARN",
      `Server exited (${reason}); restarting in ${decision.delayMs}ms (attempt ${decision.crashesInWindow})`,
    );
    notify({
      state: "restarting",
      delayMs: decision.delayMs,
      attempt: decision.crashesInWindow,
    });
    restartTimer = setTimeout(async () => {
      restartTimer = null;
      if (quitting) return;
      if ((await probe()) !== "ours") start();
      const ok = await waitFor(30000);
      if (!ok) return;
      restartPolicy.reset();
      log("INFO", "Server recovered after restart");
      notify({ state: "online" });
      const win = getWindow();
      if (win && !win.isDestroyed()) win.loadURL(widgetUrl);
    }, decision.delayMs);
  }

  function stop() {
    quitting = true;
    if (restartTimer) {
      clearTimeout(restartTimer);
      restartTimer = null;
    }
    if (!spawnedServer || !serverChild) return;
    const pid = serverChild.pid;
    serverChild = null;
    spawnedServer = false;
    if (!pid) return;
    try {
      if (process.platform === "win32") {
        doSpawn("taskkill", ["/pid", String(pid), "/f", "/t"], {
          windowsHide: true,
          stdio: "ignore",
        });
      } else {
        process.kill(-pid, "SIGTERM");
      }
    } catch (err) {
      log("WARN", `Failed to stop server process ${pid}: ${err?.message || err}`);
    }
  }

  async function waitFor(timeoutMs = 60000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      if ((await probe()) === "ours") return true;
      await new Promise((r) => setTimeout(r, 1000));
    }
    return false;
  }

  return { probe, start, stop, waitFor };
}

module.exports = { createServerManager, classifyProbe };
