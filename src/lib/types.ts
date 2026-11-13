// Shared between the server (lib/, app/api) and the client (widget,
// settings). Type-only: no runtime imports allowed here.
import type { HiddenBars } from "./bars";

export type ThemeMode = "auto" | "light" | "dark";
/** "system" follows the Windows app light / dark mode. */
export type PanelTheme = "light" | "dark" | "system";

export interface Settings {
  opencodeEnabled: boolean;
  cursorEnabled: boolean;
  grokEnabled: boolean;
  codexEnabled: boolean;
  claudeEnabled: boolean;
  deepseekPeakHourWarning: boolean;
  /** hide the widget while a full-screen app is in front on its monitor */
  hideInFullscreen: boolean;
  showProviderNames: boolean;
  themeMode: ThemeMode;
  panelTheme: PanelTheme;
  /** nickname per OpenCode account email, or per provider id */
  displayNames: Record<string, string>;
  /** lower-cased OpenCode account emails hidden from the widget */
  disabledAccounts: string[];
  /** left-to-right provider order, also the top-to-bottom widget order */
  providerOrder: WidgetProvider[];
  /** start the app when the user signs in to Windows */
  startAtLogin: boolean;
  /** install a downloaded update once the user is away, then reopen */
  autoInstallUpdates: boolean;
  /** Claude Code hourly spend target in whole USD per hour (1-10000); null = off */
  claudeHourlyTarget: number | null;
  /** used-up cards hidden from the widget until they reset: account key → epoch ms */
  snoozedUntil: Record<string, number>;
  /** usage bars turned off on each provider's cards; {} shows every bar */
  hiddenBars: HiddenBars;
}

export type ProviderId = "cursor" | "grok" | "codex" | "claude";
/** Providers on the widget, including OpenCode. Order is user-controlled. */
export type WidgetProvider = "opencode" | ProviderId;

/**
 * Why the last fetch of an account failed. `null` (or a missing field from an older backend)
 * means it succeeded.
 */
export type ProviderErrorKind =
  | "missing-credentials"
  | "auth-rejected"
  | "rate-limited"
  | "network"
  | "http"
  | "schema-changed"
  | "locked"
  /** the app's sign-in is there but could not be read or decrypted */
  | "sign-in-unreadable";

/** Why the last Cursor attempt gave no fresh usage, from `/api/cursor` (`attempt`). */
export interface CursorAttemptNote {
  /** one line for the card's tooltip; null when the attempt worked */
  reason: string | null;
  /** failed attempts in a row */
  failures: number;
}

/** JSON shape of a one-account provider response. */
export interface ProviderResponse<T> {
  usage: T | null;
  stale: boolean;
  cachedAt: number | null;
  authExpired: boolean;
  error: ProviderErrorKind | null;
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
  /** why the last fetch failed; absent from older backends (treat as null) */
  error?: ProviderErrorKind | null;
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
  error: ProviderErrorKind | null;
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
  /** masked, e.g. "oc_sk_…a1b2" (or "sk-…a1b2" for an older key) */
  keyHint: string;
}

// ---- Cursor ---------------------------------------------------------------

/** On-demand usage: what Cursor bills once the included buckets are used up. */
export type CursorOnDemand =
  | { state: "disabled" }
  /** `limitUsd` null: no monthly limit */
  | { state: "enabled"; spentUsd: number; limitUsd: number | null };

/** The included-usage buckets of Cursor's Settings → Plan & Usage. */
export interface CursorUsage {
  /** "Cursor Models" (Cursor's own models, e.g. Composer and Cursor Grok) */
  cursorModelsPercentUsed: number | null;
  /** "Other Models" */
  otherModelsPercentUsed: number | null;
  /** when both buckets reset */
  resetsAt: string | null;
  /** null when Cursor did not say */
  onDemand: CursorOnDemand | null;
  accountName: string | null;
  planName: string | null;
  /** null when no API usage was read: none this cycle, or the usage events did not answer */
  apiSpend: CursorApiSpend | null;
}

/** The API bucket's dollars this billing cycle and the models they went to (usage events). */
export interface CursorApiSpend {
  spentUsd: number;
  /** the plan's included usage */
  limitUsd: number | null;
  /** largest first */
  byModel: { model: string; usd: number }[];
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
  /** when each banked reset in hand expires, soonest first; absent in older caches */
  resetExpiries?: string[];
  limitReached: boolean;
}

// ---- Claude ---------------------------------------------------------------

export interface ClaudeWindow {
  percent: number | null;
  resetsAt: string | null;
}

/** A weekly limit of one model's own, e.g. Fable (`limits[].kind == "weekly_scoped"`). */
export interface ClaudeScopedWindow extends ClaudeWindow {
  /** the model's display name, else its id, else "Model" */
  label: string;
  /** the model's use in percentage points of the overall weekly bar (not a share of the weekly use) */
  sharePctOfWeekly: number | null;
  /** how much of the weekly limit the model may use, in percent of weekly */
  allowancePctOfWeekly: number | null;
}

/** Which limits a banked reset clears. */
export type ClaudeResetKind = "full" | "fiveHour" | "weekly" | "other";

/** Banked resets of one kind and name (Claude's `cedar_ember` grants). */
export interface ClaudeResetGroup {
  kind: ClaudeResetKind;
  /** Claude's name for the grant, e.g. "Full reset" */
  label: string;
  count: number;
  /** soonest expiry of the group's grants, or null when none expires */
  earliestExpiry: string | null;
  /** paused grants are listed but not counted in `resetCredits` */
  paused: boolean;
}

export interface ClaudeResets {
  groups: ClaudeResetGroup[];
  /** the weekly free 5-hour reset: available, used this week, or not reported (null) */
  weeklyFiveHourAvailable: boolean | null;
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
  weeklyScoped: ClaudeScopedWindow[];
  /** resets in hand: unpaused banked resets plus the weekly 5-hour reset while available */
  resetCredits: number | null;
  /** null when Claude lists no resets */
  resets: ClaudeResets | null;
  billingType: string | null;
  rateLimitTier: string | null;
  orgUuid: string | null;
}

// ---- Claude spend and pace (GET /api/claude/pace) --------------------------

export type PaceState = "measuring" | "lasts" | "limit";

/** Speed and forecast of one Claude usage window. */
export interface PaceWindow {
  pct: number;
  pctPerHour: number | null;
  evenPacePct: number | null;
  /** ms epoch */
  etaToLimit: number | null;
  /** ISO */
  resetsAt: string | null;
  state: PaceState;
}

/** API-equivalent spend of Claude Code on this PC, in USD. */
export interface PaceSpend {
  lastHour: number;
  last5h: number;
  today: number;
  last7d: number;
  rate60m: number;
  rate15m: number;
  byModel: { model: string; usd24h: number }[];
  /** 24 entries, index 0 = current partial hour, USD per hour */
  hourly: number[];
  /** 96 quarter hours of USD, index 0 = the current partial quarter */
  quarters: number[];
  /** Fable's part of each quarter, indexed like `quarters` */
  quartersFable: number[];
  /** last 7 days, replies with tokens on models the price table lacks */
  unpricedReplies: number;
  /** those models' ids, sorted */
  unpricedModels: string[];
  scannedAt: number | null;
  backfilling: boolean;
}

/** One model family's API-equivalent dollars beside all dollars, over the same span. */
export interface ModelShare {
  usd: number;
  totalUsd: number;
}

export interface ClaudePace {
  spend: PaceSpend;
  windows: { fiveHour: PaceWindow | null; weekly: PaceWindow | null };
  /** Fable's dollars in the current 5-hour window; null without any */
  fableFiveHour: ModelShare | null;
  calibration: { usdPerWeeklyPct: number | null };
  coverage: "claude-code-this-pc";
}
