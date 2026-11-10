import type { ProviderId, WidgetProvider } from "./types";

/**
 * Settings key for one account. Prefixed so it cannot collide with an OpenCode email (OpenCode's
 * nicknames and hidden list use the bare email; its snoozes use this key).
 */
export function accountKey(provider: WidgetProvider, id: string): string {
  return `${provider}:${id.trim().toLowerCase()}`;
}

export function isAccountHidden(
  disabled: readonly string[],
  provider: ProviderId,
  id: string,
): boolean {
  return disabled.includes(accountKey(provider, id));
}
