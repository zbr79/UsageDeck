import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import type { CardHeader } from "@/widget/_components/Card";
import { Menu, type MenuItem } from "@/ui/Menu";
import { isContextMenuKey } from "@/ui/menu-focus";

/** Wall clock for reset countdowns, ticking once a minute like the widget. */
export function useNow(intervalMs = 60_000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(id);
  }, [intervalMs]);
  return now;
}

/**
 * The widget's own cards, 2 per row, 1 when the page is narrow (a container query on the grid's
 * width). The cards are drawn at 1.1x (`.ud-cards` zoom) so their 11 px widget text reads at
 * 12 px or more, the settings minimum.
 */
export function CardGrid({ children }: { children: ReactNode }) {
  return (
    <div className="@container">
      <ul className="ud-cards grid grid-cols-1 gap-3 p-1 @[540px]:grid-cols-2">{children}</ul>
    </div>
  );
}

/**
 * One account card with its "⋯" button. The menu starts with "Hide from widget" / "Show on
 * widget", then the page's own items. It also opens on right-click, Shift+F10 and the Menu key;
 * focus returns to "⋯".
 */
export function AccountCell({
  name,
  shown,
  onShownChange,
  menuItems,
  card,
}: {
  /** the card's title (nickname, or else the email) */
  name: string;
  shown: boolean;
  onShownChange: (next: boolean) => void;
  menuItems: MenuItem[];
  card: (header: CardHeader) => ReactNode;
}) {
  const more = useRef<HTMLButtonElement>(null);
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);
  const close = useCallback(() => setMenuAt(null), []);
  const openFromButton = () => {
    const r = more.current?.getBoundingClientRect();
    if (r) setMenuAt({ x: r.left, y: r.bottom + 4 });
  };
  const items: MenuItem[] = [
    {
      id: "shown",
      label: shown ? "Hide from widget" : "Show on widget",
      onSelect: () => onShownChange(!shown),
    },
    ...menuItems,
  ];
  const actions = (
    <button
      ref={more}
      type="button"
      aria-label={`More options for ${name}`}
      title={`More options for ${name}`}
      aria-haspopup="menu"
      aria-expanded={menuAt !== null}
      onClick={() => (menuAt ? close() : openFromButton())}
      className="flex size-6 shrink-0 items-center justify-center rounded text-(--w-text) hover:bg-(--w-hover)"
    >
      <svg aria-hidden="true" width="14" height="14" viewBox="0 0 16 16" fill="currentColor">
        <circle cx="3" cy="8" r="1.4" />
        <circle cx="8" cy="8" r="1.4" />
        <circle cx="13" cy="8" r="1.4" />
      </svg>
    </button>
  );
  return (
    <li
      className="min-w-0"
      onContextMenu={(e) => {
        e.preventDefault();
        setMenuAt({ x: e.clientX, y: e.clientY });
      }}
      onKeyDown={(e) => {
        if (!isContextMenuKey(e)) return;
        e.preventDefault();
        openFromButton();
      }}
    >
      {card({ headerActions: actions })}
      {menuAt && (
        <Menu
          label={`Options for ${name}`}
          items={items}
          at={menuAt}
          onClose={close}
          returnFocus={more.current}
        />
      )}
    </li>
  );
}

/** "No accounts yet", plus what to do about it. */
export function EmptyState({ children }: { children: ReactNode }) {
  return (
    <div className="ud-row rounded border border-(--s-cardBorder) bg-(--s-card) px-6 py-8 text-center">
      <p className="text-sm font-semibold">No accounts yet</p>
      <div className="mt-1 text-sm text-(--p-muted)">{children}</div>
    </div>
  );
}

export function ProviderLogo({ src, invert }: { src: string; invert?: boolean }) {
  return (
    <img
      src={src}
      alt=""
      draggable={false}
      className={`size-8 shrink-0 rounded-md ${invert ? "invert" : ""}`}
    />
  );
}
