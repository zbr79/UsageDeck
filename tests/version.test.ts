import { describe, expect, it } from "vitest";
import cargoLock from "../src-tauri/Cargo.lock?raw";
import cargoToml from "../src-tauri/Cargo.toml?raw";
import tauriConf from "../src-tauri/tauri.conf.json?raw";
import packageLock from "../package-lock.json?raw";
import packageJson from "../package.json?raw";

// The release tag (v<version>) and the version the installed app compares against it both come
// from tauri.conf.json; the other files carry copies that must not drift.
function versions(): Record<string, string | undefined> {
  const lock = JSON.parse(packageLock);
  const cargoLockEntry = cargoLock.match(
    /\[\[package\]\]\nname = "usagedeck"\nversion = "([^"]+)"/,
  );
  return {
    "src-tauri/tauri.conf.json": JSON.parse(tauriConf).version,
    "src-tauri/Cargo.toml": cargoToml.match(/^version = "([^"]+)"/m)?.[1],
    "src-tauri/Cargo.lock": cargoLockEntry?.[1],
    "package.json": JSON.parse(packageJson).version,
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
