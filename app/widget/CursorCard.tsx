"use client";

import ProviderIcon from "@/app/components/ui/ProviderIcon";
import { WIDGET_TYPE_SCALE } from "@/lib/theme";
import CardShell from "./CardShell";
import { ResetTicker } from "./CountdownText";
import UsageBar from "./UsageBar";
import type { CursorUsage } from "./types";

export default function CursorCard({
  cursor,
  cursorError,
  cursorEnabled,
  light,
  showProviderNames,
  displayNames,
  onDragStart,
}: {
  cursor: CursorUsage | null;
  cursorError: boolean;
  cursorEnabled: boolean;
  light: boolean;
  cardBg?: string;
  showProviderNames: boolean;
  displayNames: Record<string, string>;
  onDragStart: (e: React.MouseEvent) => void;
}) {
  if (!cursorEnabled) return null;
  if (!cursor) {
    return (
      <CardShell state="default" light={light}>
        <div className="flex items-center gap-1.5 mb-1.5 cursor-move" onMouseDown={onDragStart}>
          <ProviderIcon src="/cursor.ico" alt="cursor" className="h-3 w-3 shrink-0 rounded-[3px]" />
          <span className={`${WIDGET_TYPE_SCALE.title} truncate flex-1 font-medium text-role-text`}>
            crsr
          </span>
        </div>
        <div className={`${WIDGET_TYPE_SCALE.body} text-role-muted`}>
          {cursorError ? "not signed in or unavailable" : "…"}
        </div>
      </CardShell>
    );
  }
  const total = cursor.totalPercentUsed ?? 0;
  const exhausted = total >= 100;
  return (
    <CardShell state={exhausted ? "exhausted" : "default"} light={light}>
      <div className="flex items-center gap-1.5 mb-1.5 cursor-move" onMouseDown={onDragStart}>
        <ProviderIcon src="/cursor.ico" alt="cursor" className="h-3 w-3 shrink-0 rounded-[3px]" />
        <span
          className={`${WIDGET_TYPE_SCALE.title} truncate flex-1 font-medium text-role-text`}
          title={cursor.accountName ?? ""}
        >
          {showProviderNames ? "Cursor" : displayNames["cursor"] ?? cursor.accountName ?? "Cursor"}
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
          label="First Party"
          w={{ status: "ok", percent: cursor.autoPercentUsed ?? 0, resetsAt: "" }}
          dimmed={exhausted}
          increased={false}
          reset={null}
        />
        <UsageBar
          label="API"
          w={{ status: "ok", percent: cursor.apiPercentUsed ?? 0, resetsAt: "" }}
          dimmed={exhausted}
          increased={false}
          reset={null}
        />
      </div>
      <ResetTicker
        iso={cursor.billingCycleEnd}
        prefix="Resets "
        className={`${WIDGET_TYPE_SCALE.caption} mt-1 transition-colors duration-700 text-role-muted`}
      />
    </CardShell>
  );
}
