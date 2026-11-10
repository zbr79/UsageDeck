"use client";

import Checkbox from "@/app/components/ui/Checkbox";
import type { PanelTheme, ThemeMode } from "@/lib/settings";
import { SETTINGS_TYPE_SCALE } from "@/lib/theme";

export default function GeneralTab({
  panelTheme,
  themeMode,
  showProviderNames,
  deepseekPeakHourWarning,
  onSetPanel,
  onSetTheme,
  onToggleProviderNames,
  onToggleDeepseekPeakHourWarning,
}: {
  panelTheme: PanelTheme;
  themeMode: ThemeMode;
  showProviderNames: boolean;
  deepseekPeakHourWarning: boolean;
  onSetPanel: (theme: PanelTheme) => void;
  onSetTheme: (mode: ThemeMode) => void;
  onToggleProviderNames: (enabled: boolean) => void;
  onToggleDeepseekPeakHourWarning: (enabled: boolean) => void;
}) {
  return (
    <div className="flex-1 overflow-y-auto no-scrollbar px-5 pb-5 space-y-2">
      <div className="flex items-center justify-between bg-role-surface border border-role-border rounded-lg px-3 py-3">
        <div>
          <div className={`${SETTINGS_TYPE_SCALE.body} font-medium text-role-text`}>Panel theme</div>
          <div className={`${SETTINGS_TYPE_SCALE.caption} text-role-muted`}>
            Appearance of this settings panel
          </div>
        </div>
        <div
          role="group"
          aria-label="Panel theme"
          className="flex rounded-lg overflow-hidden border border-role-border shrink-0"
        >
          {(["light", "dark"] as const).map((t) => (
            <button
              key={t}
              type="button"
              aria-pressed={panelTheme === t}
              onClick={() => onSetPanel(t)}
              className={`px-2.5 py-1 ${SETTINGS_TYPE_SCALE.caption} capitalize transition-colors ${
                panelTheme === t
                  ? "bg-role-text text-role-bg"
                  : "bg-role-surface text-role-muted hover:text-role-text"
              }`}
            >
              {t}
            </button>
          ))}
        </div>
      </div>

      <div className="flex items-center justify-between bg-role-surface border border-role-border rounded-lg px-3 py-3">
        <div>
          <div className={`${SETTINGS_TYPE_SCALE.body} font-medium text-role-text`}>Widget theme</div>
          <div className={`${SETTINGS_TYPE_SCALE.caption} text-role-muted`}>
            Auto follows screen brightness (uses CPU); Light/Dark are static
          </div>
        </div>
        <div
          role="group"
          aria-label="Widget theme"
          className="flex rounded-lg overflow-hidden border border-role-border shrink-0"
        >
          {(["auto", "light", "white", "dark"] as const).map((m) => (
            <button
              key={m}
              type="button"
              aria-pressed={themeMode === m}
              onClick={() => onSetTheme(m)}
              className={`px-2.5 py-1 ${SETTINGS_TYPE_SCALE.caption} capitalize transition-colors ${
                themeMode === m
                  ? "bg-role-text text-role-bg"
                  : "bg-role-surface text-role-muted hover:text-role-text"
              }`}
            >
              {m}
            </button>
          ))}
        </div>
      </div>

      <div className="flex items-center justify-between bg-role-surface border border-role-border rounded-lg px-3 py-3">
        <div>
          <div className={`${SETTINGS_TYPE_SCALE.body} font-medium text-role-text`}>
            Show provider names
          </div>
          <div className={`${SETTINGS_TYPE_SCALE.caption} text-role-muted`}>
            Display provider names on widget cards instead of nicknames
          </div>
        </div>
        <Checkbox
          checked={showProviderNames}
          onChange={onToggleProviderNames}
          ariaLabel="Show provider names"
        />
      </div>

      <div className="flex items-center justify-between bg-role-surface border border-role-border rounded-lg px-3 py-3">
        <div>
          <div className={`${SETTINGS_TYPE_SCALE.body} font-medium text-role-text`}>
            DeepSeek Peak Hour Warning
          </div>
          <div className={`${SETTINGS_TYPE_SCALE.caption} text-role-muted`}>
            Show peak-hour start/end countdowns on the OpenCode card
          </div>
        </div>
        <Checkbox
          checked={deepseekPeakHourWarning}
          onChange={onToggleDeepseekPeakHourWarning}
          ariaLabel="Toggle DeepSeek Peak Hour Warning"
        />
      </div>
    </div>
  );
}
