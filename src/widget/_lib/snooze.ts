import { accountKey } from "@/lib/account-key";
import { formatResetDuration } from "@/lib/format";
import type { WidgetProvider } from "@/lib/types";
import { isSnoozed, type UsedUp } from "@/lib/used-up";

/** A used-up card the user hid until it resets. */
export interface SnoozedCard {
  /** its `accountKey`, the key in `settings.snoozedUntil` */
  key: string;
  provider: WidgetProvider;
  /** epoch ms when it comes back */
  until: number;
}

/** The widget's cards, in order, with whether each is used up. */
export interface CardUsage {
  provider: WidgetProvider;
  /** account id, or the email for OpenCode */
  id: string;
  used: UsedUp | null;
}

/** The cards the widget leaves out because the user hid them until they reset. */
export function snoozedCards(
  cards: readonly CardUsage[],
  snoozedUntil: Readonly<Record<string, number>>,
  now: number,
): SnoozedCard[] {
  return cards.flatMap(({ provider, id, used }) => {
    const key = accountKey(provider, id);
    return isSnoozed(snoozedUntil, key, used, now)
      ? [{ key, provider, until: snoozedUntil[key] }]
      : [];
  });
}

const PROVIDER_NAMES: Record<WidgetProvider, string> = {
  opencode: "OpenCode",
  cursor: "Cursor",
  grok: "Grok Bot",
  codex: "Codex",
  claude: "Claude",
};

/**
 * The widget's one line for the hidden cards, "Codex hidden · back in 1h 21m" or
 * "2 hidden · next back in 1h 21m", and its spoken label. Null when nothing is hidden.
 */
export function snoozeSummary(
  cards: readonly SnoozedCard[],
  now: number,
): { text: string; label: string } | null {
  if (cards.length === 0) return null;
  const left = formatResetDuration(Math.min(...cards.map((c) => c.until)) - now);
  const names = [...new Set(cards.map((c) => PROVIDER_NAMES[c.provider]))].join(", ");
  if (cards.length === 1) {
    return {
      text: `${names} hidden · back in ${left}`,
      label: `${names} is hidden until it resets in ${left}. Show it now`,
    };
  }
  return {
    text: `${cards.length} hidden · next back in ${left}`,
    label: `${cards.length} used-up cards are hidden (${names}); the next is back in ${left}. Show them now`,
  };
}
