import path from "path";
import os from "os";
import { readJson } from "./fs-atomic";
import { createProvider, type ProviderResult } from "./provider-cache";
import { numberOrNull, stringOrNull, toIso } from "./time";
import type { CodexUsage } from "./types";

export type { CodexUsage } from "./types";

// auth.json belongs to the Codex CLI and is only ever read here. Refreshing
// would rotate the single-use refresh token and log the CLI out.
const AUTH_FILE = path.join(os.homedir(), ".codex", "auth.json");
const USAGE_URL = "https://chatgpt.com/backend-api/wham/usage";

interface CodexAuth {
  tokens?: {
    access_token?: string;
    account_id?: string;
  };
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
  async fetch() {
    const auth = await readJson<CodexAuth>(AUTH_FILE);
    const accessToken = auth?.tokens?.access_token;
    if (!accessToken) return { ok: false };
    const accountId = auth?.tokens?.account_id;
    const res = await fetch(USAGE_URL, {
      headers: {
        Authorization: `Bearer ${accessToken}`,
        ...(accountId ? { "chatgpt-account-id": accountId } : {}),
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

export function getCodexUsage(): Promise<ProviderResult<CodexUsage>> {
  return provider.get();
}
