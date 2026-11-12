import { claudeResetRows } from "@/lib/claude-resets";
import type { ClaudeResets } from "@/lib/types";

/** An account's Claude resets beside its card: each kind's name, and how many with when they end. */
export default function ResetsAside({ resets }: { resets: ClaudeResets }) {
  const rows = claudeResetRows(resets);
  if (rows.length === 0) return null;
  return (
    <ul
      aria-label="Resets"
      className="flex w-60 shrink-0 flex-col justify-center gap-1.5 rounded-lg border border-(--w-cardBorder) bg-(--w-card) px-3 py-2"
    >
      {rows.map((row) => (
        <li
          key={row.key}
          className="flex items-baseline justify-between gap-3 text-[11px] leading-4"
        >
          <span className="min-w-0 text-(--w-text)" title={row.title}>
            {row.title}
          </span>
          <span className="shrink-0 whitespace-nowrap tabular-nums text-(--w-muted)">
            {row.detail}
          </span>
        </li>
      ))}
    </ul>
  );
}
