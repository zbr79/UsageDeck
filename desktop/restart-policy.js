const fs = require("fs");
const path = require("path");

const DEFAULT_MAX_BYTES = 1024 * 1024;
const DEFAULT_MAX_CRASHES = 5;
const DEFAULT_WINDOW_MS = 5 * 60 * 1000;
const DEFAULT_BASE_DELAY_MS = 1000;
const DEFAULT_MAX_DELAY_MS = 30_000;

function createRestartPolicy(options = {}) {
  const maxCrashes = options.maxCrashes ?? DEFAULT_MAX_CRASHES;
  const windowMs = options.windowMs ?? DEFAULT_WINDOW_MS;
  const baseDelayMs = options.baseDelayMs ?? DEFAULT_BASE_DELAY_MS;
  const maxDelayMs = options.maxDelayMs ?? DEFAULT_MAX_DELAY_MS;
  let crashTimes = [];

  return {
    recordExit(now = Date.now()) {
      crashTimes = crashTimes.filter((t) => now - t < windowMs);
      crashTimes.push(now);
      const crashesInWindow = crashTimes.length;
      if (crashesInWindow > maxCrashes) {
        return { shouldRestart: false, delayMs: null, crashesInWindow };
      }
      const delayMs = Math.min(maxDelayMs, baseDelayMs * Math.pow(2, crashesInWindow - 1));
      return { shouldRestart: true, delayMs, crashesInWindow };
    },
    reset() {
      crashTimes = [];
    },
    getCrashesInWindow(now = Date.now()) {
      crashTimes = crashTimes.filter((t) => now - t < windowMs);
      return crashTimes.length;
    },
  };
}

function appendRotatedLog(logFile, line, maxBytes = DEFAULT_MAX_BYTES) {
  try {
    fs.mkdirSync(path.dirname(logFile), { recursive: true });
    if (fs.existsSync(logFile)) {
      const stat = fs.statSync(logFile);
      if (stat.size >= maxBytes) {
        const backup = `${logFile}.1`;
        try {
          if (fs.existsSync(backup)) fs.unlinkSync(backup);
        } catch {
          // ignore old backup removal errors
        }
        fs.renameSync(logFile, backup);
      }
    }
    const text = line.endsWith("\n") ? line : `${line}\n`;
    fs.appendFileSync(logFile, text, "utf8");
  } catch {
    // logging must never throw
  }
}

module.exports = {
  createRestartPolicy,
  appendRotatedLog,
};
