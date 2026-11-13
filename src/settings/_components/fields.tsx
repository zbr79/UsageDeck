import { useId, type ReactNode } from "react";

/** Windows 11 button and text box looks (32 px tall, 4 px radius, 14/20 text). */
const buttonBase =
  "inline-flex min-h-8 min-w-[96px] items-center justify-center gap-2 rounded px-3 text-sm disabled:opacity-50 disabled:cursor-not-allowed";

export const primaryButton = `${buttonBase} bg-(--p-primaryBg) text-(--p-primaryFg) hover:bg-(--p-primaryHover)`;

export const secondaryButton = `${buttonBase} border border-(--p-border) bg-(--s-card) text-(--p-text) hover:bg-(--p-surfaceHover)`;

export const dangerButton = `${buttonBase} bg-(--p-dangerBg) text-(--p-dangerFg) hover:opacity-90`;

export const inputClass =
  "w-full min-h-8 rounded border border-(--p-inputBorder) bg-(--p-inputBg) px-2.5 text-sm text-(--p-text) placeholder:text-(--p-muted)";

/** A scrolling page: title 28/36 semibold, content at most 1000 px wide. */
export function Page({
  title,
  lead,
  aside,
  action,
  children,
}: {
  title: string;
  /** shown before the title, e.g. the provider icon */
  lead?: ReactNode;
  /** shown under the title, e.g. "2 accounts" */
  aside?: ReactNode;
  /** right end of the header row, e.g. "Add account" */
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="mx-auto w-full max-w-[1000px] px-8 pt-7 pb-10">
      <div className="mb-6 flex min-h-12 items-center gap-3">
        {lead}
        <div className="min-w-0 flex-1">
          <h1 id="page-title" className="text-[28px] leading-9 font-semibold">
            {title}
          </h1>
          {aside && <div className="text-xs text-(--p-muted)">{aside}</div>}
        </div>
        {action}
      </div>
      {children}
    </div>
  );
}

/** A group of rows: header 14/20 semibold, 30 px above (except first) and 6 px below. */
export function Section({
  title,
  id,
  children,
}: {
  title?: string;
  /** an anchor to scroll to */
  id?: string;
  children: ReactNode;
}) {
  return (
    <section id={id} className="mt-[30px] scroll-mt-6 first:mt-0">
      {title && <h2 className="mb-1.5 text-sm font-semibold">{title}</h2>}
      <div className="flex flex-col gap-1">{children}</div>
    </section>
  );
}

/** WinUI SettingsCard: icon, title and description on the left, the control on the right. */
export function SettingRow({
  title,
  description,
  live,
  icon,
  control,
}: {
  title: string;
  description?: string;
  /** announce description changes (the result of an action started from this row) */
  live?: boolean;
  icon?: ReactNode;
  /** gets the ids of the title and description, for aria-labelledby / aria-describedby */
  control?: (labelId: string, descriptionId?: string) => ReactNode;
}) {
  const id = useId();
  const descId = useId();
  return (
    <div className="ud-row flex min-h-[68px] items-center gap-4 rounded border border-(--s-cardBorder) bg-(--s-card) p-4">
      {icon && <span className="flex size-5 shrink-0 items-center justify-center">{icon}</span>}
      <div className="min-w-0 flex-1">
        <div id={id} className="text-sm">
          {title}
        </div>
        {description && (
          <div
            id={descId}
            aria-live={live ? "polite" : undefined}
            className="text-xs text-(--p-muted)"
          >
            {description}
          </div>
        )}
      </div>
      {control && (
        <div className="flex min-w-[120px] shrink-0 items-center justify-end gap-2">
          {control(id, description ? descId : undefined)}
        </div>
      )}
    </div>
  );
}
