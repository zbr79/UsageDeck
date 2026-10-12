"use client";

import { useEffect, useId, useState, type FormEvent } from "react";
import { Badge, IconButton, Switch } from "@/app/_ui/controls";
import Dialog from "@/app/_ui/Dialog";
import { CloseIcon, GearIcon, PencilIcon, TrashIcon } from "@/app/_ui/Icons";
import { toastSuccess } from "@/app/components/toast/toast";
import { formatDuration } from "@/lib/format";
import type { OpenCodeAccountSummary, OpenCodeAccountUsage, Settings } from "@/lib/types";
import { removeAccount, saveAccount } from "../_lib/hooks";
import { inputClass, NicknameField, primaryButton, secondaryButton } from "./fields";

type Update = (patch: Partial<Settings>, success?: string) => Promise<boolean>;

function ResetBadges({ usage, now }: { usage: OpenCodeAccountUsage | undefined; now: number }) {
  if (!usage) return null;
  const cachedNote = usage.stale
    ? ` · last known ${usage.cachedAt ? new Date(usage.cachedAt).toLocaleString() : ""}`
    : "";
  const pill = (label: string, name: string, iso: string | undefined) => {
    const ms = iso ? new Date(iso).getTime() - now : NaN;
    if (!Number.isFinite(ms) || ms <= 0) return null;
    return (
      <Badge tone={usage.stale ? "gray" : "blue"} title={`${name} resets in ${formatDuration(ms, 2)}${cachedNote}`}>
        {label} {formatDuration(ms, 1)}
      </Badge>
    );
  };
  return (
    <>
      {pill("W", "Weekly", usage.usage?.weekly.resetsAt)}
      {pill("M", "Monthly", usage.usage?.monthly.resetsAt)}
      {usage.stale && <Badge tone="gray" title={`Showing last known data${cachedNote}`}>cached</Badge>}
    </>
  );
}

function AddAccountForm({ onAdded }: { onAdded: () => void }) {
  const emailId = useId();
  const keyId = useId();
  const [email, setEmail] = useState("");
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    if (await saveAccount(email.trim(), key.trim())) {
      toastSuccess("Account added");
      setEmail("");
      setKey("");
      onAdded();
    }
    setBusy(false);
  };
  return (
    <form onSubmit={submit} className="grid grid-cols-[auto_1fr] items-center gap-2 mb-3">
      <label htmlFor={emailId} className="text-xs text-(--p-muted)">
        Email or label
      </label>
      <input id={emailId} value={email} onChange={(e) => setEmail(e.target.value)} required maxLength={320} className={inputClass} />
      <label htmlFor={keyId} className="text-xs text-(--p-muted)">
        API key
      </label>
      <input
        id={keyId}
        type="password"
        autoComplete="off"
        value={key}
        onChange={(e) => setKey(e.target.value)}
        placeholder="sk-…"
        required
        pattern="sk-\S+"
        title="An OpenCode key starting with sk-"
        className={`${inputClass} font-mono`}
      />
      <button type="submit" disabled={busy} className={`col-span-2 ${primaryButton}`}>
        {busy ? "Adding…" : "Add account"}
      </button>
    </form>
  );
}

function KeyDialog({ email, onClose, onSaved }: { email: string; onClose: () => void; onSaved: () => void }) {
  const titleId = useId();
  const inputId = useId();
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const save = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    if (await saveAccount(email, key.trim())) {
      toastSuccess("API key updated");
      onSaved();
      onClose();
    }
    setBusy(false);
  };
  return (
    <Dialog labelledBy={titleId} onClose={onClose}>
      <form onSubmit={save} className="bg-(--p-bg) rounded-xl shadow-2xl w-[420px] border border-(--p-border) p-5">
        <h3 id={titleId} className="text-base font-bold mb-4 truncate">
          Update API key · {email}
        </h3>
        <label htmlFor={inputId} className="block text-xs text-(--p-muted) mb-1">
          New API key
        </label>
        <input
          id={inputId}
          type="password"
          autoComplete="off"
          data-autofocus
          value={key}
          onChange={(e) => setKey(e.target.value)}
          placeholder="sk-…"
          required
          pattern="sk-\S+"
          className={`w-full font-mono ${inputClass}`}
        />
        <div className="flex gap-2 justify-end mt-4">
          <button type="button" onClick={onClose} className={secondaryButton}>
            Cancel
          </button>
          <button type="submit" disabled={busy || !key.trim()} className={primaryButton}>
            {busy ? "Saving…" : "Save"}
          </button>
        </div>
      </form>
    </Dialog>
  );
}

function AccountDialog({
  account,
  settings,
  update,
  onClose,
  onKeySaved,
}: {
  account: OpenCodeAccountSummary;
  settings: Settings;
  update: Update;
  onClose: () => void;
  onKeySaved: () => void;
}) {
  const titleId = useId();
  const switchId = useId();
  const [editingKey, setEditingKey] = useState(false);
  const lower = account.email.toLowerCase();
  const enabled = !settings.disabledAccounts.includes(lower);
  const toggle = (next: boolean) =>
    void update(
      {
        disabledAccounts: next
          ? settings.disabledAccounts.filter((e) => e !== lower)
          : [...settings.disabledAccounts, lower],
      },
      next ? "Account shown on widget" : "Account hidden from widget",
    );
  return (
    <Dialog labelledBy={titleId} onClose={onClose}>
      <div className="bg-(--p-bg) rounded-xl shadow-2xl w-[420px] border border-(--p-border) p-5">
        <div className="flex items-start justify-between mb-4 gap-2">
          <div className="min-w-0">
            <div className="flex items-center gap-1.5">
              <h3 id={titleId} className="text-base font-bold">
                OpenCode
              </h3>
              <Badge tone="blue">Go</Badge>
            </div>
            <div className="text-xs text-(--p-muted) truncate">{account.email}</div>
          </div>
          <IconButton label="Close" onClick={onClose}>
            <CloseIcon size={16} />
          </IconButton>
        </div>
        <div className="space-y-3">
          <div className="flex items-center gap-2">
            <Switch labelledBy={switchId} checked={enabled} onChange={toggle} />
            <span id={switchId} className="text-sm text-(--p-textSecondary)">
              Show on widget
            </span>
          </div>
          <NicknameField
            value={settings.displayNames[account.email] ?? ""}
            onSave={(name) => update({ displayNames: { [account.email]: name } }, "Nickname saved")}
          />
          <div className="flex items-center gap-2">
            <span className="text-xs text-(--p-muted) shrink-0">API key</span>
            <code className="flex-1 min-w-0 text-xs text-(--p-textSecondary) bg-(--p-surface) border border-(--p-border) rounded-md px-2 py-1">
              {account.keyHint}
            </code>
            <IconButton label="Update API key" onClick={() => setEditingKey(true)}>
              <PencilIcon />
            </IconButton>
          </div>
        </div>
      </div>
      {editingKey && (
        <KeyDialog email={account.email} onClose={() => setEditingKey(false)} onSaved={onKeySaved} />
      )}
    </Dialog>
  );
}

export default function OpenCodeAccounts({
  accounts,
  usage,
  settings,
  update,
  reload,
}: {
  accounts: OpenCodeAccountSummary[] | null;
  usage: Record<string, OpenCodeAccountUsage>;
  settings: Settings;
  update: Update;
  reload: () => Promise<void>;
}) {
  const [adding, setAdding] = useState(false);
  const [detail, setDetail] = useState<string | null>(null);
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(id);
  }, []);
  const disabled = (email: string) => settings.disabledAccounts.includes(email.toLowerCase());
  const sorted = accounts
    ? [...accounts].sort((a, b) => Number(disabled(a.email)) - Number(disabled(b.email)))
    : null;
  const detailAccount = accounts?.find((a) => a.email === detail) ?? null;

  return (
    <div>
      <div className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-medium text-(--p-textSecondary)">Accounts</h3>
        <button
          type="button"
          onClick={() => setAdding((a) => !a)}
          aria-expanded={adding}
          className={adding ? secondaryButton : primaryButton}
        >
          {adding ? "Cancel" : "Add"}
        </button>
      </div>
      {adding && (
        <AddAccountForm
          onAdded={() => {
            setAdding(false);
            void reload();
          }}
        />
      )}
      {sorted === null ? (
        <p className="text-sm text-(--p-muted)">Loading…</p>
      ) : sorted.length === 0 ? (
        <p className="text-sm text-(--p-muted)">No accounts yet. Use Add to connect an OpenCode Go key.</p>
      ) : (
        <ul className="space-y-2">
          {sorted.map((a) => (
            <li
              key={a.email}
              className={`bg-(--p-surface) border border-(--p-border) rounded-lg px-3 py-2 ${
                disabled(a.email) ? "border-dashed" : ""
              }`}
            >
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => setDetail(a.email)}
                  className="flex-1 min-w-0 text-left text-sm text-(--p-text) truncate hover:underline"
                >
                  {a.email}
                </button>
                {usage[a.email]?.subscriptionActive === false && <Badge tone="red">inactive</Badge>}
                {disabled(a.email) && <Badge tone="gray">hidden</Badge>}
                <ResetBadges usage={usage[a.email]} now={now} />
                {confirmRemove === a.email ? (
                  <span className="flex items-center gap-1 shrink-0">
                    <button
                      type="button"
                      onClick={async () => {
                        setConfirmRemove(null);
                        if (await removeAccount(a.email)) {
                          toastSuccess("Account removed");
                          void reload();
                        }
                      }}
                      className="text-[11px] px-2 py-1 rounded bg-(--p-dangerBg) text-(--p-dangerFg) hover:opacity-90"
                    >
                      Remove
                    </button>
                    <button
                      type="button"
                      autoFocus
                      onClick={() => setConfirmRemove(null)}
                      className="text-[11px] px-2 py-1 rounded text-(--p-textSecondary) hover:bg-(--p-surfaceHover)"
                    >
                      Cancel
                    </button>
                  </span>
                ) : (
                  <span className="flex items-center gap-0.5 shrink-0">
                    <IconButton label={`Details for ${a.email}`} onClick={() => setDetail(a.email)}>
                      <GearIcon />
                    </IconButton>
                    <IconButton label={`Remove ${a.email}`} danger onClick={() => setConfirmRemove(a.email)}>
                      <TrashIcon />
                    </IconButton>
                  </span>
                )}
              </div>
              <div className="mt-1 text-xs text-(--p-muted) truncate">
                {settings.displayNames[a.email] ?? "no nickname"}
              </div>
            </li>
          ))}
        </ul>
      )}
      {detailAccount && (
        <AccountDialog
          account={detailAccount}
          settings={settings}
          update={update}
          onClose={() => setDetail(null)}
          onKeySaved={() => void reload()}
        />
      )}
    </div>
  );
}
