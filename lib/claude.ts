import { promises as fs } from "fs";
import path from "path";
import os from "os";
import { readJson } from "./fs-atomic";
import { getAppKey, unseal } from "./os-crypt";
import { createProvider, type ProviderResult } from "./provider-cache";
import type { ClaudeUsage, ClaudeWindow } from "./types";

export type { ClaudeUsage, ClaudeWindow } from "./types";

// Credentials belong to Claude Code / Claude desktop. This module only ever
// reads them: refreshing would rotate the refresh token out from under the
// owning app and log it out.
const CLAUDE_JSON = path.join(os.homedir(), ".claude.json");
const CLI_CREDENTIALS = path.join(os.homedir(), ".claude", ".credentials.json");
const USAGE_URL = "https://api.anthropic.com/api/oauth/usage";

export interface ClaudeToken {
  token: string;
  subscriptionType: string | null;
  rateLimitTier: string | null;
  expiresAt: number | null;
  expired: boolean;
}

function isExpired(expiresAt: number | null): boolean {
  return expiresAt != null && expiresAt <= Date.now();
}

async function desktopConfigDirs(): Promise<string[]> {
  const dirs: string[] = [];
  if (process.env.LOCALAPPDATA) {
    const packages = path.join(process.env.LOCALAPPDATA, "Packages");
    try {
      for (const entry of await fs.readdir(packages, { withFileTypes: true })) {
        if (entry.isDirectory() && /^Claude_/i.test(entry.name)) {
          dirs.push(path.join(packages, entry.name, "LocalCache", "Roaming", "Claude"));
        }
      }
    } catch {
      // no packages dir
    }
  }
  if (process.env.APPDATA) dirs.push(path.join(process.env.APPDATA, "Claude"));
  return dirs;
}

type TokenEntries = Record<
  string,
  { token?: string; expiresAt?: number; subscriptionType?: string; rateLimitTier?: string }
>;

/**
 * Pick the best token: live Claude Code > any live > expired ones (an
 * expired token still tells us the plan, just not the usage).
 */
export function pickToken(entries: TokenEntries): ClaudeToken | null {
  const candidates: { rank: number; token: ClaudeToken }[] = [];
  for (const [name, entry] of Object.entries(entries)) {
    if (!entry?.token) continue;
    const expiresAt = typeof entry.expiresAt === "number" ? entry.expiresAt : null;
    const expired = isExpired(expiresAt);
    candidates.push({
      rank: (expired ? 2 : 0) + (name.includes("claude_code") ? 0 : 1),
      token: {
        token: entry.token,
        subscriptionType: entry.subscriptionType ?? null,
        rateLimitTier: entry.rateLimitTier ?? null,
        expiresAt,
        expired,
      },
    });
  }
  candidates.sort((a, b) => a.rank - b.rank);
  return candidates[0]?.token ?? null;
}

async function readDesktopToken(): Promise<ClaudeToken | null> {
  for (const dir of await desktopConfigDirs()) {
    const config = await readJson<Record<string, unknown>>(path.join(dir, "config.json"));
    const sealed = config?.["oauth:tokenCacheV2"] ?? config?.["oauth:tokenCache"];
    if (typeof sealed !== "string") continue;
    const key = await getAppKey(path.join(dir, "Local State"));
    if (!key) continue;
    const raw = unseal(key, sealed);
    if (!raw) continue;
    try {
      const token = pickToken(JSON.parse(raw) as TokenEntries);
      if (token) return token;
    } catch {
      // try next dir
    }
  }
  return null;
}

async function readCliToken(): Promise<ClaudeToken | null> {
  const creds = await readJson<{ claudeAiOauth?: Record<string, unknown> }>(CLI_CREDENTIALS);
  const oauth = creds?.claudeAiOauth;
  if (typeof oauth?.accessToken !== "string") return null;
  const expiresAt = typeof oauth.expiresAt === "number" ? oauth.expiresAt : null;
  return {
    token: oauth.accessToken,
    subscriptionType: typeof oauth.subscriptionType === "string" ? oauth.subscriptionType : null,
    rateLimitTier: typeof oauth.rateLimitTier === "string" ? oauth.rateLimitTier : null,
    expiresAt,
    expired: isExpired(expiresAt),
  };
}

interface ClaudeAccount {
  email: string | null;
  displayName: string | null;
  billingType: string | null;
  organizationType: string | null;
  seatTier: string | null;
  orgUuid: string | null;
}

async function readAccount(): Promise<ClaudeAccount> {
  const main = await readJson<{ oauthAccount?: Record<string, unknown> }>(CLAUDE_JSON);
  const oauth = main?.oauthAccount ?? {};
  const str = (v: unknown) => (typeof v === "string" ? v : null);
  return {
    email: str(oauth.emailAddress),
    displayName: str(oauth.displayName),
    billingType: str(oauth.billingType),
    organizationType: str(oauth.organizationType),
    seatTier: str(oauth.seatTier),
    orgUuid: str(oauth.organizationUuid),
  };
}

export function planNameOf(
  subscriptionType: string | null,
  organizationType: string | null,
): string | null {
  const raw = (subscriptionType ?? organizationType ?? "").toLowerCase();
  if (!raw) return null;
  if (raw.includes("max")) return "Max";
  if (raw.includes("pro")) return "Pro";
  if (raw.includes("free")) return "Free";
  if (raw.includes("team")) return "Team";
  if (raw.includes("enterprise")) return "Enterprise";
  return subscriptionType ?? organizationType;
}

export function windowOf(value: unknown): ClaudeWindow {
  if (!value || typeof value !== "object") return { percent: null, resetsAt: null };
  const v = value as Record<string, unknown>;
  const percent =
    typeof v.utilization === "number"
      ? v.utilization
      : typeof v.percent === "number"
        ? v.percent
        : null;
  const rawReset = v.resets_at ?? v.resetsAt;
  let resetsAt: string | null = null;
  if (typeof rawReset === "number") resetsAt = new Date(rawReset * 1000).toISOString();
  else if (typeof rawReset === "string") {
    const d = new Date(rawReset);
    if (!Number.isNaN(d.getTime())) resetsAt = d.toISOString();
  }
  return { percent, resetsAt };
}

function resetCreditsOf(json: Record<string, unknown>): number | null {
  for (const key of ["cedar_ember", "juniper_tide", "iguana_necktie", "amber_gauge"]) {
    const v = json[key] as Record<string, unknown> | undefined;
    if (!v || typeof v !== "object") continue;
    if (v.eligible === false) continue;
    if (Array.isArray(v.grants)) return v.grants.length;
    if (typeof v.available === "boolean") return v.available ? 1 : 0;
  }
  return null;
}

export function buildClaudeUsage(
  account: ClaudeAccount,
  token: ClaudeToken | null,
  usageJson: Record<string, unknown> | null,
): ClaudeUsage {
  const subscriptionType = token?.subscriptionType ?? null;
  return {
    email: account.email,
    displayName: account.displayName,
    signedIn: token != null,
    subscribed:
      subscriptionType != null ||
      (account.billingType != null && account.billingType !== "none") ||
      account.seatTier != null,
    subscriptionType,
    planName: planNameOf(subscriptionType, account.organizationType),
    fiveHour: windowOf(usageJson?.five_hour),
    weekly: windowOf(usageJson?.seven_day),
    weeklyOpus: usageJson?.seven_day_opus ? windowOf(usageJson.seven_day_opus) : null,
    weeklySonnet: usageJson?.seven_day_sonnet ? windowOf(usageJson.seven_day_sonnet) : null,
    resetCredits: usageJson ? resetCreditsOf(usageJson) : null,
    billingType: account.billingType,
    rateLimitTier: token?.rateLimitTier ?? null,
    orgUuid: account.orgUuid,
  };
}

const provider = createProvider<ClaudeUsage>({
  section: "claude",
  ttlMs: 60_000,
  async fetch() {
    const account = await readAccount();
    const desktop = await readDesktopToken();
    const cli = desktop && !desktop.expired ? null : await readCliToken();
    const token = [desktop, cli].find((t) => t && !t.expired) ?? desktop ?? cli ?? null;
    // Without live numbers, still show the account/plan we know locally;
    // that partial snapshot is never persisted as "last known usage".
    const fallback = token || account.email ? buildClaudeUsage(account, token, null) : null;
    if (!token || token.expired) {
      return { ok: false, authExpired: token?.expired === true, fallback };
    }
    const res = await fetch(USAGE_URL, {
      headers: {
        Authorization: `Bearer ${token.token}`,
        "anthropic-beta": "oauth-2025-04-20",
        "anthropic-version": "2023-06-01",
        "User-Agent": "claude-cli/2.1.293",
        Accept: "application/json",
      },
      cache: "no-store",
      signal: AbortSignal.timeout(10_000),
    }).catch(() => null);
    if (res?.status === 401) return { ok: false, authExpired: true, fallback };
    if (!res?.ok) return { ok: false, fallback };
    const json = (await res.json().catch(() => null)) as Record<string, unknown> | null;
    if (!json) return { ok: false, fallback };
    return { ok: true, data: buildClaudeUsage(account, token, json) };
  },
});

export function getClaudeUsage(): Promise<ProviderResult<ClaudeUsage>> {
  return provider.get();
}
