import path from "path";
import os from "os";
import { readJson } from "./fs-atomic";
import { createProvider, type ProviderResult } from "./provider-cache";
import { collectAccounts } from "./tracked-accounts";
import { numberOrNull, stringOrNull, toIso } from "./time";
import type { CodexUsage, TrackedAccount } from "./types";

export type { CodexUsage } from "./types";

// auth.json belongs to the Codex CLI and is only ever read here. Refreshing
// would rotate the single-use refresh token and log the CLI out.
const AUTH_FILE = path.join(os.homedir(), ".codex", "auth.json");
const USAGE_URL = "https://chatgpt.com/backend-api/wham/usage";

interface CodexAuth {
  tokens?: {
    access_token?: string;
    id_token?: string;
    account_id?: string;
  };
}

/** Email claim from a JWT payload. The signature is not checked; this only names the account. */
export function emailFromJwt(token: string | undefined): string | null {
  const part = token?.split(".")[1];
  if (!part) return null;
  try {
    const json = JSON.parse(Buffer.from(part, "base64url").toString("utf8")) as { email?: unknown };
    return typeof json.email === "string" && json.email.trim() ? json.email.trim() : null;
  } catch {
    return null;
  }
}

interface CodexAccount {
  id: string;
  label: string | null;
  token: string;
  accountId: string | null;
}

async function readCodexAccount(): Promise<CodexAccount | null> {
  const auth = await readJson<CodexAuth>(AUTH_FILE);
  const token = auth?.tokens?.access_token;
  if (!token) return null;
  const email = emailFromJwt(auth?.tokens?.id_token);
  const accountId = auth?.tokens?.account_id?.trim() || null;
  const label = email ?? accountId;
  return { id: (label ?? "signed-in").toLowerCase(), label, token, accountId };
}

export function parseCodexUsage(json: unknown): CodexUsage {
  const j = json as Record<string, any> | null;
  const rl = j?.rate_limit;
  const primary = rl?.primary_window;
  const balance = j?.credits?.balance;
  return {
    email: stringOrNull(j?.email),
    planType: stringOrNull(j?.plan_type),
    usedPercent: numberOrNull(primary?.used_percent),
    limitWindowSeconds: numberOrNull(primary?.limit_window_seconds),
    resetAt: toIso(primary?.reset_at),
    creditsBalance: balance != null && Number.isFinite(Number(balance)) ? Number(balance) : null,
    resetCredits: numberOrNull(j?.rate_limit_reset_credits?.available_count),
    limitReached: rl?.limit_reached === true,
  };
}

const provider = createProvider<CodexUsage>({
  section: "codex",
  ttlMs: 30_000,
  async fetch(key: string) {
    const account = await readCodexAccount();
    if (!account || account.id !== key) return { ok: false };
    const res = await fetch(USAGE_URL, {
      headers: {
        Authorization: `Bearer ${account.token}`,
        ...(account.accountId ? { "chatgpt-account-id": account.accountId } : {}),
        "User-Agent": "codex-cli/0.44.0",
      },
      cache: "no-store",
      signal: AbortSignal.timeout(10_000),
    });
    if (res.status === 401 || res.status === 403) return { ok: false, authExpired: true };
    if (!res.ok) return { ok: false };
    return { ok: true, data: parseCodexUsage(await res.json()) };
  },
});

const EMPTY: ProviderResult<CodexUsage> = {
  usage: null,
  stale: false,
  cachedAt: null,
  authExpired: false,
};

export async function getCodexAccounts(): Promise<TrackedAccount<CodexUsage>[]> {
  const account = await readCodexAccount();
  return collectAccounts(
    "codex",
    "codex",
    account
      ? [
          {
            id: account.id,
            label: account.label,
            active: true,
            load: () => provider.get(account.id),
            labelFrom: (usage) => usage?.email ?? null,
          },
        ]
      : [],
  );
}

export async function getCodexUsage(): Promise<ProviderResult<CodexUsage>> {
  const account = await readCodexAccount();
  return account ? provider.get(account.id) : EMPTY;
}
