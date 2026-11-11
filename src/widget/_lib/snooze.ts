import { accountKey } from "@/lib/account-key";
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
