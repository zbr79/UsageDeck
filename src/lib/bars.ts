import type { ClaudeUsage, CodexUsage, WidgetProvider } from "./types";

// The usage bars each provider's card can show, in card order, with their names on the provider's
// Settings page. `Settings.hiddenBars` names the ones the user turned off; Rust keeps the same ids
// (`WidgetProvider::bars` in src-tauri/src/store.rs).

export const BARS = {
  opencode: [
    { id: "rolling", name: "Rolling (R)" },
    { id: "weekly", name: "Weekly (W)" },
    { id: "monthly", name: "Monthly (M)" },
  ],
  cursor: [
    { id: "firstParty", name: "First Party" },
    { id: "api", name: "API" },
  ],
  grok: [{ id: "weekly", name: "Weekly" }],
  codex: [
    { id: "fiveHour", name: "5-hour (5h)" },
    { id: "weekly", name: "Weekly" },
  ],
  claude: [
    { id: "fiveHour", name: "5-hour (5h)" },
    { id: "weekly", name: "Weekly" },
    // named after its model when the data has one ("Fable weekly")
    { id: "scoped", name: "Per-model weekly" },
  ],
} as const satisfies Record<WidgetProvider, readonly { id: string; name: string }[]>;

export type BarId<P extends WidgetProvider> = (typeof BARS)[P][number]["id"];

/** Bars turned off per provider. Never every bar of a provider: its own switch hides the card. */
export type HiddenBars = { [P in WidgetProvider]?: BarId<P>[] };

export function barIds<P extends WidgetProvider>(provider: P): BarId<P>[] {
  return BARS[provider].map((bar) => bar.id as BarId<P>);
}

/** The bars a Codex card has data for: one usage bar, plus the weekly one when Codex sends it. */
export function codexBars(x: CodexUsage): BarId<"codex">[] {
  return x.weeklyUsedPercent != null ? ["fiveHour", "weekly"] : ["fiveHour"];
}

/** The bars a Claude card has data for: the per-model bar only when a model has its own limit. */
export function claudeBars(c: ClaudeUsage): BarId<"claude">[] {
  return c.weeklyScoped.length > 0 ? ["fiveHour", "weekly", "scoped"] : ["fiveHour", "weekly"];
}

/**
 * The bars a card shows: those it has data for, minus the hidden ones. When that would leave none
 * (the only bar left on has no data, e.g. Codex's weekly), the card shows what it has.
 */
export function shownBars<P extends WidgetProvider>(
  provider: P,
  hidden: HiddenBars,
  available: readonly BarId<P>[] = barIds(provider),
): ReadonlySet<BarId<P>> {
  const off: readonly string[] = hidden[provider] ?? [];
  const on = available.filter((id) => !off.includes(id));
  return new Set(on.length > 0 ? on : available);
}
