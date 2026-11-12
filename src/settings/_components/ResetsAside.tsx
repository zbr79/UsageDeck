import type { ResetRow } from "@/lib/claude-resets";

/** An account's resets to the right of its card: each type, and how many with when the first ends. */
export default function ResetsAside({ rows }: { rows: ResetRow[] }) {
  return (
    <ul
      aria-label="Resets"
      className="flex flex-col justify-center gap-1.5 rounded-lg border border-(--w-cardBorder) bg-(--w-card) px-3 py-2 text-[11px] leading-4"
    >
      {rows.length === 0 ? (
        <li className="text-(--w-muted)">No resets</li>
      ) : (
        rows.map((row) => (
          <li key={row.key} className="flex items-baseline justify-between gap-3">
            <span className="truncate text-(--w-text)">{row.title}</span>
            <span className="shrink-0 whitespace-nowrap tabular-nums text-(--w-muted)">
              {row.detail}
            </span>
          </li>
        ))
      )}
    </ul>
  );
}
