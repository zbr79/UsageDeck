import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { fiveHourFableSegment, weeklyModelSegment } from "@/lib/pace";
import { DEFAULT_SETTINGS } from "@/lib/settings-schema";
import type { ClaudeScopedWindow, ClaudeUsage, ModelShare } from "@/lib/types";
import { Meter } from "@/widget/_components/Card";
import { ClaudeCard } from "@/widget/_components/cards";

const NOW = Date.parse("2026-11-12T10:00:00Z");
const H = 3_600_000;
const at = (ms: number) => new Date(NOW + ms).toISOString();

const fable = (share: number | null, allowance: number | null = 50): ClaudeScopedWindow => ({
  label: "Fable",
  percent: 36,
  resetsAt: at(50 * H),
  sharePctOfWeekly: share,
  allowancePctOfWeekly: allowance,
});

describe("weeklyModelSegment", () => {
  it("is the model's points of the weekly bar", () => {
    expect(weeklyModelSegment(60, fable(18))).toEqual({ pct: 18 });
  });

  it("never runs past the weekly fill", () => {
    expect(weeklyModelSegment(10, fable(12))?.pct).toBe(10);
  });

  it("is null without use, without a scoped limit, or on an empty bar", () => {
    expect(weeklyModelSegment(60, fable(0))).toBeNull();
    expect(weeklyModelSegment(60, fable(null))).toBeNull();
    expect(weeklyModelSegment(60, undefined)).toBeNull();
    expect(weeklyModelSegment(0, fable(5))).toBeNull();
  });
});

describe("fiveHourFableSegment", () => {
  const share = (usd: number, totalUsd: number): ModelShare => ({ usd, totalUsd });

  it("is the 5-hour bar times Fable's share of the dollars", () => {
    expect(fiveHourFableSegment(31, share(135.2, 353.1))).toEqual({
      pct: (31 * 135.2) / 353.1,
    });
    expect(fiveHourFableSegment(40, share(10, 10))?.pct).toBe(40);
  });

  it("is null without Fable dollars, without any dollars, or on an empty bar", () => {
    expect(fiveHourFableSegment(31, null)).toBeNull();
    expect(fiveHourFableSegment(31, share(0, 50))).toBeNull();
    expect(fiveHourFableSegment(31, share(5, 0))).toBeNull();
    expect(fiveHourFableSegment(0, share(5, 10))).toBeNull();
  });
});

const meter = (props: Partial<Parameters<typeof Meter>[0]> = {}) =>
  renderToStaticMarkup(
    createElement(Meter, { label: "Weekly", name: "Weekly", percent: 60, reset: "2d", ...props }),
  );

describe("Meter segment", () => {
  it("draws nothing extra without a segment", () => {
    expect(meter()).not.toContain("meter-segment");
    expect(meter({ segment: null })).toBe(meter());
  });

  it("draws the segment inside the fill; the tooltip has one line per model", () => {
    const html = meter({ lines: ["Fable 18 pts", "Other models 42 pts"], segment: { pct: 18 } });
    expect(html).toContain('data-testid="meter-segment"');
    expect(html).toContain("width:30%");
    expect(html).toContain("margin-left:70%");
    expect(html).toContain('title="Weekly 60%\nFable 18 pts\nOther models 42 pts"');
    expect(html).toContain("60%, resets in 2d. Fable 18 pts. Other models 42 pts");
  });

  it("sizes the segment against the capped fill of a bar over 100%", () => {
    expect(meter({ percent: 120, segment: { pct: 60 } })).toContain("width:60%");
  });

  it("draws no segment on an unknown or dimmed meter", () => {
    const segment = { pct: 18 };
    expect(meter({ unknown: true, segment })).not.toContain("meter-segment");
    expect(meter({ dimmed: true, segment })).not.toContain("meter-segment");
  });
});

const usage = (scoped: ClaudeScopedWindow[]): ClaudeUsage => ({
  email: "me@example.com",
  displayName: null,
  signedIn: true,
  subscribed: true,
  subscriptionType: "pro",
  planName: "Pro",
  fiveHour: { percent: 31, resetsAt: at(4 * H) },
  weekly: { percent: 60, resetsAt: at(50 * H) },
  weeklyOpus: null,
  weeklySonnet: null,
  weeklyScoped: scoped,
  resetCredits: null,
  resets: null,
  billingType: null,
  rateLimitTier: null,
  orgUuid: null,
});

const card = (
  data: ClaudeUsage,
  fableFiveHour: ModelShare | null,
  hiddenBars: typeof DEFAULT_SETTINGS.hiddenBars = {},
) =>
  renderToStaticMarkup(
    createElement(ClaudeCard, {
      slot: { data, stale: false, cachedAt: null, authExpired: false, error: null, loaded: true },
      settings: { ...DEFAULT_SETTINGS, hiddenBars },
      now: NOW,
      fableFiveHour,
    }),
  );
const segments = (html: string) => html.split('data-testid="meter-segment"').length - 1;

describe("Claude card", () => {
  it("shows Fable's part on the weekly and 5-hour bars", () => {
    const html = card(usage([fable(18)]), { usd: 40, totalUsd: 100 });
    expect(segments(html)).toBe(2);
    expect(html).toContain("Fable 18 pts\nOther models 42 pts");
  });

  it("is unchanged without Fable use", () => {
    const plain = card(usage([fable(0)]), null);
    expect(segments(plain)).toBe(0);
    expect(plain).not.toContain("Fable 0 pts");
  });

  it("shows segments only on the bars that are turned on", () => {
    const html = card(usage([fable(18)]), { usd: 40, totalUsd: 100 }, { claude: ["fiveHour"] });
    expect(segments(html)).toBe(1);
    expect(html).not.toContain("≈");
  });
});
