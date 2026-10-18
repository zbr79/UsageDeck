// Shared between the server (lib/, app/api) and the client (widget,
// settings). Type-only: no runtime imports allowed here.

export type ThemeMode = "auto" | "light" | "dark";
export type PanelTheme = "light" | "dark";

export interface Settings {
  opencodeEnabled: boolean;
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
  /** left-to-right provider order, also the top-to-bottom widget order */
  providerOrder: WidgetProvider[];
}

export type ProviderId = "cursor" | "grok" | "codex" | "claude";
/** Providers on the widget, including OpenCode. Order is user-controlled. */
export type WidgetProvider = "opencode" | ProviderId;

/** JSON shape of a one-account provider response. */
export interface ProviderResponse<T> {
  usage: T | null;
  stale: boolean;
  cachedAt: number | null;
  authExpired: boolean;
  error: "failed" | null;
}

/** One signed-in account of Cursor, Grok Bot, Codex, or Claude. */
export interface TrackedAccount<T> {
  id: string;
  /** email or display name; null when the app didn't share one */
  label: string | null;
  /** the account that app is signed into right now */
  active: boolean;
  usage: T | null;
  stale: boolean;
  cachedAt: number | null;
  authExpired: boolean;
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
  /** true when OpenCode rejected the API key (401 / 403) */
  authExpired?: boolean;
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
  /** weekly limit (`rate_limit.secondary_window`) */
  weeklyUsedPercent: number | null;
  weeklyLimitWindowSeconds: number | null;
  weeklyResetAt: string | null;
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
