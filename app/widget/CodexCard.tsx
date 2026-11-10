"use client";

import ProviderIcon from "@/app/components/ui/ProviderIcon";
import { WIDGET_TYPE_SCALE } from "@/lib/theme";
import CardShell from "./CardShell";
import { ResetTicker } from "./CountdownText";
import UsageBar from "./UsageBar";
import type { CodexUsage } from "./types";

export default function CodexCard({
  codex,
  codexEnabled,
  light,
  showProviderNames,
  displayNames,
  onDragStart,
}: {
  codex: CodexUsage | null;
  codexEnabled: boolean;
  light: boolean;
  cardBg?: string;
  showProviderNames: boolean;
  displayNames: Record<string, string>;
  onDragStart: (e: React.MouseEvent) => void;
}) {
  if (!codexEnabled) return null;
  const pct = codex?.usedPercent ?? 0;
  const exhausted = codex?.limitReached === true || pct >= 100;
  return (
    <CardShell state={exhausted ? "exhausted" : "default"} light={light}>
      <div className="flex items-center gap-1.5 mb-1.5 cursor-move" onMouseDown={onDragStart}>
        <ProviderIcon src="/openai.png" alt="codex" className={`h-3 w-3 shrink-0 ${light ? "invert" : ""}`} />
        <span className={`${WIDGET_TYPE_SCALE.title} truncate flex-1 font-medium text-role-text`}>
          {showProviderNames ? "Codex" : displayNames["codex"] ?? "Codex"}
        </span>
        {exhausted && <span className="sr-only">Used up</span>}
        {codex?.planType && (
          <span
            className={`ml-auto ${WIDGET_TYPE_SCALE.caption} font-semibold px-1.5 py-0.5 rounded-full border ${
              codex.planType.toLowerCase() === "free" ? "role-badge-emerald" : "role-badge-blue"
            }`}
          >
            {codex.planType.charAt(0).toUpperCase() + codex.planType.slice(1)}
          </span>
        )}
      </div>
      {codex ? (
        <>
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
            iso={codex.resetAt}
            prefix="Resets "
            className={`${WIDGET_TYPE_SCALE.caption} mt-1 transition-colors duration-700 text-role-muted`}
          />
        </>
      ) : (
        <div className={`${WIDGET_TYPE_SCALE.body} text-role-muted`}>not signed in or unavailable</div>
      )}
    </CardShell>
  );
}
