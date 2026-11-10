"use client";

import Checkbox from "@/app/components/ui/Checkbox";
import Dialog from "@/app/components/ui/Dialog";
import Field from "@/app/components/ui/Field";
import IconButton, { CloseIcon } from "@/app/components/ui/IconButton";
import { SETTINGS_TYPE_SCALE } from "@/lib/theme";
import { providerBadgeStyle } from "./ProviderAccountPanel";
import type { SubProvider } from "./types";

const PROVIDER_TITLE: Record<SubProvider, string> = {
  cursor: "Cursor",
  grok: "Grok Bot",
  codex: "Codex",
  claude: "Claude",
};

export default function ProviderDetailDialog({
  provider,
  enabled,
  account,
  badgeText,
  badgeVariant,
  nickname,
  savedFlash,
  onClose,
  onToggleMonitoring,
  onSaveNickname,
}: {
  provider: SubProvider;
  enabled: boolean;
  account: string | null;
  badgeText: string | null;
  badgeVariant: "blue" | "emerald" | "zinc";
  nickname: string;
  savedFlash: boolean;
  onClose: () => void;
  onToggleMonitoring: (next: boolean) => void;
  onSaveNickname: (key: string, name: string) => void;
}) {
  const title = PROVIDER_TITLE[provider];
  const titleId = `provider-detail-title-${provider}`;
  const nicknameId = `provider-detail-nickname-${provider}`;

  return (
    <Dialog onClose={onClose} labelledBy={titleId} zIndexClass="z-[70]">
      <div className="flex items-center justify-between mb-4">
        <div className="min-w-0">
          <div className="flex items-center gap-1.5">
            <h3 id={titleId} className={`${SETTINGS_TYPE_SCALE.heading} font-bold text-role-text`}>
              {title}
            </h3>
            {badgeText && (
              <span
                className={`text-[9px] font-semibold px-1.5 py-0.5 rounded-full border ${providerBadgeStyle(
                  badgeVariant,
                )}`}
              >
                {badgeText}
              </span>
            )}
            <Checkbox
              checked={enabled}
              onChange={onToggleMonitoring}
              title="Monitoring"
              ariaLabel={`Monitor ${title}`}
            />
          </div>
          <div className={`${SETTINGS_TYPE_SCALE.caption} text-role-muted truncate`}>
            {account ?? "account unavailable"}
          </div>
        </div>
        <IconButton
          onClick={onClose}
          title="Close"
          aria-label={`Close ${title} details`}
          className="p-1.5 rounded-md text-role-muted hover:text-role-text hover:bg-role-surface"
        >
          <CloseIcon />
        </IconButton>
      </div>
      <div className="space-y-3">
        <div className="flex items-center gap-2">
          <label
            htmlFor={nicknameId}
            className={`${SETTINGS_TYPE_SCALE.caption} text-role-muted shrink-0`}
          >
            Nickname
          </label>
          <Field
            id={nicknameId}
            defaultValue={nickname}
            placeholder="nickname"
            onBlur={(e) => onSaveNickname(provider, e.target.value)}
            className="flex-1 min-w-0 rounded px-2 py-1"
          />
          {savedFlash && <span className="text-[10px] text-role-success shrink-0">✓ saved</span>}
        </div>
      </div>
    </Dialog>
  );
}
