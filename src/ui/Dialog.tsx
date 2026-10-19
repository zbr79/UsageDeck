import { useEffect, useEffectEvent, useRef, type CSSProperties, type ReactNode } from "react";
import { backdropReason, cancelReason } from "./close-reason";

/**
 * Modal built on the native <dialog>: the browser provides the focus trap,
 * inert background, Escape handling (topmost dialog only) and focus
 * restoration on close. Clicking the backdrop closes it. The settings overlay's
 * backdrop covers the whole work area, so any real click outside the panel
 * closes Settings (as in Electron); `onClose` gets the path, for the log.
 */
export default function Dialog({
  labelledBy,
  onClose,
  children,
  className = "",
  style,
}: {
  labelledBy: string;
  /** `reason` names the close path, for logs: "escape (...)" or "backdrop (...)". */
  onClose: (reason: string) => void;
  children: ReactNode;
  className?: string;
  style?: CSSProperties;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const requestClose = useEffectEvent((reason: string) => onClose(reason));
  const openedAt = useRef(0);
  const lastKey = useRef<string | null>(null);

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (!dialog.open) dialog.showModal();
    openedAt.current = performance.now();
    // Focus a field marked data-autofocus, else the dialog body itself (no
    // focus ring on the first button just because the dialog opened).
    const target = dialog.querySelector<HTMLElement>("[data-autofocus]") ?? bodyRef.current;
    target?.focus();
    const onKey = (e: KeyboardEvent) => {
      lastKey.current = e.key;
    };
    const onCancel = (e: Event) => {
      e.preventDefault(); // we unmount instead of letting the browser close it
      const ms = Math.round(performance.now() - openedAt.current);
      requestClose(cancelReason(e, lastKey.current, ms));
    };
    document.addEventListener("keydown", onKey, true);
    dialog.addEventListener("cancel", onCancel);
    return () => {
      document.removeEventListener("keydown", onKey, true);
      dialog.removeEventListener("cancel", onCancel);
      if (dialog.open) dialog.close();
    };
  }, []);

  return (
    <dialog
      ref={ref}
      aria-labelledby={labelledBy}
      className={`m-auto bg-transparent p-0 backdrop:bg-transparent text-(--p-text) ${className}`}
      style={style}
      onMouseDown={(e) => {
        // Only the backdrop area has the <dialog> itself as target.
        if (e.target !== e.currentTarget) return;
        const ms = Math.round(performance.now() - openedAt.current);
        onClose(backdropReason(e, ms));
      }}
    >
      <div ref={bodyRef} tabIndex={-1} className="outline-none">
        {children}
      </div>
    </dialog>
  );
}
