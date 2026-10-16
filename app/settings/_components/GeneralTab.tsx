"use client";

import { SegmentedControl, Switch } from "@/app/_ui/controls";
import type { PanelTheme, Settings, ThemeMode } from "@/lib/types";
import { SettingRow } from "./fields";

const PANEL_THEMES: readonly PanelTheme[] = ["light", "dark"];
const WIDGET_THEMES: readonly ThemeMode[] = ["auto", "light", "dark"];

export default function GeneralTab({
  settings,
  update,
}: {
  settings: Settings;
  update: (patch: Partial<Settings>, success?: string) => Promise<boolean>;
}) {
  return (
    <div className="space-y-2">
      <SettingRow
        title="Panel theme"
        description="Appearance of this settings panel"
        control={() => (
          <SegmentedControl
            label="Panel theme"
            options={PANEL_THEMES}
            value={settings.panelTheme}
            onChange={(panelTheme) => void update({ panelTheme })}
          />
        )}
      />
      <SettingRow
        title="Widget theme"
        description="Auto follows the screen behind the widget; the others are fixed"
        control={() => (
          <SegmentedControl
            label="Widget theme"
            options={WIDGET_THEMES}
            value={settings.themeMode}
            onChange={(themeMode) => void update({ themeMode })}
          />
        )}
      />
      <SettingRow
        title="Show provider names"
        description="Show provider names on widget cards instead of nicknames"
        control={(labelId) => (
          <Switch
            labelledBy={labelId}
            checked={settings.showProviderNames}
            onChange={(showProviderNames) =>
              void update(
                { showProviderNames },
                showProviderNames ? "Cards show provider names" : "Cards show nicknames",
              )
            }
          />
        )}
      />
      <SettingRow
        title="DeepSeek peak-hour warning"
        description="Show peak-hour start/end countdowns on the top OpenCode card"
        control={(labelId) => (
          <Switch
            labelledBy={labelId}
            checked={settings.deepseekPeakHourWarning}
            onChange={(deepseekPeakHourWarning) =>
              void update(
                { deepseekPeakHourWarning },
                deepseekPeakHourWarning ? "Peak-hour warning on" : "Peak-hour warning off",
              )
            }
          />
        )}
      />
    </div>
  );
}
