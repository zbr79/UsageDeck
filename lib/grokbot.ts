import path from "path";
import os from "os";
import { readJson } from "./fs-atomic";
import { getAppKey, unseal } from "./os-crypt";
import { createProvider, type ProviderResult } from "./provider-cache";
import { collectAccounts } from "./tracked-accounts";
import { cursorHeaders, readCursorState } from "./cursor";
import { numberOrNull, stringOrNull, toIso } from "./time";
import type { GrokUsage, TrackedAccount } from "./types";

export type { GrokUsage } from "./types";

const GROK_DIR = path.join(
  process.env.APPDATA ?? path.join(os.homedir(), "AppData", "Roaming"),
  "Grok Bot",
);
const SECRETS_FILE = path.join(GROK_DIR, "sand-secrets.json");
const LOCAL_STATE_FILE = path.join(GROK_DIR, "Local State");
const SAND_URL = "https://api2.cursor.sh/aiserver.v1.DashboardService/GetSandUsageStatus";

interface GrokAccount {
  id: string;
  token: string;
  email: string | null;
  name: string | null;
  source: GrokUsage["source"];
  active: boolean;
}

function profileOf(raw: string | null): { email: string | null; name: string | null } {
  if (!raw) return { email: null, name: null };
  try {
    const p = JSON.parse(raw);
    return { email: stringOrNull(p?.email), name: stringOrNull(p?.name) };
  } catch {
    return { email: null, name: null };
  }
}

/** Every account stored by Grok Bot, not only the one selected in the app. */
async function readGrokBotAccounts(): Promise<GrokAccount[]> {
  const secrets = await readJson<Record<string, string>>(SECRETS_FILE);
  let parsed: { active?: string; accounts?: Record<string, Record<string, string>> };
  try {
    parsed = JSON.parse(secrets?.["cursor-accounts"] ?? "");
  } catch {
    return [];
  }
  const entries = parsed?.accounts;
  if (!entries) return [];
  const key = await getAppKey(LOCAL_STATE_FILE);
  if (!key) return [];
  const activeKey = typeof parsed.active === "string" ? parsed.active : "";
  const byId = new Map<string, GrokAccount>();
  for (const [storageId, entry] of Object.entries(entries)) {
    const token = unseal(key, entry?.["cursor-access-token"]);
    if (!token) continue;
    const { email, name } = profileOf(unseal(key, entry?.["cursor-account-profile"]));
    const id = (email ?? storageId).trim().toLowerCase();
    if (!id) continue;
    const account: GrokAccount = {
      id,
      token,
      email,
      name,
      source: "grok-bot",
      active: storageId === activeKey || email?.toLowerCase() === activeKey.toLowerCase(),
    };
    const prev = byId.get(id);
    if (!prev || (account.active && !prev.active)) byId.set(id, account);
  }
  const out = [...byId.values()];
  if (out.length === 1) out[0].active = true;
  return out;
}

/**
 * Grok Bot's saved sign-ins. When it has none, fall back to Cursor's
 * current sign-in (same backend) as a single account.
 */
async function readAccounts(): Promise<GrokAccount[]> {
  const own = await readGrokBotAccounts();
  if (own.length) return own;
  const cursor = await readCursorState();
  if (!cursor.token) return [];
  const label = cursor.accountName?.trim() || null;
  return [
    {
      id: (label ?? "cursor").toLowerCase(),
      token: cursor.token,
      email: label,
      name: null,
      source: "cursor",
      active: true,
    },
  ];
}

export function parseGrokUsage(
  json: unknown,
  account: { email: string | null; name: string | null; source: GrokUsage["source"] },
): GrokUsage {
  const sj = json as Record<string, unknown> | null;
  return {
    email: account.email,
    name: account.name,
    percentUsed: numberOrNull(sj?.usagePercent),
    resetAt: toIso(sj?.nextResetTimestampUtc),
    planLabel: stringOrNull(sj?.grokPlanLabel),
    cursorPlanName: stringOrNull(sj?.cursorPlanName),
    source: account.source,
  };
}

const provider = createProvider<GrokUsage>({
  section: "grokbot",
  ttlMs: 30_000,
  async fetch(key: string) {
    const account = (await readAccounts()).find((a) => a.id === key);
    if (!account) return { ok: false };
    const res = await fetch(SAND_URL, {
      method: "POST",
      headers: cursorHeaders(account.token),
      body: "{}",
      cache: "no-store",
      signal: AbortSignal.timeout(10_000),
    });
    if (res.status === 401) return { ok: false, authExpired: true };
    if (!res.ok) return { ok: false };
    const data = parseGrokUsage(await res.json(), account);
    return data.percentUsed == null ? { ok: false } : { ok: true, data };
  },
});

const EMPTY: ProviderResult<GrokUsage> = {
  usage: null,
  stale: false,
  cachedAt: null,
  authExpired: false,
};

export async function getGrokAccounts(): Promise<TrackedAccount<GrokUsage>[]> {
  const accounts = await readAccounts();
  return collectAccounts(
    "grok",
    "grokbot",
    accounts.map((account) => ({
      id: account.id,
      label: account.email ?? account.name,
      active: account.active,
      load: () => provider.get(account.id),
    })),
  );
}

export async function getGrokUsage(): Promise<ProviderResult<GrokUsage>> {
  const accounts = await getGrokAccounts();
  const active = accounts.find((a) => a.active) ?? accounts[0];
  if (!active) return EMPTY;
  return {
    usage: active.usage,
    stale: active.stale,
    cachedAt: active.cachedAt,
    authExpired: active.authExpired,
  };
}
