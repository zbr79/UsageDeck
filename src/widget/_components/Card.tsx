import type { CSSProperties, ReactNode } from "react";
import { formatPercent } from "@/lib/format";

export type CardState = "normal" | "active" | "exhausted";

/** Settings-only additions to a card. The widget passes none, so it renders exactly as before. */
export interface CardExtras {
  /** highlight as the signed-in account (an exhausted card keeps its red ring) */
  active?: boolean;
  /** an OpenCode account whose subscription lapsed: dim the meters */
  dimmed?: boolean;
  /** hidden from the widget: dashed border */
  dashed?: boolean;
  /** a row under the meters for notes and controls */
  footer?: ReactNode;
}

/**
 * A dimmed card's meters read the palette's dim tokens instead of fading with opacity, so text
 * keeps 4.5:1 and meter fills 3:1 against the card (tests/palette.test.ts).
 */
const DIM_VARS = {
  "--w-text": "var(--w-dimText)",
  "--w-label": "var(--w-dimText)",
  "--w-muted": "var(--w-dimMuted)",
  "--w-peakNext": "var(--w-dimText)",
  "--w-barOk": "var(--w-dimBarOk)",
  "--w-barWarn": "var(--w-dimBarWarn)",
  "--w-barDanger": "var(--w-dimBarDanger)",
} as CSSProperties;

/** Shared shell for every provider card on the widget. */
export function Card({
  label,
  icon,
  title,
  titleHint,
  hints,
  badges,
  state = "normal",
  extras,
  children,
}: {
  /** accessible name for the card region */
  label: string;
  icon: ReactNode;
  title: string;
  titleHint?: string;
  hints?: ReactNode;
  badges?: ReactNode;
  state?: CardState;
  extras?: CardExtras;
  children: ReactNode;
}) {
  const shown: CardState = state === "normal" && extras?.active ? "active" : state;
  const ring =
    shown === "exhausted"
      ? "border-(--w-ringDanger) shadow-[0_0_14px_rgba(239,68,68,0.45)]"
      : shown === "active"
        ? "border-(--w-ringOk) shadow-[0_0_14px_rgba(16,185,129,0.45)]"
        : "border-(--w-cardBorder)";
  return (
    <section
      aria-label={label}
      className={`relative rounded-lg p-2 border bg-(--w-card) transition-colors duration-700 ${ring}${extras?.dashed ? " border-dashed" : ""}`}
    >
      <div className="flex items-center gap-1.5 mb-1.5 min-w-0">
        {icon}
        <span
          className="text-[12px] leading-4 font-semibold truncate min-w-0 text-(--w-text) transition-colors duration-700"
          title={titleHint}
        >
          {title}
        </span>
        {hints}
        <span className="ml-auto flex items-center gap-1 shrink-0">{badges}</span>
      </div>
      {extras?.dimmed ? <div style={DIM_VARS}>{children}</div> : children}
      {extras?.footer}
    </section>
  );
}

export function ProviderIcon({ src, alt, invert }: { src: string; alt: string; invert?: boolean }) {
  return (
    <img
      src={src}
      alt={alt}
      className={`h-3 w-3 shrink-0 rounded-[3px] ${invert ? "invert" : ""}`}
    />
  );
}

export function Hint({
  children,
  title,
  color = "muted",
}: {
  children: ReactNode;
  title?: string;
  color?: "muted" | "warn" | "peakActive" | "peakNext";
}) {
  return (
    <span
      title={title}
      className="text-[11px] leading-4 font-medium whitespace-nowrap transition-colors duration-700"
      style={{ color: `var(--w-${color})` }}
    >
      {children}
    </span>
  );
}

/** Usage meter: label, percent, bar, optional reset countdown. */
export function Meter({
  label,
  name,
  percent,
  unknown = false,
  dimmed = false,
  increased = false,
  reset,
}: {
  label: string;
  /** full name, spoken and shown as the tooltip, e.g. "Rolling 5-hour window" */
  name: string;
  percent: number | null;
  unknown?: boolean;
  dimmed?: boolean;
  increased?: boolean;
  reset?: string | null;
}) {
  const known = !unknown && percent != null;
  const value = known ? Math.max(0, percent) : 0;
  const fill =
    !known || dimmed
      ? "var(--w-barDim)"
      : value >= 100
        ? "var(--w-barDanger)"
        : value >= 80
          ? "var(--w-barWarn)"
          : "var(--w-barOk)";
  const valueText = known
    ? `${formatPercent(value)}%${reset ? `, resets in ${reset}` : ""}`
    : "unknown";
  return (
    <div className="flex-1 min-w-0" title={name}>
      {/* the meter below speaks the full name, value and reset, so the compact text is hidden */}
      <div
        aria-hidden="true"
        className="flex items-baseline justify-between gap-1 text-[11px] leading-4 mb-0.5"
      >
        <span className="truncate min-w-0 text-(--w-label) transition-colors duration-700">
          {label}
        </span>
        <span
          className={`shrink-0 text-[12px] font-semibold tabular-nums transition-colors duration-700 ${known && !dimmed ? "text-(--w-text)" : "text-(--w-muted)"}`}
        >
          {known ? (
            <>
              {increased && !dimmed && (
                <span className="arrow-pop text-(--w-peakNext)" aria-hidden="true">
                  ▲{" "}
                </span>
              )}
              {formatPercent(value)}%
            </>
          ) : (
            "--"
          )}
        </span>
      </div>
      <div
        role="meter"
        aria-label={name}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={known ? Math.min(value, 100) : undefined}
        aria-valuetext={valueText}
        className="h-1 rounded-full overflow-hidden bg-(--w-track) transition-colors duration-700"
      >
        <div
          className="h-full rounded-full"
          style={{ width: `${known ? Math.min(value, 100) : 0}%`, background: fill }}
        />
      </div>
      {reset && known && (
        <div
          aria-hidden="true"
          className="text-[11px] leading-4 text-(--w-muted) transition-colors duration-700"
        >
          {reset}
        </div>
      )}
    </div>
  );
}

export function Note({ children }: { children: ReactNode }) {
  return <div className="text-[11px] leading-4 text-(--w-muted)">{children}</div>;
}

export function ResetLine({ children }: { children: ReactNode }) {
  return (
    <div className="text-[11px] leading-4 mt-1 text-(--w-muted) transition-colors duration-700">
      {children}
    </div>
  );
}
