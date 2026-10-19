import { useEffect, useState, type ReactNode } from "react";
import { Badge } from "@/ui/controls";
import { formatAgo } from "@/lib/format";

/** Wall clock for reset countdowns, ticking once a minute like the widget. */
export function useNow(intervalMs = 60_000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(id);
  }, [intervalMs]);
  return now;
}

/** Widget-width account cards: two per row in the panel, one when it is narrower. */
export function CardGrid({ children }: { children: ReactNode }) {
  return (
    <ul className="grid grid-cols-[repeat(auto-fill,minmax(238px,1fr))] gap-3 p-1">{children}</ul>
  );
}

/** "updated 3h ago" from a cache time, or null when nothing was cached. */
export function updatedNote(cachedAt: number | null, now: number): string | null {
  return cachedAt ? `updated ${formatAgo(cachedAt, now)}` : null;
}

/** Note and controls under a card's meters. */
export function CardFooter({
  note,
  hidden,
  lead,
  children,
}: {
  note: string | null;
  /** hidden from the widget */
  hidden: boolean;
  /** shown before the note, e.g. a switch */
  lead?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="flex items-center gap-1.5 mt-1.5 pt-1 border-t border-(--w-cardBorder) min-w-0">
      {lead}
      <span
        title={note ?? undefined}
        className="flex-1 min-w-0 truncate text-[10px] leading-3 text-(--w-muted)"
      >
        {note}
      </span>
      {hidden && (
        <Badge scope="w" tone="gray" title="Hidden from the widget">
          hidden
        </Badge>
      )}
      {children}
    </div>
  );
}
