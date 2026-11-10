import { EventEmitter } from "node:events";
import { createRequire } from "node:module";
import { describe, expect, it, vi } from "vitest";

const require = createRequire(import.meta.url);
const { classifyProbe } = require("./server-manager.js") as {
  classifyProbe: (ok: boolean, html: string) => "ours" | "other" | "down";
};

describe("classifyProbe", () => {
  it("treats this app's widget page as ours", () => {
    expect(classifyProbe(true, "<title>Provider Usage Monitor</title>")).toBe("ours");
  });

  it("treats another program on the port as other", () => {
    expect(classifyProbe(true, "<title>Something Else</title>")).toBe("other");
  });

  it("treats a failed response as down", () => {
    expect(classifyProbe(false, "Provider Usage Monitor")).toBe("down");
  });
});

describe("packaged launch", () => {
  const { createServerManager } = require("./server-manager.js");

  function fakeChild() {
    const child = new EventEmitter() as EventEmitter & {
      pid: number;
      stdout: EventEmitter;
      stderr: EventEmitter;
      kill: ReturnType<typeof vi.fn>;
    };
    child.pid = 4242;
    child.stdout = new EventEmitter();
    child.stderr = new EventEmitter();
    child.kill = vi.fn();
    return child;
  }

  function startPackaged() {
    const child = fakeChild();
    const spawnImpl = vi.fn(() => child);
    const logs: Array<[string, string]> = [];
    const server = createServerManager({
      launch: {
        kind: "packaged",
        execPath: "/opt/UsageDeck/usagedeck",
        serverEntry: "/opt/UsageDeck/resources/server/server.js",
        dataDir: "/home/me/.config/UsageDeck/data",
      },
      widgetUrl: "http://127.0.0.1:3100/widget",
      log: (level: string, line: string) => logs.push([level, line]),
      getWindow: () => null,
      spawnImpl,
      fetchImpl: vi.fn(),
    });
    server.start();
    return { server, child, spawnImpl, logs };
  }

  it("runs the bundled server.js with Electron as Node, pinned to 127.0.0.1:3100 and the app data dir", () => {
    const { spawnImpl } = startPackaged();
    expect(spawnImpl).toHaveBeenCalledTimes(1);
    const [command, args, options] = spawnImpl.mock.calls[0] as unknown as [string, string[], { env: NodeJS.ProcessEnv }];
    expect(command).toBe("/opt/UsageDeck/usagedeck");
    expect(args).toEqual(["/opt/UsageDeck/resources/server/server.js"]);
    expect(options.env).toMatchObject({
      ELECTRON_RUN_AS_NODE: "1",
      PORT: "3100",
      HOSTNAME: "127.0.0.1",
      USAGE_DATA_DIR: "/home/me/.config/UsageDeck/data",
    });
  });

  it("writes server output to the log", () => {
    const { child, logs } = startPackaged();
    child.stdout.emit("data", "Ready in 120ms\n");
    child.stderr.emit("data", "boom\n");
    expect(logs).toContainEqual(["SERVER", "Ready in 120ms"]);
    expect(logs).toContainEqual(["SERVER_ERR", "boom"]);
  });

  it("stops by killing the child it started", () => {
    const { server, child, spawnImpl } = startPackaged();
    server.stop();
    expect(child.kill).toHaveBeenCalledTimes(1);
    expect(spawnImpl).toHaveBeenCalledTimes(1);
  });
});
