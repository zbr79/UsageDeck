import { useCallback, useEffect, useState } from "react";
import { toastError, toastSuccess } from "@/components/toast/toast";
import {
  copyDiagnostics,
  cursorRows,
  getDiagnostics,
  type Diagnostics as Report,
} from "@/lib/diagnostics";
import { secondaryButton, Section } from "./fields";

const REFRESH_MS = 10_000;

/** A time for the reader: "14:02" today, "Dec 3, 14:02" on another day. */
export function formatAt(ms: number, now = Date.now()): string {
  const date = new Date(ms);
  const time = date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false });
  if (date.toDateString() === new Date(now).toDateString()) return time;
  return `${date.toLocaleDateString("en-US", { month: "short", day: "numeric" })}, ${time}`;
}

/** The diagnostics report, re-read while the page is open. */
export function useDiagnostics(): { report: Report | null; reload: () => void } {
  const [report, setReport] = useState<Report | null>(null);
  const reload = useCallback(() => {
    getDiagnostics().then(setReport, (err) => console.error("get_diagnostics failed", err));
  }, []);
  useEffect(() => {
    reload();
    const id = setInterval(reload, REFRESH_MS);
    return () => clearInterval(id);
  }, [reload]);
  return { report, reload };
}

export function CopyDiagnosticsButton() {
  const copy = () =>
    copyDiagnostics().then(
      () => toastSuccess("Diagnostics copied"),
      () => toastError("Couldn't copy the diagnostics"),
    );
  return (
    <button type="button" onClick={() => void copy()} className={secondaryButton}>
      Copy diagnostics
    </button>
  );
}

/**
 * Settings → Cursor: what the last attempt to read Cursor's usage did, step by step, so a stale
 * card can be explained from a screenshot. No token is ever part of it.
 */
export function CursorDiagnostics() {
  const { report, reload } = useDiagnostics();
  if (!report) return null;
  const rows = report.cursor ? cursorRows(report.cursor, report.cursorReason, formatAt) : [];
  return (
    <Section title="Diagnostics">
      <div className="ud-row rounded border border-(--s-cardBorder) bg-(--s-card) p-4">
        <div className="mb-3 flex flex-wrap items-center gap-2">
          <p className="min-w-0 flex-1 text-xs text-(--p-muted)">
            What UsageDeck did the last time it read Cursor&apos;s usage. Tokens are never shown:
            only the first characters of a hash.
          </p>
          <button type="button" onClick={reload} className={secondaryButton}>
            Refresh
          </button>
          <CopyDiagnosticsButton />
        </div>
        {rows.length === 0 ? (
          <p className="text-sm text-(--p-muted)">Nothing tried yet since UsageDeck started.</p>
        ) : (
          <dl className="grid grid-cols-[minmax(0,2fr)_minmax(0,3fr)] gap-x-4 gap-y-1.5 text-xs">
            {rows.map((row, i) => (
              <div key={i} className="contents">
                <dt className="break-words text-(--p-muted)">{row.label}</dt>
                <dd
                  className={`break-words select-text ${row.bad ? "font-medium text-(--w-warn)" : ""}`}
                >
                  {row.value}
                </dd>
              </div>
            ))}
          </dl>
        )}
        <p className="mt-3 text-xs text-(--p-muted)">
          UsageDeck {report.appVersion} · {report.system}
        </p>
      </div>
    </Section>
  );
}
