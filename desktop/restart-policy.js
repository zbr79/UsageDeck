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
      const delayMs = Math.min(maxDelayMs, baseDelayMs * 2 ** (crashesInWindow - 1));
      return { shouldRestart: true, delayMs, crashesInWindow };
    },
    reset() {
      crashTimes = [];
    },
  };
}

module.exports = { createRestartPolicy };
