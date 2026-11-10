export interface AccountWindow {
  resetsAt: string;
  percent: number;
  exhausted: boolean;
}

export interface AccountRow {
  email: string;
  inactive?: boolean;
  cached?: boolean;
  weekly?: AccountWindow | null;
  monthly?: AccountWindow | null;
}

export type ProviderTab = "opencode" | "cursor" | "grok" | "codex" | "claude";
export type SubProvider = "cursor" | "grok" | "codex" | "claude";
export type MainTab = "system" | "accounts";
