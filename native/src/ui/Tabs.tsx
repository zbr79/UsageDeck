import {
  Fragment,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent,
  type ReactNode,
} from "react";
import { moveItem } from "@/lib/settings-schema";

export interface TabItem<T extends string> {
  id: T;
  label: ReactNode;
}

/**
 * WAI-ARIA tabs: one tab stop for the list, arrow keys / Home / End move
 * between tabs (activation follows focus).
 */
export function TabList<T extends string>({
  items,
  value,
  onChange,
  label,
  idPrefix,
  className = "",
  onReorder,
  orientation = "horizontal",
}: {
  items: TabItem<T>[];
  value: T;
  onChange: (id: T) => void;
  label: string;
  idPrefix: string;
  className?: string;
  /** When set, a drag along the tab list's orientation reorders the tabs on drop. */
  onReorder?: (ids: T[]) => void;
  orientation?: "horizontal" | "vertical";
}) {
  const vertical = orientation === "vertical";
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const dragRef = useRef<{ id: T; pointerId: number; startX: number; moved: boolean } | null>(null);
  const suppressClick = useRef(false);
  const [dragging, setDragging] = useState<T | null>(null);
  const [dropTarget, setDropTarget] = useState<number | null>(null);

  const onKeyDown = (e: KeyboardEvent, index: number) => {
    const last = items.length - 1;
    const next =
      (!vertical && e.key === "ArrowRight") || (vertical && e.key === "ArrowDown")
        ? (index + 1) % items.length
        : (!vertical && e.key === "ArrowLeft") || (vertical && e.key === "ArrowUp")
          ? (index - 1 + items.length) % items.length
          : e.key === "Home"
            ? 0
            : e.key === "End"
              ? last
              : null;
    if (next === null) return;
    e.preventDefault();
    onChange(items[next].id);
    refs.current[next]?.focus();
  };

  const dropIndex = (position: number) => {
    const first = refs.current[0]?.getBoundingClientRect();
    if (first && position < (vertical ? first.top : first.left)) return 0;
    for (let i = 0; i < items.length; i++) {
      const rect = refs.current[i]?.getBoundingClientRect();
      if (rect && position <= (vertical ? rect.bottom : rect.right)) return i;
    }
    return items.length - 1;
  };

  const onPointerDown = (e: PointerEvent<HTMLButtonElement>, id: T) => {
    if (!onReorder || e.button !== 0) return;
    dragRef.current = {
      id,
      pointerId: e.pointerId,
      startX: vertical ? e.clientY : e.clientX,
      moved: false,
    };
    setDropTarget(null);
    e.currentTarget.setPointerCapture(e.pointerId);
  };

  const onPointerMove = (e: PointerEvent<HTMLButtonElement>) => {
    const drag = dragRef.current;
    if (!drag || e.pointerId !== drag.pointerId) return;
    if (!drag.moved && Math.abs((vertical ? e.clientY : e.clientX) - drag.startX) < 4) return;
    drag.moved = true;
    setDragging(drag.id);
    const list = items.map((item) => item.id);
    const from = list.indexOf(drag.id);
    const to = dropIndex(vertical ? e.clientY : e.clientX);
    setDropTarget(from === to ? null : to);
  };

  const onPointerUp = (e: PointerEvent<HTMLButtonElement>) => {
    const drag = dragRef.current;
    if (!drag || e.pointerId !== drag.pointerId) return;
    dragRef.current = null;
    setDragging(null);
    setDropTarget(null);
    if (!drag.moved) return;
    suppressClick.current = true;
    const list = items.map((item) => item.id);
    const from = list.indexOf(drag.id);
    const to = dropIndex(vertical ? e.clientY : e.clientX);
    if (from !== to) onReorder?.(moveItem(list, from, to));
  };

  return (
    <div
      role="tablist"
      aria-label={label}
      className={`flex ${
        vertical ? "flex-col gap-1" : "gap-5 border-b border-(--p-divider)"
      } ${className}`}
    >
      {items.map((item, i) => {
        const selected = item.id === value;
        return (
          <Fragment key={item.id}>
            {dropTarget === i && (
              <span
                aria-hidden="true"
                data-reorder-preview
                className={
                  vertical
                    ? "mx-2 h-0.5 rounded-full bg-(--p-accent)"
                    : "h-3 w-0.5 rounded-full bg-(--p-accent)"
                }
              />
            )}
            <button
              ref={(el) => {
                refs.current[i] = el;
              }}
              type="button"
              role="tab"
              id={`${idPrefix}-tab-${item.id}`}
              aria-selected={selected}
              aria-controls={`${idPrefix}-panel-${item.id}`}
              tabIndex={selected ? 0 : -1}
              draggable={false}
              onDragStart={(e) => e.preventDefault()}
              onClick={() => {
                if (suppressClick.current) {
                  suppressClick.current = false;
                  return;
                }
                onChange(item.id);
              }}
              onKeyDown={(e) => onKeyDown(e, i)}
              onPointerDown={onReorder ? (e) => onPointerDown(e, item.id) : undefined}
              onPointerMove={onReorder ? onPointerMove : undefined}
              onPointerUp={onReorder ? onPointerUp : undefined}
              onPointerCancel={onReorder ? onPointerUp : undefined}
              className={`flex items-center gap-1.5 text-sm transition-colors ${
                vertical
                  ? "w-full justify-start rounded-md border-l-2 px-3 py-2"
                  : "border-b-2 px-0.5 pb-2 pt-1 -mb-px"
              } ${onReorder ? "cursor-grab touch-none active:cursor-grabbing" : ""} ${
                dragging === item.id ? "opacity-40" : ""
              } ${
                selected
                  ? vertical
                    ? "border-(--p-accent) bg-(--p-surface) text-(--p-text) font-medium"
                    : "border-(--p-accent) text-(--p-text) font-medium"
                  : vertical
                    ? "border-transparent text-(--p-muted) hover:bg-(--p-surfaceHover) hover:text-(--p-textSecondary)"
                    : "border-transparent text-(--p-muted) hover:text-(--p-textSecondary)"
              }`}
            >
              {item.label}
            </button>
          </Fragment>
        );
      })}
    </div>
  );
}

export function TabPanel({
  idPrefix,
  id,
  children,
  className = "",
}: {
  idPrefix: string;
  id: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <div
      role="tabpanel"
      id={`${idPrefix}-panel-${id}`}
      aria-labelledby={`${idPrefix}-tab-${id}`}
      tabIndex={0}
      className={className}
    >
      {children}
    </div>
  );
}
