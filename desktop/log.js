const fs = require("fs");
const path = require("path");

/** Append one line, rotating to `file.1` once the log reaches maxBytes. Never throws. */
function appendRotatedLog(logFile, line, maxBytes = 1024 * 1024) {
  try {
    fs.mkdirSync(path.dirname(logFile), { recursive: true });
    if (fs.existsSync(logFile)) {
      const stat = fs.statSync(logFile);
      if (stat.size >= maxBytes) {
        const backup = `${logFile}.1`;
        try {
          if (fs.existsSync(backup)) fs.unlinkSync(backup);
        } catch {
          // An old backup that cannot be removed should not block the new line.
        }
        fs.renameSync(logFile, backup);
      }
    }
    const text = line.endsWith("\n") ? line : `${line}\n`;
    fs.appendFileSync(logFile, text, "utf8");
  } catch {
    // Logging must never take down the window.
  }
}

module.exports = { appendRotatedLog };
