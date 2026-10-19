import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { menuFocusIndex, menuPosition } from "./menu-focus";

export interface MenuItem {
  id: string;
  label: string;
  onSelect: () => void;
  /** destructive: red text */
  danger?: boolean;
  /** draw a separator above this item */
  separatorBefore?: boolean;
}

/**
 * A context / "more options" menu (WAI-ARIA menu pattern): roving focus with the arrows, Home
 * and End; Esc, Tab, an outside click or the window losing focus close it; focus returns to
 * `returnFocus` when it closes. Rendered into the `[data-menu-root]` element (else <body>), so
 * a zoomed or clipped card cannot clip or scale it while it keeps the page's CSS variables.
 */
export function Menu({
  label,
  items,
  at,
  onClose,
  returnFocus,
}: {
  label: string;
  items: MenuItem[];
  /** viewport point the menu opens from */
  at: { x: number; y: number };
  onClose: () => void;
  returnFocus?: HTMLElement | null;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const itemRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const [focus, setFocus] = useState(0);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    setPos(
      menuPosition(
        at,
        { width: el.offsetWidth, height: el.offsetHeight },
        { width: window.innerWidth, height: window.innerHeight },
      ),
    );
  }, [at]);

  useEffect(() => {
    if (pos) itemRefs.current[focus]?.focus();
  }, [pos, focus]);

  useEffect(() => {
    const close = () => onClose();
    const outside = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    document.addEventListener("pointerdown", outside, true);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    return () => {
      document.removeEventListener("pointerdown", outside, true);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
      returnFocus?.focus();
    };
  }, [onClose, returnFocus]);

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape" || e.key === "Tab") {
      e.preventDefault();
      e.stopPropagation();
      onClose();
      return;
    }
    const next = menuFocusIndex(e.key, focus, items.length);
    if (next === null) return;
    e.preventDefault();
    setFocus(next);
  };

  return createPortal(
    <div
      ref={ref}
      role="menu"
      aria-label={label}
      onKeyDown={onKeyDown}
      onContextMenu={(e) => e.preventDefault()}
      style={{
        left: pos?.left ?? at.x,
        top: pos?.top ?? at.y,
        visibility: pos ? "visible" : "hidden",
      }}
      className="ud-menu fixed z-50 min-w-[200px] rounded-lg border border-(--p-border) bg-(--p-bg) p-1 shadow-xl text-(--p-text)"
    >
      {items.map((item, i) => (
        <div key={item.id}>
          {item.separatorBefore && <div role="separator" className="my-1 h-px bg-(--p-border)" />}
          <button
            ref={(el) => {
              itemRefs.current[i] = el;
            }}
            type="button"
            role="menuitem"
            tabIndex={i === focus ? 0 : -1}
            onMouseEnter={() => setFocus(i)}
            onClick={() => {
              onClose();
              item.onSelect();
            }}
            className={`flex w-full min-h-8 items-center rounded px-3 py-1.5 text-left text-sm hover:bg-(--p-surfaceHover) focus-visible:bg-(--p-surfaceHover) ${
              item.danger ? "text-(--p-danger)" : ""
            }`}
          >
            {item.label}
          </button>
        </div>
      ))}
    </div>,
    document.querySelector<HTMLElement>("[data-menu-root]") ?? document.body,
  );
}
