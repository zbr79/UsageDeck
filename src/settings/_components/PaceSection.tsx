import { useEffect, useState, type ReactNode } from "react";
import { getJson } from "@/lib/api";
import {
  calibrationText,
  formatClock,
  formatPct,
  formatPctPerHour,
  formatShare,
  formatUsd,
  formatUsdRate,
  hourlyOldestFirst,
  hourlySummary,
  modelShares,
  paceTone,
  paceVsEven,
  SPEND_BAND_TONE,
  spendBand,
  targetBandText,
  windowForecast,
  WINDOW_LABELS,
  type PaceTone,
} from "@/lib/pace";
import { sanitizeHourlyTarget } from "@/lib/settings-schema";
import type { ClaudePace, PaceWindow, Settings } from "@/lib/types";
import { PANEL_SPEND_TONE } from "@/ui/palette";
import { inputClass, Section, secondaryButton, SettingRow } from "./fields";

type Update = (patch: Partial<Settings>, success?: string) => Promise<boolean>;

const PACE_MS = 60_000;

const TONE_COLORS: Record<PaceTone, string> = {
  muted: "var(--p-text)",
  amber: "var(--p-badge-amber-fg)",
  red: "var(--p-danger)",
};

/** Fetches /api/claude/pace while the Claude page is open, every 60 s. */
function usePace(): { pace: ClaudePace | null; failed: boolean; now: number } {
  const [state, setState] = useState<{ pace: ClaudePace | null; failed: boolean; now: number }>(
    () => ({ pace: null, failed: false, now: Date.now() }),
  );
  useEffect(() => {
    let alive = true;
    const load = () =>
      getJson<ClaudePace>("/api/claude/pace").then(
        (pace) => alive && setState({ pace, failed: false, now: Date.now() }),
        () => alive && setState((prev) => ({ ...prev, failed: true, now: Date.now() })),
      );
    void load();
    const id = setInterval(load, PACE_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, []);
  return state;
}

const box = "rounded border border-(--s-cardBorder) bg-(--s-card) p-4";

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <div className="text-xs text-(--p-muted)">{label}</div>
      <div className="text-xl leading-7 font-semibold tabular-nums">{value}</div>
    </div>
  );
}

/** 24 hourly bars, oldest on the left; plain SVG with an accessible summary. */
export function HourlyChart({ hourly }: { hourly: number[] }) {
  const bars = hourlyOldestFirst(hourly);
  const max = Math.max(1, ...bars);
  const W = 480;
  const H = 96;
  const step = W / Math.max(1, bars.length);
  return (
    <div>
      <svg
        role="img"
        aria-label={hourlySummary(hourly)}
        viewBox={`0 0 ${W} ${H}`}
        preserveAspectRatio="none"
        className="block h-24 w-full"
      >
        <line x1={0} x2={W} y1={H - 0.5} y2={H - 0.5} stroke="var(--p-border)" strokeWidth={1} />
        {bars.map((v, i) => {
          const h = v > 0 ? Math.max(1.5, (v / max) * (H - 4)) : 0;
          const hoursAgo = bars.length - 1 - i;
          return (
            <rect
              key={i}
              x={i * step + 1.5}
              y={H - h}
              width={step - 3}
              height={h}
              rx={1.5}
              fill="var(--p-accent)"
              opacity={hoursAgo === 0 ? 1 : 0.7}
            >
              <title>
                {hoursAgo === 0 ? "This hour so far" : `${hoursAgo} h ago`}: {formatUsd(v)}
              </title>
            </rect>
          );
        })}
      </svg>
      <div className="mt-1 flex justify-between text-xs text-(--p-muted)" aria-hidden="true">
        <span>24 h ago</span>
        <span>now</span>
      </div>
    </div>
  );
}

function WindowRow({ name, w, now }: { name: string; w: PaceWindow | null; now: number }) {
  if (!w)
    return (
      <Row title={name} detail="No reading yet">
        <span className="text-(--p-muted)">—</span>
      </Row>
    );
  const parts = [`${formatPct(w.pct)} used`];
  if (w.state !== "measuring" && w.pctPerHour != null) parts.push(formatPctPerHour(w.pctPerHour));
  if (w.evenPacePct != null) parts.push(`even pace ${formatPct(w.evenPacePct)}`);
  const vs = paceVsEven(w);
  if (vs) parts.push(vs);
  const tone = paceTone(w, now);
  const reset = w.resetsAt ? Date.parse(w.resetsAt) : NaN;
  return (
    <Row title={name} detail={parts.join(" · ")}>
      <div className="text-right">
        <div
          className={`text-sm ${tone === "muted" ? "" : "font-semibold"}`}
          style={{ color: TONE_COLORS[tone] }}
        >
          {windowForecast(w, now, true)}
        </div>
        {Number.isFinite(reset) && (
          <div className="text-xs text-(--p-muted)">resets {formatClock(reset, now)}</div>
        )}
      </div>
    </Row>
  );
}

function Row({ title, detail, children }: { title: string; detail: string; children: ReactNode }) {
  return (
    <div className={`${box} flex min-h-[68px] items-center gap-4`}>
      <div className="min-w-0 flex-1">
        <div className="text-sm">{title}</div>
        <div className="text-xs text-(--p-muted)">{detail}</div>
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

const BAND_WORDS = { below: "below target", on: "on target", above: "above target" } as const;

/** The 60-min rate, coloured like the widget header when a target is set (plus the band in words). */
function RateFigure({ rate, target }: { rate: number; target: number | null }) {
  const band = spendBand(rate, target);
  const tone = SPEND_BAND_TONE[band];
  return (
    <span data-testid="pace-rate" data-tone={tone}>
      <span
        className="font-semibold"
        style={tone === "muted" ? undefined : { color: `var(--p-${PANEL_SPEND_TONE[tone]})` }}
      >
        {formatUsdRate(rate)}
      </span>
      {band in BAND_WORDS && ` (${BAND_WORDS[band as keyof typeof BAND_WORDS]})`}
    </span>
  );
}

const draftOf = (target: number | null) => (target == null ? "" : String(target));

/** "Hourly spend target": $ [ 100 ] /h, applied on Enter or blur; empty means off. */
function TargetRow({ target, update }: { target: number | null; update: Update }) {
  const [draft, setDraft] = useState(() => draftOf(target));
  useEffect(() => setDraft(draftOf(target)), [target]);
  const save = (next: number | null) => {
    if (next === target) return setDraft(draftOf(target));
    void update(
      { claudeHourlyTarget: next },
      next == null ? "Hourly spend target off" : `Hourly spend target ${formatUsdRate(next)}`,
    ).then((ok) => ok || setDraft(draftOf(target)));
  };
  const apply = () => {
    const text = draft.trim();
    if (text === "") return save(null);
    const next = sanitizeHourlyTarget(Number(text));
    // Out of range or not a number: put the saved value back.
    if (next === undefined) return setDraft(draftOf(target));
    setDraft(String(next));
    save(next);
  };
  return (
    <div data-testid="target-row">
      <SettingRow
        title="Hourly spend target"
        description="Colours the spend on the widget: yellow below, green near, red above."
        control={(labelId, descId) => (
          <>
            <span className="flex items-center gap-1 text-sm">
              <span className="text-(--p-muted)" aria-hidden="true">
                $
              </span>
              <input
                type="number"
                inputMode="numeric"
                min={1}
                max={10000}
                step={1}
                placeholder="Off"
                aria-labelledby={labelId}
                aria-describedby={descId}
                className={`${inputClass} w-24! tabular-nums`}
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                onBlur={apply}
                onKeyDown={(e) => {
                  if (e.key === "Enter") apply();
                }}
              />
              <span className="text-(--p-muted)" aria-hidden="true">
                /h
              </span>
            </span>
            <button
              type="button"
              className={`${secondaryButton} min-w-0!`}
              disabled={target == null && draft === ""}
              aria-label="Clear hourly spend target"
              onClick={() => {
                setDraft("");
                save(null);
              }}
            >
              Clear
            </button>
          </>
        )}
      />
      <p className="mt-1 px-4 text-xs text-(--p-muted)" data-testid="target-band">
        {target == null ? "Off: the widget shows the spend in grey." : targetBandText(target)}
      </p>
    </div>
  );
}

/** Settings → Claude: API-equivalent spend of Claude Code on this PC, and each window's pace. */
export default function PaceSection({ target, update }: { target: number | null; update: Update }) {
  const { pace, failed, now } = usePace();
  return (
    <div className="mt-8" data-testid="pace-section">
      <Section title="Spend & pace">
        <TargetRow target={target} update={update} />
        {!pace ? (
          <p className="text-sm text-(--p-muted)">
            {failed ? "Spend and pace are not available right now." : "Loading…"}
          </p>
        ) : (
          <PaceBody pace={pace} now={now} target={target} />
        )}
      </Section>
    </div>
  );
}

function PaceBody({ pace, now, target }: { pace: ClaudePace; now: number; target: number | null }) {
  const s = pace.spend;
  const usd = (v: number) => (s.backfilling ? "$—" : formatUsd(v));
  const models = modelShares(s.byModel);
  const calibration = calibrationText(pace.calibration.usdPerWeeklyPct);
  return (
    <>
      <div className={box}>
        <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
          <Stat label="Last hour" value={usd(s.lastHour)} />
          <Stat label="Last 5 h" value={usd(s.last5h)} />
          <Stat label="Today" value={usd(s.today)} />
          <Stat label="Last 7 days" value={usd(s.last7d)} />
        </div>
        <div className="mt-3 text-xs text-(--p-muted)">
          {s.backfilling ? (
            "Reading Claude Code logs for the first time…"
          ) : (
            <>
              Spending <RateFigure rate={s.rate60m} target={target} /> over the last 60 min ·{" "}
              {formatUsdRate(s.rate15m)} over the last 15 min
            </>
          )}
        </div>
      </div>

      <div className={box}>
        <div className="mb-2 text-sm">Last 24 hours</div>
        <HourlyChart hourly={s.hourly} />
        {models.length > 0 && (
          <ul className="mt-3 flex flex-col gap-1 text-xs" aria-label="Spend by model, last 24 h">
            {models.map((m) => (
              <li key={m.model} className="flex justify-between gap-4">
                <span className="truncate">{m.model}</span>
                <span className="shrink-0 tabular-nums text-(--p-textSecondary)">
                  {formatUsd(m.usd)} · {formatShare(m.share)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>

      <WindowRow name={WINDOW_LABELS.fiveHour.long} w={pace.windows.fiveHour} now={now} />
      <WindowRow name={WINDOW_LABELS.weekly.long} w={pace.windows.weekly} now={now} />

      <Row
        title="Calibration"
        detail="Dollars spent ÷ weekly percentage points gained in this weekly window"
      >
        <span className="text-sm">{calibration ?? "Not measured yet"}</span>
      </Row>

      {s.unpricedModels.length > 0 && (
        <p className="text-xs text-(--p-muted)" data-testid="pace-unpriced">
          Not priced: {s.unpricedModels.join(", ")} ({s.unpricedReplies}{" "}
          {s.unpricedReplies === 1 ? "reply" : "replies"}, not in the dollar figures)
        </p>
      )}
      <p className="text-xs text-(--p-muted)">
        Dollars cover Claude Code on this PC (API-equivalent, not your bill). Percentages cover all
        your Claude use.
      </p>
    </>
  );
}
