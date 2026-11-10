import type { UsageWindow } from "@/lib/ranking";

export type { UsageWindow };

export interface Usage {
  rolling: UsageWindow;
  weekly: UsageWindow;
  monthly: UsageWindow;
}

export interface AccountRow {
  email: string;
  usage: Usage | null;
  prev: Usage | null;
  enabled: boolean;
  error: string | null;
  lastChangeAt: number | null;
  lastChangeDetail: string | null;
}

export interface CursorUsage {
  autoPercentUsed: number | null;
  apiPercentUsed: number | null;
  totalPercentUsed: number | null;
  billingCycleEnd: string | null;
  accountName: string | null;
  planName: string | null;
  grokPercentUsed: number | null;
  grokResetAt: string | null;
}

export interface CodexUsage {
  email: string | null;
  planType: string | null;
  usedPercent: number | null;
  limitWindowSeconds: number | null;
  resetAt: string | null;
  creditsBalance: number | null;
  limitReached: boolean;
}

export interface ClaudeUsage {
  email: string | null;
  displayName: string | null;
  signedIn: boolean;
  subscribed: boolean;
  subscriptionType: string | null;
  billingType: string | null;
  rateLimitTier: string | null;
  orgUuid: string | null;
}
