import { useEffect, useState, type ReactNode } from "react";
import { getJson } from "@/lib/api";
import {
  calibrationText,
  formatShare,
  formatUsd,
  formatUsdRate,
  modelShares,
  spendSeries,
  spendSeriesSummary,
  type SpendPoint,
} from "@/lib/pace";
import { sanitizeHourlyTarget } from "@/lib/settings-schema";
import type { ClaudePace, Settings } from "@/lib/types";
import { inputClass, Section, secondaryButton, SettingRow } from "./fields";

type Update = (patch: Partial<Settings>, success?: string) => Promise<boolean>;

const PACE_MS = 60_000;

export interface PaceState {
  pace: ClaudePace | null;
  failed: boolean;
  now: number;
}

/** Fetches /api/claude/pace while `enabled` (the Claude page is open), every 60 s. */
export function usePace(enabled: boolean): PaceState {
  const [state, setState] = useState<PaceState>(() => ({
    pace: null,
    failed: false,
    now: Date.now(),
  }));
  useEffect(() => {
    if (!enabled) return;
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
  }, [enabled]);
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

const TOTAL_LINE = "var(--p-badge-blue-fg)";
const FABLE_LINE = "var(--p-badge-amber-fg)";

/** Spend over 24 h in quarter hours, oldest on the left: all spend as a filled line, Fable as a
 * yellow line. Plain SVG with an accessible summary. */
export function SpendChart({ quarters, fable }: { quarters: number[]; fable: number[] }) {
  const points = spendSeries(quarters, fable);
  const W = 480;
  const H = 110;
  const max = Math.max(1, ...points.map((p) => p.total)) * 1.1;
  const x = (i: number) => (points.length > 1 ? (i / (points.length - 1)) * W : 0);
  const y = (v: number) => H - (v / max) * H;
  const line = (pick: (p: SpendPoint) => number) =>
    points.map((p, i) => `${i ? "L" : "M"}${x(i).toFixed(1)},${y(pick(p)).toFixed(1)}`).join("");
  const total = line((p) => p.total);
  return (
    <div>
      <div className="relative">
        <svg
          role="img"
          aria-label={spendSeriesSummary(points)}
          viewBox={`0 0 ${W} ${H}`}
          preserveAspectRatio="none"
          className="block h-28 w-full overflow-visible"
        >
          <defs>
            <linearGradient id="spend-fill" x1="0" x2="0" y1="0" y2="1">
              <stop offset="0" stopColor={TOTAL_LINE} stopOpacity={0.3} />
              <stop offset="1" stopColor={TOTAL_LINE} stopOpacity={0} />
            </linearGradient>
          </defs>
          <line x1={0} x2={W} y1={H - 0.5} y2={H - 0.5} stroke="var(--p-border)" strokeWidth={1} />
          {points.length > 1 && (
            <>
              <path d={`${total}L${W},${H}L0,${H}Z`} fill="url(#spend-fill)" />
              <path
                d={total}
                fill="none"
                stroke={TOTAL_LINE}
                strokeWidth={2}
                strokeLinejoin="round"
                vectorEffect="non-scaling-stroke"
              />
              <path
                d={line((p) => p.fable)}
                fill="none"
                stroke={FABLE_LINE}
                strokeWidth={2}
                strokeLinejoin="round"
                vectorEffect="non-scaling-stroke"
              />
            </>
          )}
        </svg>
        <span
          aria-hidden="true"
          className="absolute top-0 left-0 text-xs tabular-nums text-(--p-muted)"
        >
          {formatUsdRate(max / 1.1)}
        </span>
      </div>
      <div
        className="mt-1 flex items-center justify-between gap-4 text-xs text-(--p-muted)"
        aria-hidden="true"
      >
        <span>24 h ago</span>
        <span className="flex items-center gap-3">
          <span className="flex items-center gap-1.5">
            <span className="h-0.5 w-3 rounded-full" style={{ background: TOTAL_LINE }} />
            All
          </span>
          <span className="flex items-center gap-1.5">
            <span className="h-0.5 w-3 rounded-full" style={{ background: FABLE_LINE }} />
            Fable
          </span>
        </span>
        <span>now</span>
      </div>
    </div>
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
    </div>
  );
}

/** Settings → Claude: API-equivalent spend of Claude Code on this PC, and the calibration. */
export default function PaceSection({
  target,
  update,
  state: { pace, failed, now },
}: {
  target: number | null;
  update: Update;
  state: PaceState;
}) {
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
        {s.backfilling && (
          <div className="mt-3 text-xs text-(--p-muted)">
            Reading Claude Code logs for the first time…
          </div>
        )}
      </div>

      <div className={box}>
        <div className="mb-2 text-sm">Last 24 hours</div>
        <SpendChart quarters={s.quarters} fable={s.quartersFable} />
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
