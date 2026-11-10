"use client";

import ProviderIcon from "@/app/components/ui/ProviderIcon";
import { WIDGET_TYPE_SCALE } from "@/lib/theme";
import CardShell from "./CardShell";
import { ResetTicker } from "./CountdownText";
import UsageBar from "./UsageBar";
import type { CursorUsage } from "./types";

export default function GrokCard({
  cursor,
  grokEnabled,
  light,
  showProviderNames,
  displayNames,
  onDragStart,
}: {
  cursor: CursorUsage | null;
  grokEnabled: boolean;
  light: boolean;
  cardBg?: string;
  showProviderNames: boolean;
  displayNames: Record<string, string>;
  onDragStart: (e: React.MouseEvent) => void;
}) {
  if (!grokEnabled) return null;
  if (cursor?.grokPercentUsed == null) return null;
  const pct = cursor.grokPercentUsed;
  const exhausted = pct >= 100;
  return (
    <CardShell state={exhausted ? "exhausted" : "default"} light={light}>
      <div className="flex items-center gap-1.5 mb-1.5 cursor-move" onMouseDown={onDragStart}>
        <ProviderIcon src="/grokbot.ico" alt="grok bot" className="h-3 w-3 shrink-0 rounded-[3px]" />
        <span className={`${WIDGET_TYPE_SCALE.title} truncate flex-1 font-medium text-role-text`}>
          {showProviderNames ? "Grok Bot" : displayNames["grok"] ?? "Grok Bot"}
        </span>
        {exhausted && <span className="sr-only">Used up</span>}
        {cursor.planName && (
          <span
            className={`ml-auto ${WIDGET_TYPE_SCALE.caption} font-semibold px-1.5 py-0.5 rounded-full border role-badge-blue`}
          >
            {cursor.planName}
          </span>
        )}
      </div>
      <div className="flex gap-2">
        <UsageBar
          label="Usage"
          w={{ status: "ok", percent: pct, resetsAt: "" }}
          dimmed={exhausted}
          increased={false}
          reset={null}
        />
      </div>
      <ResetTicker
        iso={cursor.grokResetAt}
        prefix="Resets "
        className={`${WIDGET_TYPE_SCALE.caption} mt-1 transition-colors duration-700 text-role-muted`}
      />
    </CardShell>
  );
}
