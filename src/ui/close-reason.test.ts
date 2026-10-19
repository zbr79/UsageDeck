import { describe, expect, it } from "vitest";
import { backdropReason, cancelReason } from "./close-reason";

describe("dialog close reasons", () => {
  it("names the Escape path, the last key and the time since open", () => {
    expect(cancelReason({ isTrusted: true }, "Escape", 1200)).toBe(
      "escape (cancel, trusted=true, last key=Escape, 1200 ms after open)",
    );
  });

  it("marks a cancel with no key before it", () => {
    expect(cancelReason({ isTrusted: false }, null, 5)).toBe(
      "escape (cancel, trusted=false, last key=none, 5 ms after open)",
    );
  });

  it("names the backdrop path with the button state and the point", () => {
    const e = { isTrusted: true, button: 0, buttons: 1, clientX: 1490.5, clientY: 736.4 };
    expect(backdropReason(e, 3431)).toBe(
      "backdrop (trusted=true, button=0, buttons=1, at 1491,736, 3431 ms after open)",
    );
  });
});
