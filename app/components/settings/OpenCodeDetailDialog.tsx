"use client";

import Checkbox from "@/app/components/ui/Checkbox";
import Dialog from "@/app/components/ui/Dialog";
import Field from "@/app/components/ui/Field";
import IconButton, { CloseIcon, EditIcon } from "@/app/components/ui/IconButton";
import { SETTINGS_TYPE_SCALE } from "@/lib/theme";

function maskKey(k: string): string {
  return `${k.slice(0, Math.ceil(k.length / 2))}***`;
}

export default function OpenCodeDetailDialog({
  email,
  detailKey,
  disabled,
  nickname,
  savedFlash,
  onClose,
  onToggleAccount,
  onSaveNickname,
  onStartEditKey,
}: {
  email: string;
  detailKey: string | null;
  disabled: boolean;
  nickname: string;
  savedFlash: boolean;
  onClose: () => void;
  onToggleAccount: (email: string, enabled: boolean) => void;
  onSaveNickname: (key: string, name: string) => void;
  onStartEditKey: () => void;
}) {
  const titleId = "opencode-detail-dialog-title";
  const nicknameId = "opencode-detail-nickname";

  return (
    <Dialog onClose={onClose} labelledBy={titleId} zIndexClass="z-50">
      <div className="flex items-center justify-between mb-4">
        <div className="min-w-0">
          <div className="flex items-center gap-1.5">
            <h3 id={titleId} className={`${SETTINGS_TYPE_SCALE.heading} font-bold text-role-text`}>
              OpenCode
            </h3>
            <span className="text-[9px] font-semibold px-1.5 py-0.5 rounded-full border role-badge-blue">
              Go
            </span>
            <Checkbox
              checked={!disabled}
              onChange={() => onToggleAccount(email, disabled)}
              title={disabled ? "Enable account" : "Disable account"}
              ariaLabel={`${disabled ? "Enable" : "Disable"} account ${email}`}
            />
          </div>
          <div className={`${SETTINGS_TYPE_SCALE.caption} text-role-muted truncate`}>{email}</div>
        </div>
        <IconButton
          onClick={onClose}
          title="Close"
          aria-label="Close OpenCode account details"
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
            onBlur={(e) => onSaveNickname(email, e.target.value)}
            className="flex-1 min-w-0 rounded px-2 py-1"
          />
          {savedFlash && <span className="text-[10px] text-role-success shrink-0">✓ saved</span>}
        </div>
        <div className="flex items-center gap-2">
          <div className={`${SETTINGS_TYPE_SCALE.caption} text-role-muted shrink-0`}>API key</div>
          <div
            className={`flex-1 min-w-0 ${SETTINGS_TYPE_SCALE.caption} font-mono text-role-text bg-role-surface border border-role-border rounded-lg px-2 py-1 break-all`}
          >
            {detailKey ? maskKey(detailKey) : "…"}
          </div>
          <IconButton
            onClick={onStartEditKey}
            title="Update API key"
            aria-label={`Update API key for ${email}`}
            className="p-1 rounded text-role-muted hover:text-role-text hover:bg-role-surface"
          >
            <EditIcon />
          </IconButton>
        </div>
      </div>
    </Dialog>
  );
}
