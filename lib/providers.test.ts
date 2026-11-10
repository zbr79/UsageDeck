import { describe, expect, it } from "vitest";
import { parseCursorUsage } from "@/lib/cursor";
import { parseCodexUsage } from "@/lib/codex";
import { parseClaudeUsage } from "@/lib/claude";
import { parseOpenCodeUsage } from "@/lib/opencode";

describe("parseCursorUsage", () => {
  it("parses plan usage, plan info, and grok sand status", () => {
    const res = parseCursorUsage(
      {
        billingCycleEnd: "1791993600000",
        planUsage: {
          autoPercentUsed: 18,
          apiPercentUsed: 55,
          totalPercentUsed: 37,
          totalSpend: 12.5,
          limit: 20,
          bonusSpend: 0,
        },
      },
      { planInfo: { planName: "Pro" } },
      {
        usagePercent: 72,
        nextResetTimestampUtc: "2026-10-15T00:00:00.000Z",
      },
      "dev@cursor.com",
    );

    expect(res).toEqual({
      autoPercentUsed: 18,
      apiPercentUsed: 55,
      totalPercentUsed: 37,
      billingCycleEnd: "2026-10-14T16:00:00.000Z",
      totalSpend: 12.5,
      limit: 20,
      bonusSpend: 0,
      accountName: "dev@cursor.com",
      planName: "Pro",
      grokPercentUsed: 72,
      grokResetAt: "2026-10-15T00:00:00.000Z",
    });
  });

  it("handles missing or malformed payloads gracefully", () => {
    const res = parseCursorUsage(null, null, { nextResetTimestampUtc: "invalid-date" }, null);
    expect(res.autoPercentUsed).toBeNull();
    expect(res.apiPercentUsed).toBeNull();
    expect(res.planName).toBeNull();
    expect(res.grokPercentUsed).toBeNull();
    expect(res.grokResetAt).toBeNull();
  });
});

describe("parseCodexUsage", () => {
  it("parses primary window rate limit and credits balance", () => {
    const res = parseCodexUsage({
      email: "user@openai.com",
      plan_type: "plus",
      rate_limit: {
        limit_reached: false,
        primary_window: {
          used_percent: 29,
          limit_window_seconds: 18000,
          reset_at: 1791993600,
        },
      },
      credits: { balance: "42.5" },
    });

    expect(res).toEqual({
      email: "user@openai.com",
      planType: "plus",
      usedPercent: 29,
      limitWindowSeconds: 18000,
      resetAt: "2026-10-14T16:00:00.000Z",
      creditsBalance: 42.5,
      limitReached: false,
    });
  });

  it("flags limitReached and handles null fields", () => {
    const res = parseCodexUsage({
      rate_limit: { limit_reached: true },
    });
    expect(res.limitReached).toBe(true);
    expect(res.usedPercent).toBeNull();
    expect(res.creditsBalance).toBeNull();
  });
});

describe("parseClaudeUsage", () => {
  it("detects signed-in and subscribed account from oauth + credentials", () => {
    const res = parseClaudeUsage(
      {
        oauthAccount: {
          emailAddress: "user@anthropic.com",
          displayName: "Claude User",
          billingType: "stripe_subscription",
          organizationUuid: "org-123",
        },
      },
      {
        claudeAiOauth: {
          accessToken: "sk-ant-oat01-xxx",
          subscriptionType: "max",
          rateLimitTier: "tier_4",
        },
      },
    );

    expect(res).toEqual({
      email: "user@anthropic.com",
      displayName: "Claude User",
      signedIn: true,
      subscribed: true,
      subscriptionType: "max",
      billingType: "stripe_subscription",
      rateLimitTier: "tier_4",
      orgUuid: "org-123",
    });
  });

  it("parses the three OpenCode usage windows", () => {
    const usage = {
      rolling: { status: "ok", percent: 12, resetsAt: "2026-10-15T01:00:00.000Z" },
      weekly: { status: "ok", percent: 40, resetsAt: "2026-10-18T00:00:00.000Z" },
      monthly: { status: "ok", percent: 70, resetsAt: "2026-11-01T00:00:00.000Z" },
    };
    expect(parseOpenCodeUsage({ usage })).toEqual(usage);
    expect(parseOpenCodeUsage({ usage: { rolling: usage.rolling } })).toBeNull();
    expect(parseOpenCodeUsage(null)).toBeNull();
  });

  it("returns unsigned and unsubscribed defaults when files are empty", () => {
    const res = parseClaudeUsage(null, null);
    expect(res.signedIn).toBe(false);
    expect(res.subscribed).toBe(false);
    expect(res.email).toBeNull();
  });
});
