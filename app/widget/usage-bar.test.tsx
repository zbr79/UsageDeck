import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import UsageBar from "./UsageBar";

describe("UsageBar", () => {
  it("exposes the window as a meter with a full name and value text", () => {
    const html = renderToStaticMarkup(
      <UsageBar
        label="W"
        w={{ status: "ok", percent: 40, resetsAt: "2026-10-18T00:00:00.000Z" }}
        dimmed={false}
        reset="2d"
      />,
    );
    expect(html).toContain('role="meter"');
    expect(html).toContain('aria-label="Weekly window"');
    expect(html).toContain('aria-valuenow="40"');
    expect(html).toContain('aria-valuetext="40 percent, resets in 2d"');
  });
});
