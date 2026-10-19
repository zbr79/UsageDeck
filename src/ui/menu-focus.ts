/**
 * Roving focus for a menu (WAI-ARIA menu pattern): arrows wrap, Home / End jump. `current` is
 * the focused item's index, or -1 when none is focused yet. Returns the next index, or null when
 * the key does not move focus.
 */
export function menuFocusIndex(key: string, current: number, count: number): number | null {
  if (count <= 0) return null;
  switch (key) {
    case "ArrowDown":
      return current < 0 ? 0 : (current + 1) % count;
    case "ArrowUp":
      return current < 0 ? count - 1 : (current - 1 + count) % count;
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return null;
  }
}

/** The keyboard shortcuts that open a context menu: the Menu key and Shift+F10. */
export function isContextMenuKey(e: { key: string; shiftKey: boolean }): boolean {
  return e.key === "ContextMenu" || (e.shiftKey && e.key === "F10");
}

/**
 * Where a menu of `size` goes when opened at `at`: below and right of the point, flipped or
 * shifted to stay inside the `viewport` with an 8 px margin.
 */
export function menuPosition(
  at: { x: number; y: number },
  size: { width: number; height: number },
  viewport: { width: number; height: number },
  margin = 8,
): { left: number; top: number } {
  let left = at.x;
  let top = at.y;
  if (left + size.width > viewport.width - margin) left = at.x - size.width;
  if (top + size.height > viewport.height - margin) top = at.y - size.height;
  left = Math.max(margin, Math.min(left, viewport.width - size.width - margin));
  top = Math.max(margin, Math.min(top, viewport.height - size.height - margin));
  return { left, top };
}
