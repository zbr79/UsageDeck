import { useId, useState } from "react";
import Dialog from "@/ui/Dialog";
import { toastSuccess } from "@/components/toast/toast";
import { MAX_NICKNAME_LENGTH } from "@/lib/settings-schema";
import { OPENCODE_KEY_MAX_LENGTH, OPENCODE_KEY_PLACEHOLDER } from "@/lib/opencode-key";
import { saveAccount } from "../_lib/hooks";
import { keyError } from "../_lib/key-error";
import { dangerButton, inputClass, primaryButton, secondaryButton } from "./fields";

/** Rename: a pre-filled nickname box. Enter saves, Esc cancels; an empty box clears it. */
export function RenameDialog({
  name,
  nickname,
  onSave,
  onClose,
}: {
  /** what the card shows now (nickname, or else the email) */
  name: string;
  nickname: string;
  onSave: (next: string) => Promise<boolean>;
  onClose: () => void;
}) {
  const id = useId();
  const [value, setValue] = useState(nickname || name);
  const [busy, setBusy] = useState(false);
  const save = async () => {
    const next = value.trim();
    // Saving the shown email as a nickname would change nothing.
    if (next === nickname || (!nickname && next === name)) return onClose();
    setBusy(true);
    if (await onSave(next)) onClose();
    else setBusy(false);
  };
  return (
    <Dialog
      title="Rename account"
      onClose={onClose}
      onSubmit={() => void save()}
      footer={
        <>
          <button type="submit" disabled={busy} className={primaryButton}>
            Save
          </button>
          <button type="button" onClick={onClose} className={secondaryButton}>
            Cancel
          </button>
        </>
      }
    >
      <label htmlFor={id} className="mb-1 block text-sm">
        Name shown on the widget
      </label>
      <input
        id={id}
        data-autofocus
        value={value}
        maxLength={MAX_NICKNAME_LENGTH}
        onChange={(e) => setValue(e.target.value)}
        onFocus={(e) => e.currentTarget.select()}
        className={inputClass}
      />
      <p className="mt-2 text-xs text-(--p-muted)">Leave it empty to show the account email.</p>
    </Dialog>
  );
}

/** Remove: names the account; Cancel has focus, so Enter never removes by accident. */
export function RemoveDialog({
  name,
  onConfirm,
  onClose,
}: {
  name: string;
  onConfirm: () => Promise<boolean>;
  onClose: () => void;
}) {
  const [busy, setBusy] = useState(false);
  return (
    <Dialog
      title="Remove account?"
      onClose={onClose}
      footer={
        <>
          <button
            type="button"
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              if (await onConfirm()) onClose();
              else setBusy(false);
            }}
            className={dangerButton}
          >
            Remove
          </button>
          <button type="button" data-autofocus onClick={onClose} className={secondaryButton}>
            Cancel
          </button>
        </>
      }
    >
      <p>
        <span className="font-semibold break-all">{name}</span> and its API key will be removed from
        UsageDeck. You can add it again later with its key.
      </p>
    </Dialog>
  );
}

/** A password box with a show toggle and the inline key error. */
function KeyField({
  id,
  value,
  onChange,
  autoFocus,
}: {
  id: string;
  value: string;
  onChange: (next: string) => void;
  autoFocus?: boolean;
}) {
  const errorId = useId();
  const [shown, setShown] = useState(false);
  const error = keyError(value);
  return (
    <>
      <div className="flex gap-2">
        <input
          id={id}
          type={shown ? "text" : "password"}
          autoComplete="off"
          spellCheck={false}
          data-autofocus={autoFocus ? true : undefined}
          value={value}
          maxLength={OPENCODE_KEY_MAX_LENGTH + 64}
          placeholder={OPENCODE_KEY_PLACEHOLDER}
          aria-invalid={error ? true : undefined}
          aria-describedby={error ? errorId : undefined}
          onChange={(e) => onChange(e.target.value)}
          className={`${inputClass} font-mono`}
        />
        <button
          type="button"
          aria-pressed={shown}
          onClick={() => setShown(!shown)}
          className={`${secondaryButton} min-w-[72px]`}
        >
          {shown ? "Hide" : "Show"}
        </button>
      </div>
      <p id={errorId} role="status" className="mt-1 min-h-4 text-xs text-(--p-danger)">
        {error}
      </p>
    </>
  );
}

/** Add account (email + key) or Replace API key (key only). Save stays off until valid. */
export function KeyDialog({
  email,
  onClose,
  onDone,
}: {
  /** the account whose key is replaced; absent when adding an account */
  email?: string;
  onClose: () => void;
  onDone: () => void;
}) {
  const emailId = useId();
  const keyId = useId();
  const adding = email === undefined;
  const [account, setAccount] = useState("");
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const valid = key.trim() !== "" && keyError(key) === null && (!adding || account.trim() !== "");
  const save = async () => {
    if (!valid || busy) return;
    setBusy(true);
    if (await saveAccount(adding ? account.trim() : email, key.trim())) {
      toastSuccess(adding ? "Account added" : "API key replaced");
      onDone();
      onClose();
    } else setBusy(false);
  };
  return (
    <Dialog
      title={adding ? "Add OpenCode account" : "Replace API key"}
      onClose={onClose}
      onSubmit={() => void save()}
      footer={
        <>
          <button type="submit" disabled={!valid || busy} className={primaryButton}>
            {adding ? "Add account" : "Replace key"}
          </button>
          <button type="button" onClick={onClose} className={secondaryButton}>
            Cancel
          </button>
        </>
      }
    >
      {adding ? (
        <>
          <label htmlFor={emailId} className="mb-1 block text-sm">
            Email or label
          </label>
          <input
            id={emailId}
            data-autofocus
            value={account}
            maxLength={320}
            onChange={(e) => setAccount(e.target.value)}
            className={`${inputClass} mb-4`}
          />
        </>
      ) : (
        <p className="mb-4 break-all text-(--p-textSecondary)">{email}</p>
      )}
      <label htmlFor={keyId} className="mb-1 block text-sm">
        {adding ? "API key" : "New API key"}
      </label>
      <KeyField id={keyId} value={key} onChange={setKey} autoFocus={!adding} />
    </Dialog>
  );
}
