import type { PanelTheme, ThemeMode } from "@/lib/settings";
import type { AccountRow, ProviderTab, SubProvider } from "./types";

export interface SettingsState {
  tab: ProviderTab;
  accounts: AccountRow[] | null;
  cursorEnabled: boolean;
  grokEnabled: boolean;
  codexEnabled: boolean;
  codexAccount: string | null;
  codexPlan: string | null;
  claudeEnabled: boolean;
  deepseekPeakHourWarning: boolean;
  claudeAccount: string | null;
  claudeStatus: string | null;
  showProviderNames: boolean;
  themeMode: ThemeMode;
  panelTheme: PanelTheme;
  subDetail: SubProvider | null;
  displayNames: Record<string, string>;
  disabledAccounts: string[];
  cursorAccount: string | null;
  cursorPlan: string | null;
  newEmail: string;
  newKey: string;
  busy: boolean;
  detailEmail: string | null;
  detailKey: string | null;
  editKey: boolean;
  newKeyValue: string;
  keyBusy: boolean;
  showAdd: boolean;
  usageAsOf: number;
  confirmRemove: string | null;
  savedFlash: string | null;
}

export const initialSettingsState: SettingsState = {
  tab: "opencode",
  accounts: null,
  cursorEnabled: true,
  grokEnabled: true,
  codexEnabled: true,
  codexAccount: null,
  codexPlan: null,
  claudeEnabled: true,
  deepseekPeakHourWarning: true,
  claudeAccount: null,
  claudeStatus: null,
  showProviderNames: false,
  themeMode: "auto",
  panelTheme: "dark",
  subDetail: null,
  displayNames: {},
  disabledAccounts: [],
  cursorAccount: null,
  cursorPlan: null,
  newEmail: "",
  newKey: "",
  busy: false,
  detailEmail: null,
  detailKey: null,
  editKey: false,
  newKeyValue: "",
  keyBusy: false,
  showAdd: false,
  usageAsOf: 0,
  confirmRemove: null,
  savedFlash: null,
};

export type SettingsAction =
  | { type: "patch"; patch: Partial<SettingsState> }
  | { type: "loaded"; patch: Partial<SettingsState> }
  | { type: "clearSavedFlash"; key: string };

export function settingsReducer(state: SettingsState, action: SettingsAction): SettingsState {
  switch (action.type) {
    case "patch":
    case "loaded":
      return { ...state, ...action.patch };
    case "clearSavedFlash":
      return state.savedFlash === action.key ? { ...state, savedFlash: null } : state;
    default:
      return state;
  }
}
