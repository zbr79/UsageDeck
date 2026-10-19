// Log text for the ways a Dialog closes itself. The settings overlay passes it to Rust, which
// logs it, so a close nobody expected can be traced to its path and to the event behind it.

/** Escape: the browser's `cancel` event, with the last key seen while the dialog was open. */
export function cancelReason(
  e: { isTrusted: boolean },
  lastKey: string | null,
  ms: number,
): string {
  return `escape (cancel, trusted=${e.isTrusted}, last key=${lastKey ?? "none"}, ${ms} ms after open)`;
}

/** A mousedown on the backdrop. clientX/Y are CSS pixels (physical = CSS x display scale). */
export function backdropReason(
  e: { isTrusted: boolean; button: number; buttons: number; clientX: number; clientY: number },
  ms: number,
): string {
  return (
    `backdrop (trusted=${e.isTrusted}, button=${e.button}, buttons=${e.buttons}, ` +
    `at ${Math.round(e.clientX)},${Math.round(e.clientY)}, ${ms} ms after open)`
  );
}
