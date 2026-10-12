import { promises as fs } from "fs";
import crypto from "crypto";
import { dataDir, dataFile } from "./paths";

export const SECRET_HEADER = "x-usage-secret";

let cached: { file: string; secret: string } | null = null;

async function readSecret(file: string): Promise<string | null> {
  try {
    const existing = (await fs.readFile(file, "utf8")).trim();
    return existing || null;
  } catch {
    return null;
  }
}

/** Shared secret between the Electron shell and this server (data/.api-secret). */
export async function getOrCreateSecret(): Promise<string> {
  const file = dataFile(".api-secret");
  if (cached?.file === file) return cached.secret;
  let secret = await readSecret(file);
  if (!secret) {
    const fresh = crypto.randomBytes(32).toString("hex");
    await fs.mkdir(dataDir(), { recursive: true });
    try {
      // 'wx' fails if the Electron shell created it first; then use theirs.
      await fs.writeFile(file, fresh, { encoding: "utf8", mode: 0o600, flag: "wx" });
      secret = fresh;
    } catch {
      secret = (await readSecret(file)) ?? fresh;
    }
  }
  cached = { file, secret };
  return secret;
}

export function secretsMatch(provided: string | null | undefined, secret: string): boolean {
  if (!provided) return false;
  const a = Buffer.from(provided);
  const b = Buffer.from(secret);
  return a.length === b.length && crypto.timingSafeEqual(a, b);
}

/** Route-level check (defense in depth; proxy.ts gates every /api request too). */
export async function requireLocalSecret(req: Request): Promise<Response | null> {
  const secret = await getOrCreateSecret();
  if (secretsMatch(req.headers.get(SECRET_HEADER), secret)) return null;
  return Response.json({ error: "unauthorized" }, { status: 401 });
}
