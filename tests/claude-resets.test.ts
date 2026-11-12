import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { claudeResetRows } from "@/lib/claude-resets";
import { DEFAULT_SETTINGS } from "@/lib/settings-schema";
import type { ClaudeResetGroup, ClaudeResets, ClaudeUsage } from "@/lib/types";
import { ClaudeCard } from "@/widget/_components/cards";

const group = (g: Partial<ClaudeResetGroup>): ClaudeResetGroup => ({
  kind: "full",
  label: "Full reset",
  count: 1,
  earliestExpiry: null,
  paused: false,
  ...g,
});

const card = (resetCredits: number | null, resets: ClaudeResets | null) =>
  renderToStaticMarkup(
    createElement(ClaudeCard, {
      slot: {
        data: {
          email: "me@example.com",
          displayName: null,
          signedIn: true,
          subscribed: true,
          subscriptionType: "pro",
          planName: "Pro",
          fiveHour: { percent: 31, resetsAt: null },
          weekly: { percent: 60, resetsAt: null },
          weeklyOpus: null,
          weeklySonnet: null,
          weeklyScoped: [],
          resetCredits,
          resets,
          billingType: null,
          rateLimitTier: null,
          orgUuid: null,
        } satisfies ClaudeUsage,
        stale: false,
        cachedAt: null,
        authExpired: false,
        error: null,
        loaded: true,
      },
      settings: DEFAULT_SETTINGS,
      now: Date.parse("2026-11-12T10:00:00Z"),
      fableFiveHour: null,
    }),
  );

describe("Claude card reset badge", () => {
  it("shows the total; hovering lists each kind by its model or kind", () => {
    const html = card(4, {
      groups: [
        group({ label: "Free limit reset", count: 2, earliestExpiry: "2026-12-15T12:00:00Z" }),
        group({
          kind: "weekly",
          label: "New model launch: one Fable reset for Max, Team Premium and Enterprise Premium",
        }),
        group({ kind: "fiveHour", label: "5-hour reset", paused: true }),
      ],
      weeklyFiveHourAvailable: true,
    });
    expect(html.match(/↺ /g)).toHaveLength(1);
    expect(html).toContain("↺ 4");
    expect(html).toContain(
      'title="2 Full resets · 1 Fable reset · 1 5-hour reset (paused) · 1 weekly 5-hour reset"',
    );
  });

  it("falls back to the total when Claude names no kinds", () => {
    expect(card(0, { groups: [], weeklyFiveHourAvailable: false })).toContain("↺ 0");
  });

  it("shows no badge when Claude lists no resets", () => {
    expect(card(null, null)).not.toContain("↺");
  });
});

describe("claudeResetRows", () => {
  it("gives one row per kind with its own label, count and expiry, then the weekly reset", () => {
    const resets: ClaudeResets = {
      groups: [
        group({ label: "free limit reset", count: 2, earliestExpiry: "2026-12-15T12:00:00Z" }),
        group({ kind: "fiveHour", label: "", paused: true }),
      ],
      weeklyFiveHourAvailable: false,
    };
    expect(claudeResetRows(resets).map(({ title, detail }) => [title, detail])).toEqual([
      ["Free limit reset", "2 left · earliest ends Dec 15"],
      ["5-hour reset", "1 left · paused"],
      ["Weekly 5-hour reset", "Used this week"],
    ]);
  });
});
