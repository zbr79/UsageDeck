"use client";

import Button from "@/app/components/ui/Button";
import Dialog from "@/app/components/ui/Dialog";
import Field from "@/app/components/ui/Field";
import { SETTINGS_TYPE_SCALE } from "@/lib/theme";

export default function UpdateKeyDialog({
  email,
  value,
  busy,
  onChange,
  onCancel,
  onSave,
}: {
  email: string;
  value: string;
  busy: boolean;
  onChange: (v: string) => void;
  onCancel: () => void;
  onSave: () => void;
}) {
  const titleId = "update-key-dialog-title";

  return (
    <Dialog onClose={onCancel} labelledBy={titleId} zIndexClass="z-[60]">
      <h3
        id={titleId}
        className={`${SETTINGS_TYPE_SCALE.heading} font-bold text-role-text mb-4`}
      >
        Update API key · {email}
      </h3>
      <div className="flex flex-col gap-3">
        <Field
          id="update-key-input"
          label="New API key"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder="new sk-... key"
          mono
          className="w-full rounded px-2 py-1"
        />
        <div className="flex gap-2 justify-end">
          <Button
            variant="ghost"
            onClick={onCancel}
            className={`px-3 py-1.5 ${SETTINGS_TYPE_SCALE.body}`}
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={onSave}
            disabled={busy || !value.trim()}
            className={`px-3 py-1.5 ${SETTINGS_TYPE_SCALE.body}`}
          >
            {busy ? "Saving…" : "Save"}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
