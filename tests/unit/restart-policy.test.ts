import { mkdtemp, readFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { createRequire } from "node:module";
import { afterEach, describe, expect, it } from "vitest";

const require = createRequire(import.meta.url);
const { createRestartPolicy } = require("../../desktop/restart-policy.js");
const { appendRotatedLog } = require("../../desktop/log.js");
const { classifyProbe } = require("../../desktop/server-manager.js");

describe("restart policy", () => {
  it("backs off, then stops after too many crashes in the window", () => {
    const policy = createRestartPolicy({
      maxCrashes: 3,
      windowMs: 60_000,
      baseDelayMs: 1000,
      maxDelayMs: 10_000,
    });
    expect(policy.recordExit(0)).toEqual({
      shouldRestart: true,
      delayMs: 1000,
      crashesInWindow: 1,
    });
    expect(policy.recordExit(1000)).toEqual({
      shouldRestart: true,
      delayMs: 2000,
      crashesInWindow: 2,
    });
    expect(policy.recordExit(2000)).toEqual({
      shouldRestart: true,
      delayMs: 4000,
      crashesInWindow: 3,
    });
    expect(policy.recordExit(3000)).toEqual({
      shouldRestart: false,
      delayMs: null,
      crashesInWindow: 4,
    });
    policy.reset();
    expect(policy.recordExit(4000).shouldRestart).toBe(true);
  });
});

describe("server probe", () => {
  it("tells our widget apart from another program on the same port", () => {
    expect(classifyProbe(false, "")).toBe("down");
    expect(classifyProbe(true, "<title>Provider Usage Monitor</title>")).toBe("ours");
    expect(classifyProbe(true, "<title>Something else</title>")).toBe("other");
  });
});

describe("rotated log", () => {
  let dir = "";
  afterEach(async () => {
    if (dir) await rm(dir, { recursive: true, force: true });
  });

  it("rotates once the file reaches the size cap", async () => {
    dir = await mkdtemp(path.join(os.tmpdir(), "usage-log-"));
    const file = path.join(dir, "main.log");
    appendRotatedLog(file, "first", 4);
    appendRotatedLog(file, "second-line", 4);
    expect(await readFile(`${file}.1`, "utf8")).toContain("first");
    expect(await readFile(file, "utf8")).toContain("second-line");
  });
});
