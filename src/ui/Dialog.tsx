import { useEffect, useEffectEvent, useId, useRef, type FormEvent, type ReactNode } from "react";

/**
 * A 440 px modal over the settings window (WinUI ContentDialog layout: title, body, buttons on
 * a footer band). Built on the native <dialog>: the browser traps focus, makes the page inert,
 * turns Esc into `cancel` (topmost dialog only) and restores focus on close. Clicking outside
 * does nothing. With `onSubmit` the dialog is a form, so Enter submits.
 */
export default function Dialog({
  title,
  onClose,
  onSubmit,
  footer,
  children,
}: {
  title: ReactNode;
  onClose: () => void;
  onSubmit?: () => void;
  footer: ReactNode;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const requestClose = useEffectEvent(() => onClose());

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    const previous = document.activeElement as HTMLElement | null;
    if (!dialog.open) dialog.showModal();
    // A field marked data-autofocus, else the first control.
    const target =
      dialog.querySelector<HTMLElement>("[data-autofocus]") ??
      dialog.querySelector<HTMLElement>("input, button");
    target?.focus();
    const onCancel = (e: Event) => {
      e.preventDefault(); // we unmount instead of letting the browser close it
      requestClose();
    };
    dialog.addEventListener("cancel", onCancel);
    return () => {
      dialog.removeEventListener("cancel", onCancel);
      if (dialog.open) dialog.close();
      if (previous?.isConnected) previous.focus();
    };
  }, []);

  const body = (
    <>
      <div className="px-6 pt-6 pb-5">
        <h2 id={titleId} className="mb-3 text-xl leading-7 font-semibold break-words">
          {title}
        </h2>
        <div className="text-sm text-(--p-text)">{children}</div>
      </div>
      <div className="flex justify-end gap-2 rounded-b-lg border-t border-(--p-border) bg-(--p-surface) px-6 py-4">
        {footer}
      </div>
    </>
  );
  const frame =
    "w-[440px] max-w-[calc(100vw-32px)] rounded-lg border border-(--p-border) bg-(--p-bg) shadow-2xl";

  return (
    <dialog
      ref={ref}
      aria-labelledby={titleId}
      className="m-auto overflow-visible bg-transparent p-0 text-(--p-text) backdrop:bg-black/40"
    >
      {onSubmit ? (
        <form
          className={frame}
          onSubmit={(e: FormEvent) => {
            e.preventDefault();
            onSubmit();
          }}
        >
          {body}
        </form>
      ) : (
        <div className={frame}>{body}</div>
      )}
    </dialog>
  );
}
