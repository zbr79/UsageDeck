import { useRef, type KeyboardEvent, type ReactNode } from "react";
import { CheckIcon } from "./Icons";
import type { Tone } from "./palette";

/**
 * Grows a small control's hit area to at least 24x24 px (WCAG 2.5.8) with a centred invisible
 * pseudo-element, so the visible size and the layout stay the same.
 */
export const HIT_24 =
  "relative before:absolute before:left-1/2 before:top-1/2 before:size-6 before:-translate-x-1/2 before:-translate-y-1/2";

/** On/off control exposed to assistive tech as a switch. */
export function Switch({
  checked,
  onChange,
  label,
  labelledBy,
  disabled,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  label?: string;
  labelledBy?: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      aria-labelledby={labelledBy}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`${HIT_24} h-[18px] w-[18px] rounded-[4px] border flex items-center justify-center shrink-0 transition-colors disabled:opacity-50 ${
        checked
          ? "bg-(--p-switchOn) border-(--p-switchOn) text-white"
          : "bg-(--p-inputBg) border-(--p-inputBorder)"
      }`}
    >
      {checked && <CheckIcon size={11} />}
    </button>
  );
}

/** Single-choice tabs exposed as a radio group (arrow keys move). */
export function SegmentedControl<T extends string>({
  options,
  value,
  onChange,
  label,
}: {
  options: readonly { value: T; label: string; title?: string }[];
  value: T;
  onChange: (next: T) => void;
  label: string;
}) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const onKeyDown = (e: KeyboardEvent, i: number) => {
    const delta =
      e.key === "ArrowRight" || e.key === "ArrowDown"
        ? 1
        : e.key === "ArrowLeft" || e.key === "ArrowUp"
          ? -1
          : 0;
    if (!delta) return;
    e.preventDefault();
    const next = (i + delta + options.length) % options.length;
    onChange(options[next].value);
    refs.current[next]?.focus();
  };
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className="flex shrink-0 gap-0.5 rounded-md border border-(--p-border) bg-(--p-surface) p-0.5"
    >
      {options.map((option, i) => {
        const selected = option.value === value;
        return (
          <button
            key={option.value}
            ref={(el) => {
              refs.current[i] = el;
            }}
            type="button"
            role="radio"
            aria-checked={selected}
            tabIndex={selected ? 0 : -1}
            title={option.title}
            onClick={() => onChange(option.value)}
            onKeyDown={(e) => onKeyDown(e, i)}
            className={`min-h-7 rounded px-3 text-sm transition-colors ${
              selected
                ? "bg-(--p-primaryBg) font-medium text-(--p-primaryFg)"
                : "text-(--p-muted) hover:text-(--p-text)"
            }`}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}

/** Small pill; `scope` picks the widget (--w-*) or panel (--p-*) palette. */
export function Badge({
  tone,
  children,
  title,
  label,
  scope = "p",
}: {
  tone: Tone;
  children: ReactNode;
  title?: string;
  /** full name for a compact pill: spoken instead of the visible text, and the tooltip */
  label?: string;
  scope?: "w" | "p";
}) {
  return (
    <span
      title={title ?? label}
      className="text-[11px] leading-3 font-semibold px-1.5 py-0.5 rounded-full border shrink-0 whitespace-nowrap"
      style={{
        background: `var(--${scope}-badge-${tone}-bg)`,
        color: `var(--${scope}-badge-${tone}-fg)`,
        borderColor: `var(--${scope}-badge-${tone}-border)`,
      }}
    >
      {label ? (
        <>
          <span aria-hidden="true">{children}</span>
          <span className="sr-only">{label}</span>
        </>
      ) : (
        children
      )}
    </span>
  );
}

export function IconButton({
  label,
  onClick,
  children,
  danger,
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className={`${HIT_24} p-1 rounded text-(--p-muted) hover:bg-(--p-surfaceHover) shrink-0 ${
        danger ? "hover:text-(--p-danger)" : "hover:text-(--p-text)"
      }`}
    >
      {children}
    </button>
  );
}
