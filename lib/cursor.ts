import path from "path";
import os from "os";
import { createProvider, type ProviderResult } from "./provider-cache";
import { numberOrNull, stringOrNull, toIso } from "./time";
import type { CursorUsage } from "./types";

export type { CursorUsage } from "./types";

const STATE_DB = path.join(
  process.env.APPDATA ?? path.join(os.homedir(), "AppData", "Roaming"),
  "Cursor",
  "User",
  "globalStorage",
  "state.vscdb",
);
const API = "https://api2.cursor.sh/aiserver.v1.DashboardService";
const USAGE_URL = `${API}/GetCurrentPeriodUsage`;
const PLAN_URL = `${API}/GetPlanInfo`;

interface CursorState {
  token: string | null;
  accountName: string | null;
}

let stateCache: { at: number; state: CursorState } | null = null;

/**
 * Cursor's sign-in, read (read-only) from its SQLite state DB in one open.
 * Requires node:sqlite (Node 22.13+).
 */
export async function readCursorState(): Promise<CursorState> {
  if (stateCache && Date.now() - stateCache.at < 60_000) return stateCache.state;
  const state: CursorState = { token: null, accountName: null };
  try {
    const { DatabaseSync } = await import("node:sqlite");
    const db = new DatabaseSync(STATE_DB, { readOnly: true });
    try {
      const get = (key: string) =>
        (
          db.prepare("SELECT value FROM ItemTable WHERE key = ?").get(key) as
            { value: string } | undefined
        )?.value ?? null;
      state.token = get("cursorAuth/accessToken");
      state.accountName = get("cursorAuth/cachedEmail");
      if (!state.accountName) {
        const profile = get("cursorAuth/cachedScopedProfile");
        try {
          state.accountName = profile ? (JSON.parse(profile)?.displayName ?? null) : null;
        } catch {
          // malformed profile
        }
      }
    } finally {
      db.close();
    }
  } catch {
    // Cursor not installed, DB locked, or node:sqlite unavailable
  }
  stateCache = { at: Date.now(), state };
  return state;
}

export function cursorHeaders(token: string): Record<string, string> {
  return {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
    Accept: "application/json",
    "User-Agent": "Mozilla/5.0",
  };
}

export function parseCursorUsage(
  usageJson: unknown,
  planJson: unknown,
  accountName: string | null,
): CursorUsage {
  const json = usageJson as Record<string, any> | null;
  const pu = json?.planUsage;
  return {
    autoPercentUsed: numberOrNull(pu?.autoPercentUsed),
    apiPercentUsed: numberOrNull(pu?.apiPercentUsed),
    totalPercentUsed: numberOrNull(pu?.totalPercentUsed),
    billingCycleEnd: toIso(json?.billingCycleEnd),
    totalSpend: numberOrNull(pu?.totalSpend),
    limit: numberOrNull(pu?.limit),
    bonusSpend: numberOrNull(pu?.bonusSpend),
    accountName,
    planName: stringOrNull((planJson as Record<string, any> | null)?.planInfo?.planName),
  };
}

const provider = createProvider<CursorUsage>({
  section: "cursor",
  ttlMs: 30_000,
  async fetch() {
    const { token, accountName } = await readCursorState();
    if (!token) return { ok: false };
    const post = (url: string) =>
      fetch(url, {
        method: "POST",
        headers: cursorHeaders(token),
        body: "{}",
        cache: "no-store",
        signal: AbortSignal.timeout(10_000),
      });
    const [usageRes, planRes] = await Promise.all([
      post(USAGE_URL),
      post(PLAN_URL).catch(() => null),
    ]);
    if (usageRes.status === 401) return { ok: false, authExpired: true };
    if (!usageRes.ok) return { ok: false };
    const planJson = planRes?.ok ? await planRes.json().catch(() => null) : null;
    return { ok: true, data: parseCursorUsage(await usageRes.json(), planJson, accountName) };
  },
});

export function getCursorUsage(): Promise<ProviderResult<CursorUsage>> {
  return provider.get();
}
