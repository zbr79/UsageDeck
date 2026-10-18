import { useEffect, useEffectEvent, useRef, type CSSProperties, type ReactNode } from "react";

/**
 * Modal built on the native <dialog>: the browser provides the focus trap,
 * inert background, Escape handling (topmost dialog only) and focus
 * restoration on close. Clicking the backdrop closes it.
 */
export default function Dialog({
  labelledBy,
  onClose,
  children,
  className = "",
  style,
}: {
  labelledBy: string;
  onClose: () => void;
  children: ReactNode;
  className?: string;
  style?: CSSProperties;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const requestClose = useEffectEvent(() => onClose());

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (!dialog.open) dialog.showModal();
    // Focus a field marked data-autofocus, else the dialog body itself (no
    // focus ring on the first button just because the dialog opened).
    const target = dialog.querySelector<HTMLElement>("[data-autofocus]") ?? bodyRef.current;
    target?.focus();
    const onCancel = (e: Event) => {
      e.preventDefault(); // we unmount instead of letting the browser close it
      requestClose();
    };
    dialog.addEventListener("cancel", onCancel);
    return () => {
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
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div ref={bodyRef} tabIndex={-1} className="outline-none">
        {children}
      </div>
    </dialog>
  );
}
