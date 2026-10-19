import { useId, useState, type FormEvent } from "react";
import { Badge, HIT_24, IconButton, Switch } from "@/ui/controls";
import Dialog from "@/ui/Dialog";
import { CloseIcon, GearIcon, PencilIcon, PlusIcon, TrashIcon } from "@/ui/Icons";
import { toastSuccess } from "@/components/toast/toast";
import { copyKey } from "@/lib/copy-key";
import type { PeakInfo } from "@/lib/peak";
import type { OpenCodeAccountSummary, OpenCodeAccountUsage, Settings } from "@/lib/types";
import { OpenCodeCard } from "@/widget/_components/cards";
import { removeAccount, saveAccount } from "../_lib/hooks";
import { CardFooter, CardGrid, useNow } from "./CardGrid";
import { inputClass, NicknameField, primaryButton, secondaryButton } from "./fields";

type Update = (patch: Partial<Settings>, success?: string) => Promise<boolean>;

/** Settings cards show no peak-hour hint: that is a live widget cue. */
const NO_PEAK: PeakInfo = { active: false, endAt: null, nextStartAt: null };

/** A row for an account whose usage hasn't arrived yet (the card says "Unavailable"). */
function emptyRow(email: string): OpenCodeAccountUsage {
  return {
    email,
    usage: null,
    prev: null,
    enabled: true,
    error: null,
    stale: false,
    cachedAt: null,
    subscriptionActive: null,
    lastChangeAt: null,
    lastChangeDetail: null,
  };
}

/** Show or hide one OpenCode account on the widget. */
function toggleAccount(update: Update, settings: Settings, email: string, next: boolean) {
  const lower = email.toLowerCase();
  void update(
    {
      disabledAccounts: next
        ? settings.disabledAccounts.filter((e) => e !== lower)
        : [...settings.disabledAccounts, lower],
    },
    next ? "Account shown on widget" : "Account hidden from widget",
  );
}

function AddAccountDialog({ onClose, onAdded }: { onClose: () => void; onAdded: () => void }) {
  const titleId = useId();
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
      onAdded();
      onClose();
    }
    setBusy(false);
  };
  return (
    <Dialog labelledBy={titleId} onClose={onClose}>
      <form
        onSubmit={submit}
        className="bg-(--p-bg) rounded-xl shadow-2xl w-[420px] border border-(--p-border) p-5"
      >
        <h3 id={titleId} className="text-base font-bold mb-4">
          Add OpenCode account
        </h3>
        <div className="grid grid-cols-[auto_1fr] items-center gap-2">
          <label htmlFor={emailId} className="text-xs text-(--p-muted)">
            Email or label
          </label>
          <input
            id={emailId}
            data-autofocus
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            required
            maxLength={320}
            className={inputClass}
          />
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
        </div>
        <div className="flex gap-2 justify-end mt-4">
          <button type="button" onClick={onClose} className={secondaryButton}>
            Cancel
          </button>
          <button type="submit" disabled={busy} className={primaryButton}>
            {busy ? "Adding…" : "Add account"}
          </button>
        </div>
      </form>
    </Dialog>
  );
}

function KeyDialog({
  email,
  onClose,
  onSaved,
}: {
  email: string;
  onClose: () => void;
  onSaved: () => void;
}) {
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
      <form
        onSubmit={save}
        className="bg-(--p-bg) rounded-xl shadow-2xl w-[420px] border border-(--p-border) p-5"
      >
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
  const enabled = !settings.disabledAccounts.includes(account.email.toLowerCase());
  const toggle = (next: boolean) => toggleAccount(update, settings, account.email, next);
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
              Active
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
        <KeyDialog
          email={account.email}
          onClose={() => setEditingKey(false)}
          onSaved={onKeySaved}
        />
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
  const showId = useId();
  const [adding, setAdding] = useState(false);
  const [detail, setDetail] = useState<string | null>(null);
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);
  const now = useNow();
  const disabled = (email: string) => settings.disabledAccounts.includes(email.toLowerCase());
  const sorted = accounts
    ? [...accounts].sort((a, b) => Number(disabled(a.email)) - Number(disabled(b.email)))
    : null;
  const detailAccount = accounts?.find((a) => a.email === detail) ?? null;

  return (
    <div>
      <div className="flex items-center justify-between mb-3">
        <div className="flex items-center gap-2">
          <Switch
            labelledBy={showId}
            checked={settings.opencodeEnabled}
            onChange={(next) =>
              void update(
                { opencodeEnabled: next },
                next ? "OpenCode shown on widget" : "OpenCode hidden from widget",
              )
            }
          />
          <span id={showId} className="text-sm text-(--p-textSecondary)">
            Active
          </span>
        </div>
        <button
          type="button"
          aria-label="Add OpenCode account"
          title="Add OpenCode account"
          onClick={() => setAdding(true)}
          className="h-7 w-7 rounded border border-(--p-border) bg-(--p-surface) text-(--p-muted) hover:bg-(--p-surfaceHover) hover:text-(--p-text) flex items-center justify-center shrink-0"
        >
          <PlusIcon size={16} />
        </button>
      </div>
      {adding && (
        <AddAccountDialog
          onClose={() => setAdding(false)}
          onAdded={() => {
            setAdding(false);
            void reload();
          }}
        />
      )}
      {sorted === null ? (
        <p className="text-sm text-(--p-muted)">Loading…</p>
      ) : sorted.length === 0 ? (
        <p className="text-sm text-(--p-muted)">
          No accounts yet. Use Add to connect an OpenCode Go key.
        </p>
      ) : (
        <CardGrid>
          {sorted.map((a) => {
            const row = usage[a.email] ?? emptyRow(a.email);
            const off = disabled(a.email);
            const lapsed = row.subscriptionActive === false;
            // Cached usage looks like fresh usage; the note names only what needs action.
            const note = lapsed
              ? "Subscription inactive"
              : off
                ? null // the "hidden" badge says it
                : "On widget";
            const footer = (
              <CardFooter
                note={note}
                hidden={off}
                lead={
                  <Switch
                    label={`Show ${a.email} on the widget`}
                    checked={!off}
                    onChange={(next) => toggleAccount(update, settings, a.email, next)}
                  />
                }
              >
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
                      className={`${HIT_24} text-[11px] px-1.5 py-0.5 rounded bg-(--p-dangerBg) text-(--p-dangerFg) hover:opacity-90`}
                    >
                      Remove
                    </button>
                    <button
                      type="button"
                      autoFocus
                      onClick={() => setConfirmRemove(null)}
                      className={`${HIT_24} text-[11px] px-1.5 py-0.5 rounded text-(--p-textSecondary) hover:bg-(--p-surfaceHover)`}
                    >
                      Cancel
                    </button>
                  </span>
                ) : (
                  <span className="flex items-center gap-0.5 shrink-0">
                    <IconButton label={`Details for ${a.email}`} onClick={() => setDetail(a.email)}>
                      <GearIcon size={12} />
                    </IconButton>
                    <IconButton
                      label={`Remove ${a.email}`}
                      danger
                      onClick={() => setConfirmRemove(a.email)}
                    >
                      <TrashIcon size={12} />
                    </IconButton>
                  </span>
                )}
              </CardFooter>
            );
            return (
              <li key={a.email} className="min-w-0">
                <OpenCodeCard
                  row={row}
                  active={!off && !lapsed}
                  primary
                  settings={settings}
                  peak={NO_PEAK}
                  now={now}
                  increased={() => false}
                  onCopyKey={() => void copyKey(a.email)}
                  extras={{ dimmed: lapsed, dashed: off, footer }}
                  fullEmail
                />
              </li>
            );
          })}
        </CardGrid>
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
