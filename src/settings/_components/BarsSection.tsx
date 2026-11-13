import { BARS, type BarId, type HiddenBars } from "@/lib/bars";
import type { Settings, WidgetProvider } from "@/lib/types";
import { Section, SettingRow } from "./fields";

const LAST_BAR = "At least one bar stays on — turn the provider off to hide the card";

/**
 * Which usage bars the provider's widget card shows: one row of chips, pressed when the bar is on.
 * The last bar on cannot be turned off (Grok Bot has only one): the provider's own switch on the
 * Widget page hides the whole card.
 */
export default function BarsSection({
  provider,
  settings,
  update,
  scopedName,
}: {
  provider: WidgetProvider;
  settings: Settings;
  update: (patch: { hiddenBars: HiddenBars }, success?: string) => Promise<boolean>;
  /** Claude's per-model bar after its model, e.g. "Fable weekly", when the data names one */
  scopedName?: string | null;
}) {
  const hidden: readonly string[] = settings.hiddenBars[provider] ?? [];
  const bars: readonly { id: BarId<WidgetProvider>; name: string }[] = BARS[provider];
  const onCount = bars.filter((bar) => !hidden.includes(bar.id)).length;
  return (
    <Section>
      <SettingRow
        title="Widget bars"
        control={(labelId) => (
          <div
            role="group"
            aria-labelledby={labelId}
            className="flex flex-wrap justify-end gap-1.5"
          >
            {bars.map((bar) => {
              const on = !hidden.includes(bar.id);
              const last = on && onCount === 1;
              const name = (bar.id === "scoped" && scopedName) || bar.name;
              const next = on ? [...hidden, bar.id] : hidden.filter((id) => id !== bar.id);
              return (
                <button
                  key={bar.id}
                  type="button"
                  aria-pressed={on}
                  disabled={last}
                  title={last ? LAST_BAR : undefined}
                  onClick={() =>
                    void update(
                      { hiddenBars: { [provider]: next } as HiddenBars },
                      `${name} bar ${on ? "hidden" : "shown"}`,
                    )
                  }
                  className={`flex min-h-8 items-center gap-1.5 rounded-full border px-3 text-sm transition-colors disabled:cursor-default ${
                    on
                      ? "border-(--p-accent) bg-[color-mix(in_srgb,var(--p-accent)_14%,transparent)] text-(--p-text)"
                      : "border-(--p-border) text-(--p-muted) hover:text-(--p-text)"
                  }`}
                >
                  <svg
                    aria-hidden="true"
                    width="12"
                    height="12"
                    viewBox="0 0 12 12"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.8"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    className={on ? "text-(--p-accent)" : "opacity-0"}
                  >
                    <path d="M2.5 6.2 5 8.5l4.5-5" />
                  </svg>
                  {name}
                </button>
              );
            })}
          </div>
        )}
      />
    </Section>
  );
}
