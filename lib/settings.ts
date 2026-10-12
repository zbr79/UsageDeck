import { dataFile } from "./paths";
import { readJson, withLock, writeJsonAtomic } from "./fs-atomic";
import { normalizeSettings } from "./settings-schema";
import type { Settings } from "./types";

export type { PanelTheme, Settings, ThemeMode } from "./types";
export {
  applySettingsPatch,
  DEFAULT_SETTINGS,
  MAX_NICKNAME_LENGTH,
  mergeDisplayNames,
  normalizeSettings,
  sanitizeDisabledAccounts,
} from "./settings-schema";

const FILE_NAME = "settings.json";

export async function getSettings(): Promise<Settings> {
  return normalizeSettings(await readJson(dataFile(FILE_NAME)));
}

/** Read-modify-write under a lock so concurrent saves can't lose updates. */
export function updateSettings(change: (current: Settings) => Settings): Promise<Settings> {
  return withLock(FILE_NAME, async () => {
    const next = change(await getSettings());
    await writeJsonAtomic(dataFile(FILE_NAME), next);
    return next;
  });
}

/** Drop per-account settings when an OpenCode account is removed. */
export function forgetAccount(email: string): Promise<Settings> {
  const lower = email.toLowerCase();
  return updateSettings((s) => ({
    ...s,
    displayNames: Object.fromEntries(
      Object.entries(s.displayNames).filter(([k]) => k.toLowerCase() !== lower),
    ),
    disabledAccounts: s.disabledAccounts.filter((e) => e !== lower),
  }));
}
