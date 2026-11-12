/**
 * Windows-style toggle switch (WinUI ToggleSwitch): a 40x20 pill, exposed as role="switch".
 * Forced-colors styling lives in src/settings/settings.css (`.ud-toggle`).
 */
export function Toggle({
  checked,
  onChange,
  label,
  labelledBy,
  disabled,
  title = label,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  label?: string;
  labelledBy?: string;
  disabled?: boolean;
  /** tooltip; defaults to the label */
  title?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      aria-labelledby={labelledBy}
      title={title}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`ud-toggle group relative inline-flex h-5 w-10 shrink-0 items-center rounded-full border transition-colors disabled:opacity-50 before:absolute before:-inset-y-1 before:-inset-x-0 ${
        checked
          ? "bg-(--p-switchOn) border-(--p-switchOn)"
          : "bg-transparent border-(--p-inputBorder) hover:bg-(--p-surfaceHover)"
      }`}
    >
      <span
        aria-hidden="true"
        className={`ud-toggle-thumb absolute top-1/2 -translate-y-1/2 rounded-full size-3 group-hover:size-3.5 transition-all ${
          checked ? "right-[3px] bg-white" : "left-[3px] bg-(--p-textSecondary)"
        }`}
      />
    </button>
  );
}
