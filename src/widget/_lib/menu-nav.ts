/**
 * Keyboard movement inside the widget's right-click menu (WAI-ARIA menu pattern):
 * Up / Down wrap around, Home / End jump to the ends.
 *
 * `current` is the focused item's index, or -1 when focus is not on an item.
 * Returns the index to focus, or null when the key does not move focus.
 */
export function menuNavIndex(key: string, current: number, count: number): number | null {
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

/** The keys of a keydown event that matter to the widget's shortcuts. */
export interface ShortcutKey {
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

/** Ctrl+, opens Settings (the common Windows app shortcut, also named in the right-click menu). */
export function isOpenSettingsKey(e: ShortcutKey): boolean {
  return e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey && e.key === ",";
}
