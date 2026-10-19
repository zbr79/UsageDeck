// Pure settings rules, shared by the server (validation) and the client
// (optimistic updates). No Node imports allowed here.
import type { PanelTheme, Settings, ThemeMode, WidgetProvider } from "./types";

export const DEFAULT_PROVIDER_ORDER: readonly WidgetProvider[] = [
  "opencode",
  "cursor",
  "grok",
  "codex",
  "claude",
];

const PROVIDER_IDS = new Set<string>(DEFAULT_PROVIDER_ORDER);

/** Keep a saved order, drop unknowns and duplicates, and append any provider that was missing. */
export function normalizeProviderOrder(value: unknown): WidgetProvider[] {
  const seen = new Set<string>();
  const out: WidgetProvider[] = [];
  if (Array.isArray(value)) {
    for (const id of value) {
      if (typeof id !== "string" || !PROVIDER_IDS.has(id) || seen.has(id)) continue;
      seen.add(id);
      out.push(id as WidgetProvider);
    }
  }
  for (const id of DEFAULT_PROVIDER_ORDER) {
    if (!seen.has(id)) out.push(id);
  }
  return out;
}

/** Move the item at `from` into index `to`. Out-of-range indexes return a copy. */
export function moveItem<T>(list: readonly T[], from: number, to: number): T[] {
  if (from === to || from < 0 || to < 0 || from >= list.length || to >= list.length) {
    return [...list];
  }
  const next = [...list];
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item);
  return next;
}

const THEME_MODES: readonly ThemeMode[] = ["auto", "light", "dark"];
const PANEL_THEMES: readonly PanelTheme[] = ["light", "dark", "system"];
const BOOLEAN_KEYS = [
  "opencodeEnabled",
  "cursorEnabled",
  "grokEnabled",
  "codexEnabled",
  "claudeEnabled",
  "deepseekPeakHourWarning",
  "showProviderNames",
  "startAtLogin",
] as const;

export const MAX_NICKNAME_LENGTH = 64;
const MAX_KEY_LENGTH = 320;
const MAX_ENTRIES = 500;

export const DEFAULT_SETTINGS: Settings = {
  opencodeEnabled: true,
  cursorEnabled: true,
  grokEnabled: true,
  codexEnabled: true,
  claudeEnabled: true,
  deepseekPeakHourWarning: true,
  showProviderNames: false,
  themeMode: "auto",
  panelTheme: "dark",
  displayNames: {},
  disabledAccounts: [],
  providerOrder: [...DEFAULT_PROVIDER_ORDER],
  startAtLogin: true,
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

const FORBIDDEN_KEYS = new Set(["__proto__", "constructor", "prototype"]);

/**
 * Merge nickname changes into `current`: a non-empty name sets it, an empty
 * one removes it. Merging (rather than replacing the map) means a client
 * holding a stale copy can't wipe out nicknames it didn't touch.
 */
export function mergeDisplayNames(
  current: Record<string, string>,
  changes: unknown,
): Record<string, string> | null {
  if (!isRecord(changes)) return null;
  const out: Record<string, string> = { ...current };
  for (const [key, name] of Object.entries(changes)) {
    if (typeof name !== "string" || !key || key.length > MAX_KEY_LENGTH) continue;
    if (FORBIDDEN_KEYS.has(key)) continue;
    const trimmed = name.trim().slice(0, MAX_NICKNAME_LENGTH);
    if (trimmed) out[key] = trimmed;
    else delete out[key];
  }
  return Object.fromEntries(Object.entries(out).slice(0, MAX_ENTRIES));
}

export function sanitizeDisabledAccounts(value: unknown): string[] | null {
  if (!Array.isArray(value)) return null;
  const emails = value
    .filter((e): e is string => typeof e === "string" && e.length > 0 && e.length <= MAX_KEY_LENGTH)
    .map((e) => e.toLowerCase());
  return [...new Set(emails)].slice(0, MAX_ENTRIES);
}

/**
 * Apply a partial update. Unknown keys and wrongly-typed values are ignored,
 * so a client can only ever change what it sent, and only to valid values.
 */
export function applySettingsPatch(current: Settings, patch: unknown): Settings {
  if (!isRecord(patch)) return current;
  const next: Settings = { ...current };
  for (const key of BOOLEAN_KEYS) {
    if (typeof patch[key] === "boolean") next[key] = patch[key] as boolean;
  }
  if (patch.themeMode === "white") {
    // White was the old duplicate of Light. Keep existing settings readable
    // while converging them on the single light theme.
    next.themeMode = "light";
  } else if (THEME_MODES.includes(patch.themeMode as ThemeMode)) {
    next.themeMode = patch.themeMode as ThemeMode;
  }
  if (PANEL_THEMES.includes(patch.panelTheme as PanelTheme)) {
    next.panelTheme = patch.panelTheme as PanelTheme;
  }
  const displayNames = mergeDisplayNames(current.displayNames, patch.displayNames);
  if (displayNames) next.displayNames = displayNames;
  const disabledAccounts = sanitizeDisabledAccounts(patch.disabledAccounts);
  if (disabledAccounts) next.disabledAccounts = disabledAccounts;
  if (Array.isArray(patch.providerOrder)) {
    next.providerOrder = normalizeProviderOrder(patch.providerOrder);
  }
  return next;
}

/** Settings from whatever is on disk (missing/corrupt values → defaults). */
export function normalizeSettings(raw: unknown): Settings {
  return applySettingsPatch(DEFAULT_SETTINGS, raw);
}
