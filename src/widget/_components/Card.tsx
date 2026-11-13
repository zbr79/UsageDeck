import type { CSSProperties, ReactNode } from "react";
import { EyeOffIcon } from "@/ui/Icons";
import { formatPercent } from "@/lib/format";
import { usageSeverity, type MeterSegment, type MeterSeverity } from "@/lib/pace";

export type CardState = "normal" | "active" | "exhausted";

/** Settings-only additions to a card. The widget passes none, so it renders exactly as before. */
export interface CardExtras {
  /** highlight as the signed-in account (an exhausted card keeps its red ring) */
  active?: boolean;
  /** a gray card: hidden from the widget, or an OpenCode subscription that lapsed */
  dimmed?: boolean;
  /** hidden from the widget: an eye-slash icon after the name */
  hidden?: boolean;
  /** no badges and no hints (a lapsed OpenCode subscription is just a gray card) */
  plain?: boolean;
  /** controls at the right end of the header row (settings: the "more" button) */
  headerActions?: ReactNode;
}

/** What a settings account cell hands its card: the header controls. */
export type CardHeader = Pick<CardExtras, "headerActions">;

/**
 * A dimmed card's text and meters read the palette's dim tokens instead of fading with opacity, so text
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
  titleAfter,
  titleAction,
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
  /** muted text right after the title, same size (Claude: the spend rate, "$28/h") */
  titleAfter?: ReactNode;
  /** a control right after the title (widget: hide a used-up card until it resets) */
  titleAction?: ReactNode;
  hints?: ReactNode;
  badges?: ReactNode;
  state?: CardState;
  extras?: CardExtras;
  children: ReactNode;
}) {
  // Settings cards also hold a "more" button, so header hints move to their own
  // line below the title and the account name keeps its width. The widget keeps them inline.
  const hintsBelow = extras?.headerActions !== undefined;
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
      className={`relative rounded-lg p-2 border border-solid bg-(--w-card) transition-colors duration-700 ${ring}`}
      style={extras?.dimmed ? DIM_VARS : undefined}
    >
      <div className="flex items-center gap-1.5 mb-1.5 min-w-0">
        {icon}
        <span
          className="text-[12px] leading-4 font-semibold truncate min-w-0 text-(--w-text) transition-colors duration-700"
          title={titleHint}
        >
          {title}
        </span>
        {titleAfter}
        {titleAction}
        {extras?.hidden && <HiddenIcon />}
        {!hintsBelow && hints}
        <span className="ml-auto flex items-center gap-1 shrink-0">{!extras?.plain && badges}</span>
        {extras?.headerActions && (
          <span className="flex items-center gap-1 shrink-0">{extras.headerActions}</span>
        )}
      </div>
      {hintsBelow && !extras?.plain && (
        // Hidden while every hint is empty (a hint group with nothing to say).
        <div className="ud-card-hints hidden has-[*:not(:empty)]:flex flex-wrap items-center gap-x-1.5 -mt-0.5 mb-1.5 min-w-0">
          {hints}
        </div>
      )}
      {children}
    </section>
  );
}

/** Settings: the account is hidden from the widget. The native tooltip shows on hover. */
function HiddenIcon() {
  return (
    <span
      role="img"
      aria-label="Hidden from widget"
      title="Hidden from widget"
      className="flex size-4 shrink-0 items-center justify-center text-(--w-muted)"
    >
      <EyeOffIcon size={16} />
    </span>
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

const SEVERITY_FILL: Record<MeterSeverity, string> = {
  ok: "var(--w-barOk)",
  warn: "var(--w-barWarn)",
  danger: "var(--w-barDanger)",
};

/** How much of the fill colour a meter segment keeps; the rest is white, so it reads lighter. */
const SEGMENT_MIX = "55%";

/** A segment's width as a share of the (capped) fill. */
function segmentWidth(pct: number, value: number): number {
  return Math.min(100, (Math.max(0, pct) / Math.min(value, 100)) * 100);
}

/** Usage meter: label, percent, bar, optional even-pace tick, optional reset countdown. */
export function Meter({
  label,
  name,
  percent,
  unknown = false,
  dimmed = false,
  increased = false,
  reset,
  pacePct = null,
  severity,
  lines = [],
  segment = null,
}: {
  label: string;
  /** full name, spoken and shown as the tooltip, e.g. "Rolling 5-hour window" */
  name: string;
  percent: number | null;
  unknown?: boolean;
  dimmed?: boolean;
  increased?: boolean;
  reset?: string | null;
  /** even pace: a thin tick at this percent across the bar; null draws none */
  pacePct?: number | null;
  /** fill colour; defaults to the usage colours (warn from 80%, danger from 100%) */
  severity?: MeterSeverity;
  /** tooltip lines under the name and percent, e.g. one per model; also spoken */
  lines?: readonly string[];
  /** one model's part of the fill, drawn lighter at its end; null draws the plain fill */
  segment?: MeterSegment | null;
}) {
  const known = !unknown && percent != null;
  const value = known ? Math.max(0, percent) : 0;
  const fill =
    !known || dimmed ? "var(--w-barDim)" : SEVERITY_FILL[severity ?? usageSeverity(value)];
  const part = known && !dimmed && segment && value > 0 ? segment : null;
  const valueText = known
    ? `${formatPercent(value)}%${reset ? `, resets in ${reset}` : ""}${lines.length ? `. ${lines.join(". ")}` : ""}`
    : "unknown";
  const tick = known && !dimmed && pacePct != null ? Math.min(100, Math.max(0, pacePct)) : null;
  return (
    <div
      className="flex-1 min-w-0"
      title={known ? [`${name} ${formatPercent(value)}%`, ...lines].join("\n") : name}
    >
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
      <div className="relative">
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
            className="h-full rounded-full transition-colors duration-700"
            style={{ width: `${known ? Math.min(value, 100) : 0}%`, background: fill }}
          >
            {part && (
              // sized against the fill and placed at its end, so it never runs past it
              <div
                data-testid="meter-segment"
                className="h-full rounded-full transition-colors duration-700"
                style={{
                  width: `${segmentWidth(part.pct, value)}%`,
                  marginLeft: `${100 - segmentWidth(part.pct, value)}%`,
                  background: `color-mix(in srgb, ${fill} ${SEGMENT_MIX}, white)`,
                }}
              />
            )}
          </div>
        </div>
        {tick != null && (
          // 2 px wide, 8 px tall over the 4 px bar; kept inside the bar's ends
          <div
            aria-hidden="true"
            data-testid="pace-tick"
            className="absolute -top-0.5 h-2 w-0.5 rounded-full bg-(--w-paceTick) transition-colors duration-700"
            style={{ left: `clamp(0px, calc(${tick}% - 1px), calc(100% - 2px))` }}
          />
        )}
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
