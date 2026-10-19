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
