import path from "path";
import os from "os";
import { createProvider, type ProviderResult } from "./provider-cache";
import { collectAccounts } from "./tracked-accounts";
import { numberOrNull, stringOrNull, toIso } from "./time";
import type { CursorUsage, TrackedAccount } from "./types";

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

interface CursorAccount {
  id: string;
  label: string | null;
  token: string;
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

/** The account Cursor is signed into now. Cursor keeps only that one token. */
export async function readCursorAccount(): Promise<CursorAccount | null> {
  const { token, accountName } = await readCursorState();
  if (!token) return null;
  const label = accountName?.trim() || null;
  return { id: (label ?? "signed-in").toLowerCase(), label, token };
}

export function cursorHeaders(token: string): Record<string, string> {
  return {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
    Accept: "application/json",
    "User-Agent": "Mozilla/5.0",
  };
}

function cursorPlanName(value: unknown): string | null {
  const plan = stringOrNull(value);
  if (!plan) return null;
  const lower = plan.toLowerCase();
  return lower.includes("pro") || /\$?\s*20(?:\.00)?\b/.test(lower) ? "Pro" : plan;
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
    planName: cursorPlanName((planJson as Record<string, any> | null)?.planInfo?.planName),
  };
}

const provider = createProvider<CursorUsage>({
  section: "cursor",
  ttlMs: 30_000,
  async fetch(key: string) {
    const account = await readCursorAccount();
    if (!account || account.id !== key) return { ok: false };
    const { token, label: accountName } = account;
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

const EMPTY: ProviderResult<CursorUsage> = {
  usage: null,
  stale: false,
  cachedAt: null,
  authExpired: false,
};

export async function getCursorAccounts(): Promise<TrackedAccount<CursorUsage>[]> {
  const account = await readCursorAccount();
  return collectAccounts(
    "cursor",
    "cursor",
    account
      ? [
          {
            id: account.id,
            label: account.label,
            active: true,
            load: () => provider.get(account.id),
          },
        ]
      : [],
  );
}

export async function getCursorUsage(): Promise<ProviderResult<CursorUsage>> {
  const account = await readCursorAccount();
  return account ? provider.get(account.id) : EMPTY;
}
