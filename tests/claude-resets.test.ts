import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { claudeResetRows, claudeResetsText } from "@/lib/claude-resets";
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

describe("claudeResetsText", () => {
  it("lists each kind with its count, nearest expiry, and the weekly 5-hour reset", () => {
    expect(
      claudeResetsText({
        groups: [
          group({ count: 2, earliestExpiry: "2026-12-03T12:00:00.000Z" }),
          group({ kind: "fiveHour", label: "5-hour reset" }),
        ],
        weeklyFiveHourAvailable: true,
      }),
    ).toBe(
      "2 full resets (earliest ends Dec 3) · 1 five-hour reset · weekly 5-hour reset available",
    );
  });

  it("marks paused grants, keeps Claude's own names, and says when the weekly reset is spent", () => {
    expect(
      claudeResetsText({
        groups: [
          group({
            kind: "fiveHour",
            label: "5-hour reset",
            paused: true,
            earliestExpiry: "2026-12-01T12:00:00.000Z",
          }),
          group({ kind: "other", label: "Welcome back", count: 3 }),
          group({ kind: "weekly", label: "Bonus weekly reset" }),
        ],
        weeklyFiveHourAvailable: false,
      }),
    ).toBe(
      '1 five-hour reset (paused, ends Dec 1) · 3 "Welcome back" resets · 1 bonus weekly reset · weekly 5-hour reset used this week',
    );
  });

  it("is only the weekly reset when no grants are banked", () => {
    expect(claudeResetsText({ groups: [], weeklyFiveHourAvailable: true })).toBe(
      "weekly 5-hour reset available",
    );
  });
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
  it("shows the total and names each kind in its tooltip", () => {
    const html = card(3, {
      groups: [group({ count: 2 })],
      weeklyFiveHourAvailable: true,
    });
    expect(html).toContain("↺ 3");
    expect(html).toContain(
      'title="Reset credits available: 3. 2 full resets · weekly 5-hour reset available"',
    );
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
