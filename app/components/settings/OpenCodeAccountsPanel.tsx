"use client";

import Button from "@/app/components/ui/Button";
import IconButton, { GearIcon, TrashIcon } from "@/app/components/ui/IconButton";
import { SETTINGS_TYPE_SCALE } from "@/lib/theme";
import { accountLabels, accountNote, lookupName, resetParts } from "./account-labels";
import type { AccountRow } from "./types";

const iconButtonClass =
  "inline-flex h-8 w-8 items-center justify-center rounded text-role-muted hover:bg-role-surface hover:text-role-text";

export default function OpenCodeAccountsPanel({
  accounts,
  disabledAccounts,
  displayNames,
  confirmRemove,
  now,
  onAdd,
  onOpenDetail,
  onConfirmRemove,
  onRemoveAccount,
}: {
  accounts: AccountRow[] | null;
  disabledAccounts: string[];
  displayNames: Record<string, string>;
  confirmRemove: string | null;
  now: number;
  onAdd: () => void;
  onOpenDetail: (email: string) => void;
  onConfirmRemove: (email: string | null) => void;
  onRemoveAccount: (email: string) => void;
}) {
  return (
    <div>
      <div className="mb-1 flex justify-end">
        <Button
          type="button"
          variant="secondary"
          onClick={onAdd}
          className={`h-8 px-3 ${SETTINGS_TYPE_SCALE.body}`}
        >
          Add account
        </Button>
      </div>
      {accounts === null ? (
        <div className={`${SETTINGS_TYPE_SCALE.body} text-role-muted py-3`}>Loading…</div>
      ) : accounts.length === 0 ? (
        <div className={`${SETTINGS_TYPE_SCALE.body} text-role-muted py-3`}>No accounts yet.</div>
      ) : (
        <ul>
          {accounts.map((a) => {
            const hidden = disabledAccounts.includes(a.email.toLowerCase());
            const { title, subtitle } = accountLabels(a.email, lookupName(displayNames, a.email));
            const note = accountNote(hidden, Boolean(a.cached), Boolean(a.inactive));
            const resets = resetParts(a.weekly, a.monthly, now);
            const confirming = confirmRemove === a.email;
            return (
              <li
                key={a.email}
                className="flex items-center gap-2 border-b border-role-border py-2.5 last:border-b-0"
              >
                <button
                  type="button"
                  onClick={() => onOpenDetail(a.email)}
                  className="min-w-0 flex-1 text-left"
                >
                  <span className="flex items-baseline gap-3">
                    <span
                      className={`min-w-0 truncate ${SETTINGS_TYPE_SCALE.body} ${
                        hidden ? "text-role-muted" : "text-role-text"
                      }`}
                    >
                      {title}
                    </span>
                    {resets.length > 0 && (
                      <span className={`ml-auto shrink-0 ${SETTINGS_TYPE_SCALE.caption}`}>
                        {resets.map((part, index) => (
                          <span key={part.text}>
                            {index > 0 ? " · " : ""}
                            <span className={part.full ? "text-role-danger" : "text-role-muted"}>
                              {part.text}
                            </span>
                          </span>
                        ))}
                      </span>
                    )}
                  </span>
                  {(subtitle || note) && (
                    <span className="mt-0.5 flex items-baseline gap-3">
                      {subtitle && (
                        <span className={`min-w-0 truncate ${SETTINGS_TYPE_SCALE.caption} text-role-muted`}>
                          {subtitle}
                        </span>
                      )}
                      {note && (
                        <span className={`ml-auto shrink-0 ${SETTINGS_TYPE_SCALE.caption} text-role-muted`}>
                          {note}
                        </span>
                      )}
                    </span>
                  )}
                </button>
                {confirming ? (
                  <span className="flex shrink-0 items-center gap-1">
                    <Button
                      type="button"
                      variant="ghost"
                      onClick={() => onConfirmRemove(null)}
                      className={`h-8 px-2 ${SETTINGS_TYPE_SCALE.caption}`}
                    >
                      Cancel
                    </Button>
                    <Button
                      type="button"
                      variant="danger"
                      autoFocus
                      onClick={() => {
                        onRemoveAccount(a.email);
                        onConfirmRemove(null);
                      }}
                      className={`h-8 px-2 ${SETTINGS_TYPE_SCALE.caption}`}
                    >
                      Remove
                    </Button>
                  </span>
                ) : (
                  <span className="flex shrink-0 items-center">
                    <IconButton
                      onClick={() => onOpenDetail(a.email)}
                      title="Account details"
                      aria-label={`Account details for ${a.email}`}
                      className={iconButtonClass}
                    >
                      <GearIcon />
                    </IconButton>
                    <IconButton
                      onClick={() => onConfirmRemove(a.email)}
                      title="Remove account"
                      aria-label={`Remove account ${a.email}`}
                      className={`${iconButtonClass} hover:text-role-danger`}
                    >
                      <TrashIcon />
                    </IconButton>
                  </span>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
