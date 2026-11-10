const { spawn } = require("child_process");
const { createRestartPolicy } = require("./restart-policy");

const WIDGET_MARKER = "Provider Usage Monitor";

function classifyProbe(ok, html) {
  if (!ok) return "down";
  return String(html).includes(WIDGET_MARKER) ? "ours" : "other";
}

// launch is { kind: "dev", appRoot } (npm run start in a checkout) or
// { kind: "packaged", execPath, serverEntry, dataDir } (the bundled standalone server).
function spawnServer(doSpawn, launch) {
  const stdio = ["ignore", "pipe", "pipe"];
  switch (launch.kind) {
    case "packaged":
      return doSpawn(launch.execPath, [launch.serverEntry], {
        env: {
          ...process.env,
          ELECTRON_RUN_AS_NODE: "1",
          PORT: "3100",
          HOSTNAME: "127.0.0.1",
          USAGE_DATA_DIR: launch.dataDir,
        },
        stdio,
        windowsHide: true,
      });
    case "dev":
      return process.platform === "win32"
        ? doSpawn("cmd.exe", ["/c", "npm run start"], { cwd: launch.appRoot, stdio, windowsHide: true })
        : doSpawn("npm", ["run", "start"], { cwd: launch.appRoot, stdio, detached: true });
    default:
      throw new Error(`Unknown server launch kind: ${launch.kind}`);
  }
}

function killServer(doSpawn, launch, child) {
  if (launch.kind === "packaged") {
    child.kill();
    return;
  }
  const pid = child.pid;
  if (!pid) return;
  if (process.platform === "win32") {
    doSpawn("taskkill", ["/pid", String(pid), "/f", "/t"], { windowsHide: true, stdio: "ignore" });
  } else {
    process.kill(-pid, "SIGTERM");
  }
}

function createServerManager({ launch, widgetUrl, log, getWindow, fetchImpl, spawnImpl }) {
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
      // Expected while the server is still starting up
      return "down";
    }
  }

  async function isUp() {
    return (await probe()) === "ours";
  }

  function pipeStreamToLog(stream, label) {
    if (!stream) return;
    stream.on("data", (chunk) => {
      const text = String(chunk).trim();
      if (!text) return;
      for (const line of text.split(/\r?\n/)) {
        log(label, line);
      }
    });
  }

  function start() {
    log("INFO", "Spawning local Next.js server on 127.0.0.1:3100");
    const child = spawnServer(doSpawn, launch);
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
      if (!(await isUp())) start();
      const ok = await waitFor(30000);
      if (ok) {
        restartPolicy.reset();
        log("INFO", "Server recovered after restart");
        const win = getWindow();
        if (win && !win.isDestroyed()) {
          win.webContents.send("widget-server-status", { state: "online" });
          win.loadURL(widgetUrl);
        }
      }
    }, decision.delayMs);
  }

  function stop() {
    quitting = true;
    if (restartTimer) {
      clearTimeout(restartTimer);
      restartTimer = null;
    }
    if (!spawnedServer || !serverChild) return;
    const child = serverChild;
    serverChild = null;
    spawnedServer = false;
    try {
      killServer(doSpawn, launch, child);
    } catch (err) {
      log("WARN", `Failed to stop server process ${child.pid}: ${err?.message || err}`);
    }
  }

  async function waitFor(timeoutMs = 60000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      if (await isUp()) return true;
      await new Promise((r) => setTimeout(r, 1000));
    }
    return false;
  }

  return {
    probe,
    isUp,
    start,
    stop,
    waitFor,
    scheduleRestart,
    isQuitting: () => quitting,
    hasChild: () => Boolean(serverChild),
    hasRestartTimer: () => Boolean(restartTimer),
    resetPolicy: () => restartPolicy.reset(),
  };
}

module.exports = { classifyProbe, createServerManager };
