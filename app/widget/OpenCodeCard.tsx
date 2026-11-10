"use client";

import ProviderIcon from "@/app/components/ui/ProviderIcon";
import { isExhausted, isExhaustedWindow } from "@/lib/ranking";
import { WIDGET_TYPE_SCALE } from "@/lib/theme";
import CardShell from "./CardShell";
import { PeakHourBadge } from "./CountdownText";
import UsageBar from "./UsageBar";
import type { AccountRow } from "./types";

export default function OpenCodeCard({
  row,
  isInUse,
  withControls,
  light,
  showProviderNames,
  displayNames,
  deepseekPeakHourWarning,
  increaseAt,
  onDragStart,
  onCopyKey,
}: {
  row: AccountRow;
  isInUse: boolean;
  withControls: boolean;
  light: boolean;
  cardBg?: string;
  showProviderNames: boolean;
  displayNames: Record<string, string>;
  deepseekPeakHourWarning: boolean;
  increaseAt: Map<string, number>;
  onDragStart: (e: React.MouseEvent) => void;
  onCopyKey: (email: string) => void;
}) {
  const disabled = row.enabled === false;
  const exhausted = isExhausted(row);
  const state = disabled ? "disabled" : exhausted ? "exhausted" : isInUse ? "inUse" : "default";
  const labelText = showProviderNames
    ? "OpenCode"
    : displayNames[row.email] ?? row.email.slice(0, 4);

  return (
    <CardShell state={state} light={light}>
      <div className="flex items-center gap-1.5 mb-1.5 cursor-move" onMouseDown={onDragStart}>
        <button
          type="button"
          onClick={() => onCopyKey(row.email)}
          title="Copy key"
          aria-label={`Copy API key for ${labelText}`}
          className="shrink-0 h-6 w-6 -m-1 rounded-[3px] flex items-center justify-center hover:bg-role-surface transition-colors"
          style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
        >
          <ProviderIcon src="/opencode.ico" className="h-3 w-3 rounded-[3px]" />
        </button>
        <span className={`${WIDGET_TYPE_SCALE.title} truncate font-medium text-role-text`}>
          {labelText}
        </span>
        {exhausted ? (
          <span className="sr-only">Used up</span>
        ) : isInUse ? (
          <span className="sr-only">In use</span>
        ) : disabled ? (
          <span className="sr-only">Disabled</span>
        ) : null}
        <PeakHourBadge enabled={Boolean(deepseekPeakHourWarning && withControls)} />
        <span
          className={`ml-auto ${WIDGET_TYPE_SCALE.caption} font-semibold px-1.5 py-0.5 rounded-full border role-badge-blue`}
        >
          Go
        </span>
      </div>
      {row.error ? (
        <div className={`${WIDGET_TYPE_SCALE.body} text-role-danger`}>unavailable</div>
      ) : row.usage ? (
        <div className="flex gap-2">
          <UsageBar
            label="R"
            w={row.usage.rolling}
            dimmed={
              exhausted ||
              isExhaustedWindow(row.usage.weekly) ||
              isExhaustedWindow(row.usage.monthly)
            }
            increaseAtMs={increaseAt.get(`${row.email}:rolling`)}
            resetIso={withControls ? row.usage.rolling.resetsAt : null}
          />
          <UsageBar
            label="W"
            w={row.usage.weekly}
            dimmed={exhausted || isExhaustedWindow(row.usage.monthly)}
            increaseAtMs={increaseAt.get(`${row.email}:weekly`)}
            resetIso={withControls ? row.usage.weekly.resetsAt : null}
          />
          <UsageBar
            label="M"
            w={row.usage.monthly}
            dimmed={exhausted}
            increaseAtMs={increaseAt.get(`${row.email}:monthly`)}
            resetIso={row.usage.monthly.resetsAt}
          />
        </div>
      ) : null}
    </CardShell>
  );
}
