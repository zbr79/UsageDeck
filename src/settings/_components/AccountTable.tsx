import type { ReactNode } from "react";
import { formatPercent } from "@/lib/format";
import { usageSeverity } from "@/lib/pace";
import { resetCellText, resetCellTitle, type ResetRow } from "@/lib/claude-resets";
import { Badge } from "@/ui/controls";
import type { Tone } from "@/ui/palette";
import { EyeOffIcon } from "@/ui/Icons";

/** One usage window in a table cell. */
export interface UsageCell {
  percent: number | null;
  /** countdown, "3h 34m"; null when unknown */
  reset: string | null;
}

export interface TableColumn {
  id: string;
  /** header text, "5-hour", "Fable weekly" */
  name: string;
}

const FILL = { ok: "var(--w-barOk)", warn: "var(--w-barWarn)", danger: "var(--w-barDanger)" };

/** Grid columns: account, one per usage window, resets, then the "⋯" button. */
export function tableTemplate(windows: number): string {
  return `minmax(0,1.6fr) repeat(${windows}, minmax(0,1fr)) minmax(0,1.4fr) 24px`;
}

/** The accounts table: a header row, then one `AccountCell` per account rendering `AccountRow`. */
export function AccountTable({
  columns,
  children,
}: {
  columns: readonly TableColumn[];
  children: ReactNode;
}) {
  return (
    <ul className="ud-row overflow-hidden rounded border border-(--s-cardBorder) bg-(--s-card)">
      <li
        aria-hidden="true"
        className="grid items-center gap-4 px-4 py-2 text-xs text-(--p-muted)"
        style={{ gridTemplateColumns: tableTemplate(columns.length) }}
      >
        <span>Account</span>
        {columns.map((c) => (
          <span key={c.id} className="truncate">
            {c.name}
          </span>
        ))}
        <span>Resets</span>
        <span />
      </li>
      {children}
    </ul>
  );
}

function Usage({ name, cell, dimmed }: { name: string; cell: UsageCell; dimmed: boolean }) {
  const known = cell.percent != null;
  const value = known ? Math.max(0, cell.percent!) : 0;
  const severity = usageSeverity(value);
  const fill = !known || dimmed ? "var(--w-barDim)" : FILL[severity];
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <div aria-hidden="true" className="flex items-baseline justify-between gap-2 text-xs">
        <span
          className={`font-semibold tabular-nums ${
            !known || dimmed
              ? "text-(--p-muted)"
              : severity === "danger"
                ? "text-(--w-barDanger)"
                : ""
          }`}
        >
          {known ? `${formatPercent(value)}%` : "--"}
        </span>
        {known && cell.reset && (
          <span className="truncate tabular-nums text-(--p-muted)">{cell.reset}</span>
        )}
      </div>
      <div
        role="meter"
        aria-label={name}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={known ? Math.min(value, 100) : undefined}
        aria-valuetext={
          known
            ? `${formatPercent(value)}%${cell.reset ? `, resets in ${cell.reset}` : ""}`
            : "unknown"
        }
        className="h-1 overflow-hidden rounded-full bg-(--w-track)"
      >
        <div
          className="h-full rounded-full"
          style={{ width: `${known ? Math.min(value, 100) : 0}%`, background: fill }}
        />
      </div>
    </div>
  );
}

/** One account's row. `note` (sign-in problems, loading) replaces the usage cells. */
export function AccountRow({
  name,
  hint,
  plan,
  active,
  hidden,
  columns,
  cells,
  note,
  resets,
  warning,
  actions,
}: {
  name: string;
  /** the full email, as the name's tooltip */
  hint?: string;
  plan: { label: string; tone: Tone } | null;
  active: boolean;
  hidden: boolean;
  columns: readonly TableColumn[];
  cells: Partial<Record<string, UsageCell>>;
  note: string | null;
  resets: readonly ResetRow[];
  /** "re-auth", with its explanation as the tooltip */
  warning: { text: string; title: string } | null;
  actions: ReactNode;
}) {
  const resetText = resetCellText(resets);
  return (
    <div
      className={`relative grid items-center gap-4 border-t border-(--s-cardBorder) px-4 py-2.5 text-sm ${
        hidden ? "opacity-60" : ""
      }`}
      style={{ gridTemplateColumns: tableTemplate(columns.length) }}
    >
      {active && (
        <span
          aria-hidden="true"
          className="absolute top-1/2 left-0 h-5 w-[3px] -translate-y-1/2 rounded-full bg-(--p-accent)"
        />
      )}
      <div className="flex min-w-0 items-center gap-2">
        <span className="truncate font-semibold" title={hint ?? name}>
          {name}
        </span>
        {hidden && (
          <span title="Hidden from widget" className="shrink-0 text-(--p-muted)">
            <EyeOffIcon size={14} />
          </span>
        )}
        {warning && (
          <span title={warning.title} className="shrink-0 text-xs font-medium text-(--w-warn)">
            {warning.text}
          </span>
        )}
        {plan && (
          <Badge scope="w" tone={plan.tone}>
            {plan.label}
          </Badge>
        )}
      </div>
      {note ? (
        <span
          className="truncate text-xs text-(--p-muted)"
          style={{ gridColumn: `span ${columns.length}` }}
        >
          {note}
        </span>
      ) : (
        columns.map((c) => (
          <Usage
            key={c.id}
            name={c.name}
            cell={cells[c.id] ?? { percent: null, reset: null }}
            dimmed={hidden}
          />
        ))
      )}
      <span
        className={`truncate text-xs ${resetText ? "" : "text-(--p-muted)"}`}
        title={resetText ? resetCellTitle(resets) : undefined}
      >
        {resetText ? (
          <>
            <span aria-hidden="true" className="text-(--w-badge-violet-fg)">
              ↺{" "}
            </span>
            {resetText}
          </>
        ) : (
          "No resets"
        )}
      </span>
      {actions}
    </div>
  );
}
