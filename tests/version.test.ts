import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");

// The release workflow tags v<tauri.conf.json version>, and the installed app compares its
// Cargo version against that tag, so every copy of the version must agree.
function versions(): Record<string, string | undefined> {
  const lock = JSON.parse(read("package-lock.json"));
  const cargoLockEntry = read("src-tauri/Cargo.lock").match(
    /\[\[package\]\]\nname = "usagedeck"\nversion = "([^"]+)"/,
  );
  return {
    "src-tauri/tauri.conf.json": JSON.parse(read("src-tauri/tauri.conf.json")).version,
    "src-tauri/Cargo.toml": read("src-tauri/Cargo.toml").match(/^version = "([^"]+)"/m)?.[1],
    "src-tauri/Cargo.lock": cargoLockEntry?.[1],
    "package.json": JSON.parse(read("package.json")).version,
    "package-lock.json": lock.version,
    'package-lock.json packages[""]': lock.packages[""].version,
  };
}

describe("app version", () => {
  it("is the same semver in every file that carries it", () => {
    const found = versions();
    const expected = found["src-tauri/tauri.conf.json"];
    expect(expected).toMatch(/^\d+\.\d+\.\d+$/);
    for (const [file, version] of Object.entries(found)) {
      expect(version, `${file} must say ${expected}`).toBe(expected);
    }
  });
});
