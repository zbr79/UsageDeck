"use client";

import { useEffect, useRef, type ReactNode } from "react";

const FOCUSABLE_SELECTOR =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function useDialogFocusTrap(
  containerRef: React.RefObject<HTMLElement | null>,
  active = true,
) {
  useEffect(() => {
    if (!active) return;
    const container = containerRef.current;
    if (!container) return;

    const previousFocus =
      typeof document !== "undefined" && document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;

    const focusables = Array.from(
      container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
    );
    if (focusables.length > 0) {
      focusables[0].focus();
    } else {
      container.focus();
    }

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Tab") return;
      const nodes = Array.from(
        container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
      );
      if (nodes.length === 0) {
        e.preventDefault();
        return;
      }
      const first = nodes[0];
      const last = nodes[nodes.length - 1];
      const current = document.activeElement as HTMLElement | null;

      if (e.shiftKey) {
        if (!current || current === first || !container.contains(current)) {
          e.preventDefault();
          last.focus();
        }
      } else {
        if (!current || current === last || !container.contains(current)) {
          e.preventDefault();
          first.focus();
        }
      }
    };

    container.addEventListener("keydown", onKeyDown);
    return () => {
      container.removeEventListener("keydown", onKeyDown);
      if (previousFocus && typeof previousFocus.focus === "function") {
        previousFocus.focus();
      }
    };
  }, [containerRef, active]);
}

export default function Dialog({
  onClose,
  labelledBy,
  ariaLabel,
  zIndexClass = "z-50",
  widthClass = "w-[420px]",
  paddingClass = "p-5",
  children,
}: {
  onClose: () => void;
  labelledBy?: string;
  ariaLabel?: string;
  zIndexClass?: string;
  widthClass?: string;
  paddingClass?: string;
  children: ReactNode;
}) {
  const dialogRef = useRef<HTMLDivElement | null>(null);
  useDialogFocusTrap(dialogRef, true);

  return (
    <div
      className={`fixed inset-0 bg-transparent flex items-center justify-center p-4 ${zIndexClass}`}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={labelledBy}
        aria-label={ariaLabel}
        tabIndex={-1}
        className={`bg-role-bg rounded-xl shadow-2xl ${widthClass} border border-role-border ${paddingClass}`}
        onClick={(e) => e.stopPropagation()}
      >
        {children}
      </div>
    </div>
  );
}
