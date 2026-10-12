"use client";

import { useRef, type KeyboardEvent, type ReactNode } from "react";
import { CheckIcon } from "./Icons";
import type { Tone } from "./palette";

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
      className={`h-[18px] w-[18px] rounded-[4px] border flex items-center justify-center shrink-0 transition-colors disabled:opacity-50 ${
        checked
          ? "bg-(--p-switchOn) border-(--p-switchOn) text-white"
          : "bg-(--p-inputBg) border-(--p-inputBorder)"
      }`}
    >
      {checked && <CheckIcon size={11} />}
    </button>
  );
}

/** Single-choice button group exposed as a radio group (arrow keys move). */
export function SegmentedControl<T extends string>({
  options,
  value,
  onChange,
  label,
}: {
  options: readonly T[];
  value: T;
  onChange: (next: T) => void;
  label: string;
}) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const onKeyDown = (e: KeyboardEvent, i: number) => {
    const delta = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
    if (!delta) return;
    e.preventDefault();
    const next = (i + delta + options.length) % options.length;
    onChange(options[next]);
    refs.current[next]?.focus();
  };
  return (
    <div role="radiogroup" aria-label={label} className="flex rounded-lg overflow-hidden border border-(--p-border) shrink-0">
      {options.map((option, i) => {
        const selected = option === value;
        return (
          <button
            key={option}
            ref={(el) => {
              refs.current[i] = el;
            }}
            type="button"
            role="radio"
            aria-checked={selected}
            tabIndex={selected ? 0 : -1}
            onClick={() => onChange(option)}
            onKeyDown={(e) => onKeyDown(e, i)}
            className={`px-2.5 py-1 text-xs capitalize transition-colors ${
              selected
                ? "bg-(--p-primaryBg) text-(--p-primaryFg)"
                : "bg-(--p-surface) text-(--p-muted) hover:text-(--p-text)"
            }`}
          >
            {option}
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
  scope = "p",
}: {
  tone: Tone;
  children: ReactNode;
  title?: string;
  scope?: "w" | "p";
}) {
  return (
    <span
      title={title}
      className="text-[10px] leading-3 font-semibold px-1.5 py-0.5 rounded-full border shrink-0 whitespace-nowrap"
      style={{
        background: `var(--${scope}-badge-${tone}-bg)`,
        color: `var(--${scope}-badge-${tone}-fg)`,
        borderColor: `var(--${scope}-badge-${tone}-border)`,
      }}
    >
      {children}
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
      className={`p-1 rounded text-(--p-muted) hover:bg-(--p-surfaceHover) shrink-0 ${
        danger ? "hover:text-(--p-danger)" : "hover:text-(--p-text)"
      }`}
    >
      {children}
    </button>
  );
}
