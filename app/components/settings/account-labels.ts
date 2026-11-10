import type { AccountWindow } from "./types";

export function remainingUntil(resetsAt: string, now = Date.now()): string {
  const ms = new Date(resetsAt).getTime() - now;
  if (!Number.isFinite(ms) || ms <= 0) return "now";
  const minutes = Math.floor(ms / 60000);
  const days = Math.floor(minutes / 1440);
  const hours = Math.floor((minutes % 1440) / 60);
  const mins = minutes % 60;
  if (days > 0) return hours > 0 ? `${days}d ${hours}h` : `${days}d`;
  if (hours > 0) return mins > 0 ? `${hours}h ${mins}m` : `${hours}h`;
  return `${mins}m`;
}

export function resetParts(
  weekly: AccountWindow | null | undefined,
  monthly: AccountWindow | null | undefined,
  now: number,
): { text: string; full: boolean }[] {
  const parts: { text: string; full: boolean }[] = [];
  for (const [name, window] of [
    ["Week", weekly],
    ["Month", monthly],
  ] as const) {
    if (!window?.resetsAt) continue;
    const left = remainingUntil(window.resetsAt, now);
    parts.push({
      text: window.exhausted ? `${name} full, ${left}` : `${name} ${left}`,
      full: window.exhausted,
    });
  }
  return parts;
}

export function accountNote(hidden: boolean, cached: boolean, inactive: boolean): string | null {
  const parts: string[] = [];
  if (hidden) parts.push("Hidden");
  if (cached) parts.push("Cached");
  else if (inactive) parts.push("Inactive");
  return parts.length ? parts.join(" · ") : null;
}

export function accountLabels(
  email: string,
  nickname: string | undefined,
): { title: string; subtitle: string | null } {
  const name = nickname?.trim() ?? "";
  if (!name || name.toLowerCase() === email.toLowerCase()) {
    return { title: email, subtitle: null };
  }
  return { title: name, subtitle: email };
}

export function providerLabels(
  providerTitle: string,
  account: string | null,
  nickname: string | undefined,
): { title: string; subtitle: string | null } {
  const name = nickname?.trim() ?? "";
  if (name) {
    return {
      title: name,
      subtitle: account && account.toLowerCase() !== name.toLowerCase() ? account : null,
    };
  }
  if (account) return { title: account, subtitle: null };
  return { title: providerTitle, subtitle: null };
}

export function providerStatus(account: string | null, badgeText: string | null): string | null {
  if (!account) return "Inactive";
  if (!badgeText) return null;
  const lowered = badgeText.toLowerCase();
  if (lowered === "unavailable" || lowered === "not signed in" || lowered === "n/a") {
    return "Inactive";
  }
  return badgeText;
}

export function lookupName(displayNames: Record<string, string>, key: string): string | undefined {
  if (displayNames[key]) return displayNames[key];
  const found = Object.entries(displayNames).find(([name]) => name.toLowerCase() === key.toLowerCase());
  return found?.[1];
}
