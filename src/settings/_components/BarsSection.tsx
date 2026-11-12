import { BARS, type BarId, type HiddenBars } from "@/lib/bars";
import type { Settings, WidgetProvider } from "@/lib/types";
import { Toggle } from "@/ui/Toggle";
import { Section, SettingRow } from "./fields";

const LAST_BAR = "At least one bar stays on — turn the provider off to hide the card";

/**
 * One switch per usage bar of a provider's cards. The last bar on cannot be turned off (Grok Bot
 * has only one): the provider's own switch on the Widget page hides the whole card.
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
    <Section title="Bars">
      {bars.map((bar) => {
        const on = !hidden.includes(bar.id);
        const last = on && onCount === 1;
        const name = (bar.id === "scoped" && scopedName) || bar.name;
        const next = (show: boolean) =>
          show ? hidden.filter((id) => id !== bar.id) : [...hidden, bar.id];
        return (
          <SettingRow
            key={bar.id}
            title={name}
            control={(labelId) => (
              <Toggle
                labelledBy={labelId}
                checked={on}
                disabled={last}
                title={last ? LAST_BAR : undefined}
                onChange={(show) =>
                  void update(
                    { hiddenBars: { [provider]: next(show) } as HiddenBars },
                    `${name} bar ${show ? "shown" : "hidden"}`,
                  )
                }
              />
            )}
          />
        );
      })}
    </Section>
  );
}
