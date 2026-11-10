import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const desktopDir = path.resolve(__dirname);
const pkg = JSON.parse(readFileSync(path.join(desktopDir, "package.json"), "utf8"));
const packagedFiles: string[] = pkg.build.files;

function localDependencies(file: string): string[] {
  const source = readFileSync(path.join(desktopDir, file), "utf8");
  const required = [...source.matchAll(/require\("\.\/([^"]+?)(?:\.js)?"\)/g)].map((m) => `${m[1]}.js`);
  const preloads = [...source.matchAll(/path\.join\(__dirname, "([^"]+\.js)"\)/g)].map((m) => m[1]);
  return [...required, ...preloads];
}

describe("electron-builder files", () => {
  it("ships every desktop file main.js loads at runtime", () => {
    const seen = new Set<string>();
    const queue = [pkg.main as string];
    while (queue.length) {
      const file = queue.pop()!;
      if (seen.has(file)) continue;
      seen.add(file);
      queue.push(...localDependencies(file));
    }
    expect([...seen].filter((f) => !packagedFiles.includes(f))).toEqual([]);
  });

});
