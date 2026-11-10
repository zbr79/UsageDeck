"use client";

import Button from "@/app/components/ui/Button";
import Field from "@/app/components/ui/Field";
import { SETTINGS_TYPE_SCALE } from "@/lib/theme";

export default function AddAccountForm({
  email,
  apiKey,
  busy,
  onChangeEmail,
  onChangeKey,
  onCancel,
  onSubmit,
}: {
  email: string;
  apiKey: string;
  busy: boolean;
  onChangeEmail: (v: string) => void;
  onChangeKey: (v: string) => void;
  onCancel: () => void;
  onSubmit: (e: React.FormEvent) => void;
}) {
  return (
    <form onSubmit={onSubmit} className="flex flex-col gap-3 pt-1" aria-label="Add account">
      <h3 className={`${SETTINGS_TYPE_SCALE.body} font-medium text-role-text`}>Add account</h3>
      <Field
        id="opencode-new-email"
        label="Email or label"
        value={email}
        onChange={(e) => onChangeEmail(e.target.value)}
        placeholder="you@example.com"
        required
        autoFocus
        className="w-full rounded-lg px-3 py-2"
      />
      <Field
        id="opencode-new-key"
        label="API key"
        value={apiKey}
        onChange={(e) => onChangeKey(e.target.value)}
        placeholder="sk-..."
        required
        mono
        className="w-full rounded-lg px-3 py-2"
      />
      <div className="flex justify-end gap-2">
        <Button
          type="button"
          variant="secondary"
          onClick={onCancel}
          className={`h-9 px-3 ${SETTINGS_TYPE_SCALE.body}`}
        >
          Cancel
        </Button>
        <Button type="submit" disabled={busy} className={`h-9 px-3 ${SETTINGS_TYPE_SCALE.body}`}>
          {busy ? "Adding…" : "Add account"}
        </Button>
      </div>
    </form>
  );
}
