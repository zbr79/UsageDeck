"use client";

import { useId, useState, type ReactNode } from "react";
import { MAX_NICKNAME_LENGTH } from "@/lib/settings-schema";

export const inputClass =
  "rounded-md bg-(--p-inputBg) border border-(--p-inputBorder) px-2 py-1 text-sm text-(--p-text) placeholder:text-(--p-muted)";

export const primaryButton =
  "px-3 py-1.5 rounded-lg text-sm bg-(--p-primaryBg) text-(--p-primaryFg) hover:bg-(--p-primaryHover) disabled:opacity-50";

export const secondaryButton =
  "px-3 py-1.5 rounded-lg text-sm text-(--p-textSecondary) hover:bg-(--p-surfaceHover)";

/**
 * Nickname input that saves on blur or Enter, and only when the value
 * actually changed (no request, no toast otherwise).
 */
export function NicknameField({
  value,
  onSave,
}: {
  value: string;
  onSave: (next: string) => Promise<boolean>;
}) {
  const id = useId();
  const [saved, setSaved] = useState(false);
  const commit = async (raw: string) => {
    const next = raw.trim();
    if (next === value) return;
    if (await onSave(next)) {
      setSaved(true);
      setTimeout(() => setSaved(false), 1500);
    }
  };
  return (
    <div className="flex items-center gap-2">
      <label htmlFor={id} className="text-xs text-(--p-muted) shrink-0">
        Nickname
      </label>
      <input
        id={id}
        key={value}
        defaultValue={value}
        maxLength={MAX_NICKNAME_LENGTH}
        placeholder="nickname"
        onBlur={(e) => void commit(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") e.currentTarget.blur();
        }}
        className={`flex-1 min-w-0 ${inputClass}`}
      />
      <span aria-live="polite" className="text-[11px] text-(--p-success) shrink-0 w-12">
        {saved ? "✓ saved" : ""}
      </span>
    </div>
  );
}

/** A labelled settings row: title + description on the left, control right. */
export function SettingRow({
  title,
  description,
  control,
}: {
  title: string;
  description: string;
  control: (labelId: string) => ReactNode;
}) {
  const id = useId();
  return (
    <div className="flex items-center justify-between gap-4 bg-(--p-surface) border border-(--p-border) rounded-lg px-3 py-3">
      <div>
        <div id={id} className="text-sm font-medium text-(--p-text)">
          {title}
        </div>
        <div className="text-xs text-(--p-muted)">{description}</div>
      </div>
      {control(id)}
    </div>
  );
}
