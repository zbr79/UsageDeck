"use client";

import IconButton, { GearIcon } from "@/app/components/ui/IconButton";
import { SETTINGS_TYPE_SCALE } from "@/lib/theme";
import { providerLabels, providerStatus } from "./account-labels";
import type { SubProvider } from "./types";

export function providerBadgeStyle(variant: "blue" | "emerald" | "zinc"): string {
  if (variant === "emerald") return "role-badge-emerald";
  if (variant === "zinc") return "role-badge-muted";
  return "role-badge-blue";
}

export default function ProviderAccountPanel({
  provider,
  title,
  account,
  nickname,
  badgeText,
  badgeVariant,
  onOpenDetail,
}: {
  provider: SubProvider;
  title: string;
  account: string | null;
  nickname: string | undefined;
  badgeText: string | null;
  badgeVariant: "blue" | "emerald" | "zinc";
  onOpenDetail: (p: SubProvider) => void;
}) {
  const labels = providerLabels(title, account, nickname);
  const status = providerStatus(account, badgeText);
  const statusStyle = providerBadgeStyle(status === "Inactive" ? "zinc" : badgeVariant);

  return (
    <div className="rounded-lg border border-role-border bg-role-surface px-3 py-2">
      <div className="flex items-center gap-2">
        <div className="min-w-0 flex-1">
          <div className={`truncate ${SETTINGS_TYPE_SCALE.body} text-role-text`}>{labels.title}</div>
          {labels.subtitle && (
            <div className={`truncate ${SETTINGS_TYPE_SCALE.caption} text-role-muted`}>
              {labels.subtitle}
            </div>
          )}
        </div>
        {status && (
          <span
            className={`shrink-0 rounded-full border px-2 py-0.5 text-xs font-medium ${statusStyle}`}
          >
            {status}
          </span>
        )}
        <IconButton
          onClick={() => onOpenDetail(provider)}
          title={`${title} details`}
          aria-label={`${title} details`}
          className="inline-flex h-6 w-6 items-center justify-center rounded text-role-muted hover:bg-role-bg hover:text-role-text"
        >
          <GearIcon />
        </IconButton>
      </div>
    </div>
  );
}
