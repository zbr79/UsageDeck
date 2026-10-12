// Shared between the server (lib/, app/api) and the client (widget,
// settings). Type-only: no runtime imports allowed here.

export type ThemeMode = "auto" | "light" | "dark" | "white";
export type PanelTheme = "light" | "dark";

export interface Settings {
  cursorEnabled: boolean;
  grokEnabled: boolean;
  codexEnabled: boolean;
  claudeEnabled: boolean;
  deepseekPeakHourWarning: boolean;
  showProviderNames: boolean;
  themeMode: ThemeMode;
  panelTheme: PanelTheme;
  /** nickname per OpenCode account email, or per provider id */
  displayNames: Record<string, string>;
  /** lower-cased OpenCode account emails hidden from the widget */
  disabledAccounts: string[];
}

export type ProviderId = "cursor" | "grok" | "codex" | "claude";

/** JSON shape of every single-provider API route. */
export interface ProviderResponse<T> {
  usage: T | null;
  stale: boolean;
  cachedAt: number | null;
  authExpired: boolean;
  error: "failed" | null;
}

// ---- OpenCode Go ----------------------------------------------------------

export interface UsageWindow {
  status: string;
  percent: number;
  resetsAt: string;
}

export interface OpenCodeUsage {
  rolling: UsageWindow;
  weekly: UsageWindow;
  monthly: UsageWindow;
}

export interface OpenCodeAccountUsage {
  email: string;
  usage: OpenCodeUsage | null;
  /** the previous distinct snapshot, for "usage went up" arrows */
  prev: OpenCodeUsage | null;
  enabled: boolean;
  error: "failed" | null;
  stale: boolean;
  cachedAt: number | null;
  /** false when the API says the Go subscription is not active */
  subscriptionActive: boolean | null;
  lastChangeAt: number | null;
  lastChangeDetail: string | null;
}

export interface OpenCodeAccountSummary {
  email: string;
  /** masked, e.g. "sk-…a1b2" */
  keyHint: string;
}

// ---- Cursor ---------------------------------------------------------------

export interface CursorUsage {
  autoPercentUsed: number | null;
  apiPercentUsed: number | null;
  totalPercentUsed: number | null;
  billingCycleEnd: string | null;
  totalSpend: number | null;
  limit: number | null;
  bonusSpend: number | null;
  accountName: string | null;
  planName: string | null;
}

// ---- Grok Bot -------------------------------------------------------------

export interface GrokUsage {
  email: string | null;
  name: string | null;
  percentUsed: number | null;
  resetAt: string | null;
  planLabel: string | null;
  cursorPlanName: string | null;
  /** whose sign-in was used: the Grok Bot app's, or Cursor's as a fallback */
  source: "grok-bot" | "cursor";
}

// ---- Codex ----------------------------------------------------------------

export interface CodexUsage {
  email: string | null;
  planType: string | null;
  usedPercent: number | null;
  limitWindowSeconds: number | null;
  resetAt: string | null;
  creditsBalance: number | null;
  resetCredits: number | null;
  limitReached: boolean;
}

// ---- Claude ---------------------------------------------------------------

export interface ClaudeWindow {
  percent: number | null;
  resetsAt: string | null;
}

export interface ClaudeUsage {
  email: string | null;
  displayName: string | null;
  signedIn: boolean;
  subscribed: boolean;
  subscriptionType: string | null;
  planName: string | null;
  fiveHour: ClaudeWindow;
  weekly: ClaudeWindow;
  weeklyOpus: ClaudeWindow | null;
  weeklySonnet: ClaudeWindow | null;
  resetCredits: number | null;
  billingType: string | null;
  rateLimitTier: string | null;
  orgUuid: string | null;
}
