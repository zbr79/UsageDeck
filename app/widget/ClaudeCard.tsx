"use client";

import ProviderIcon from "@/app/components/ui/ProviderIcon";
import { WIDGET_TYPE_SCALE } from "@/lib/theme";
import CardShell from "./CardShell";
import UsageBar from "./UsageBar";
import type { ClaudeUsage } from "./types";

export default function ClaudeCard({
  claude,
  claudeEnabled,
  light,
  showProviderNames,
  displayNames,
  onDragStart,
}: {
  claude: ClaudeUsage | null;
  claudeEnabled: boolean;
  light: boolean;
  cardBg?: string;
  showProviderNames: boolean;
  displayNames: Record<string, string>;
  onDragStart: (e: React.MouseEvent) => void;
}) {
  if (!claudeEnabled) return null;
  const badge = !claude
    ? "n/a"
    : !claude.signedIn
      ? "signed out"
      : !claude.subscribed
        ? "Free"
        : claude.subscriptionType ?? "subscribed";
  const badgeClass =
    badge === "Free"
      ? "role-badge-emerald"
      : badge === "n/a" || badge === "signed out"
        ? "role-badge-muted"
        : "role-badge-blue";

  return (
    <CardShell state="default" light={light}>
      <div className="flex items-center gap-1.5 mb-1.5 cursor-move" onMouseDown={onDragStart}>
        <ProviderIcon src="/claude.ico" alt="claude" className="h-3 w-3 shrink-0 rounded-[3px]" />
        <span className={`${WIDGET_TYPE_SCALE.title} truncate flex-1 font-medium text-role-text`}>
          {showProviderNames ? "Claude" : displayNames["claude"] ?? "Claude"}
        </span>
        <span className={`${WIDGET_TYPE_SCALE.caption} font-semibold px-1.5 py-0.5 rounded-full border ${badgeClass}`}>
          {badge}
        </span>
      </div>
      {(!claude || !claude.signedIn) && (
        <div className={`${WIDGET_TYPE_SCALE.body} mb-1.5 text-role-muted`}>
          {!claude ? "unavailable" : "not signed in"}
        </div>
      )}
      <div className="flex gap-2">
        <UsageBar
          label="Usage"
          w={{ status: "ok", percent: 0, resetsAt: "" }}
          dimmed={!claude?.subscribed}
          increased={false}
          reset={null}
        />
      </div>
    </CardShell>
  );
}
