import { promises as fs } from "fs";
import crypto from "crypto";
import { execFile } from "child_process";

/**
 * Read-only access to Chromium/Electron "safeStorage" secrets (Claude
 * desktop, Grok Bot): the AES key in `Local State` is DPAPI-protected for
 * the current Windows user; values are sealed as "v10" + nonce + AES-GCM.
 */

function dpapiUnprotect(encryptedKeyB64: string): Promise<Buffer> {
  const script = [
    "Add-Type -AssemblyName System.Security",
    "$enc = [Convert]::FromBase64String($env:OS_KEY)",
    "$blob = $enc[5..($enc.Length - 1)]",
    "$key = [System.Security.Cryptography.ProtectedData]::Unprotect($blob, $null, [System.Security.Cryptography.DataProtectionScope]::CurrentUser)",
    "[Convert]::ToBase64String($key)",
  ].join("\n");
  return new Promise((resolve, reject) => {
    execFile(
      "powershell.exe",
      ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script],
      {
        env: { ...process.env, OS_KEY: encryptedKeyB64 },
        windowsHide: true,
        timeout: 10_000,
        maxBuffer: 1024 * 1024,
      },
      (err, stdout) => {
        if (err) return reject(err);
        const lines = stdout.trim().split(/\r?\n/).filter(Boolean);
        const b64 = lines[lines.length - 1]?.trim();
        if (!b64) return reject(new Error("empty dpapi output"));
        resolve(Buffer.from(b64, "base64"));
      },
    );
  });
}

// Keyed by the encrypted blob, so a re-keyed app (new Local State) misses
// the cache naturally. Avoids spawning PowerShell on every refresh.
const keyCache = new Map<string, Promise<Buffer | null>>();

/** The AES-256 key protecting an app's safeStorage values, or null. */
export async function getAppKey(localStatePath: string): Promise<Buffer | null> {
  let encryptedKey: unknown;
  try {
    encryptedKey = JSON.parse(await fs.readFile(localStatePath, "utf8"))?.os_crypt?.encrypted_key;
  } catch {
    return null;
  }
  if (typeof encryptedKey !== "string" || !encryptedKey) return null;
  let pending = keyCache.get(encryptedKey);
  if (!pending) {
    pending = dpapiUnprotect(encryptedKey)
      .then((key) => (key.length === 32 ? key : null))
      .catch(() => null);
    keyCache.set(encryptedKey, pending);
    // Don't pin failures: allow a retry on the next refresh.
    void pending.then((key) => {
      if (!key) keyCache.delete(encryptedKey as string);
    });
  }
  return pending;
}

export function unseal(key: Buffer, sealedB64: unknown): string | null {
  if (typeof sealedB64 !== "string") return null;
  try {
    const b = Buffer.from(sealedB64, "base64");
    if (b.length < 3 + 12 + 16) return null;
    if (b.subarray(0, 3).toString("latin1") !== "v10") return null;
    const nonce = b.subarray(3, 15);
    const tag = b.subarray(b.length - 16);
    const ct = b.subarray(15, b.length - 16);
    const d = crypto.createDecipheriv("aes-256-gcm", key, nonce);
    d.setAuthTag(tag);
    return Buffer.concat([d.update(ct), d.final()]).toString("utf8");
  } catch {
    return null;
  }
}
