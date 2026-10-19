import type { ProviderId } from "./types";

/** Settings key for one account. Prefixed so it cannot collide with an OpenCode email. */
export function accountKey(provider: ProviderId, id: string): string {
  return `${provider}:${id.trim().toLowerCase()}`;
}

export function isAccountHidden(
  disabled: readonly string[],
  provider: ProviderId,
  id: string,
): boolean {
  return disabled.includes(accountKey(provider, id));
}
