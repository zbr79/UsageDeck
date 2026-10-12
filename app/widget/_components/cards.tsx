"use client";

import { Badge } from "@/app/_ui/controls";
import { CopyIcon } from "@/app/_ui/Icons";
import type { Tone } from "@/app/_ui/palette";
import { capitalize, formatDuration, formatUntil } from "@/lib/format";
import type { PeakInfo } from "@/lib/peak";
import { isAccountExhausted, isExhaustedWindow, isPassed, isUnknownWindow } from "@/lib/ranking";
import type {
  ClaudeUsage,
  CodexUsage,
  CursorUsage,
  GrokUsage,
  OpenCodeAccountUsage,
  Settings,
} from "@/lib/types";
import type { ProviderSlot } from "../_lib/useWidgetData";
import { Card, Hint, Meter, Note, ProviderIcon, ResetLine, type CardState } from "./Card";

type NameSettings = Pick<Settings, "displayNames" | "showProviderNames">;

function nameFor(s: NameSettings, key: string, providerName: string, fallback?: string | null) {
  if (s.showProviderNames) return providerName;
  return s.displayNames[key] ?? fallback ?? providerName;
}

function CachedHint({ slot }: { slot: { stale: boolean; cachedAt: number | null } }) {
  if (!slot.stale) return null;
  const when = slot.cachedAt ? new Date(slot.cachedAt).toLocaleString() : null;
  return <Hint title={when ? `Last known data · ${when}` : "Last known data"}>cached</Hint>;
}

function ReauthHint({ expired, how }: { expired: boolean; how: string }) {
  if (!expired) return null;
  return (
    <Hint color="warn" title={`Sign-in expired · ${how} to refresh it`}>
      re-auth
    </Hint>
  );
}

function planTone(plan: string): Tone {
  return plan.toLowerCase() === "free" ? "green" : "blue";
}

function Unavailable<T>({ slot }: { slot: ProviderSlot<T> }) {
  return <Note>{slot.loaded ? "Not signed in or unavailable" : "Loading…"}</Note>;
}

// ---- OpenCode -------------------------------------------------------------

export function OpenCodeCard({
  row,
  active,
  primary,
  settings,
  peak,
  now,
  increased,
  onCopyKey,
}: {
  row: OpenCodeAccountUsage;
  active: boolean;
  /** the top card shows reset countdowns and the peak-hour hint */
  primary: boolean;
  settings: Settings;
  peak: PeakInfo;
  now: number;
  increased: (window: string) => boolean;
  onCopyKey: () => void;
}) {
  const title = nameFor(settings, row.email, "OpenCode", row.email.slice(0, 4));
  const u = row.usage;
  const state: CardState = isAccountExhausted(row, now)
    ? "exhausted"
    : active
      ? "active"
      : "normal";
  const unknown = (w: "rolling" | "weekly" | "monthly") =>
    !!u && isUnknownWindow(u[w], row.stale, now);
  const reset = (w: "rolling" | "weekly" | "monthly", show: boolean) =>
    u && show && !unknown(w) ? formatUntil(u[w].resetsAt, now) : null;

  let peakHint = null;
  if (settings.deepseekPeakHourWarning && primary) {
    if (peak.active && peak.endAt) {
      peakHint = <Hint color="peakActive">Peak ends in {formatDuration(peak.endAt - now, 2)}</Hint>;
    } else if (peak.nextStartAt) {
      peakHint = (
        <Hint color="peakNext">Peak starts in {formatDuration(peak.nextStartAt - now, 2)}</Hint>
      );
    }
  }

  return (
    <Card
      label={`OpenCode account ${title}`}
      state={state}
      icon={
        <button
          type="button"
          onClick={onCopyKey}
          aria-label={`Copy API key for ${title}`}
          title="Copy API key"
          className="group/copy relative shrink-0 rounded-[3px] p-0.5 -m-0.5 hover:bg-(--w-hover)"
        >
          <ProviderIcon src="/opencode.ico" alt="" />
          <span className="absolute inset-0 flex items-center justify-center rounded-[3px] bg-(--w-card) text-(--w-text) opacity-0 group-hover/copy:opacity-100 group-focus-visible/copy:opacity-100 transition-opacity">
            <CopyIcon size={10} />
          </span>
        </button>
      }
      title={title}
      titleHint={row.email}
      hints={
        <>
          {peakHint}
          <CachedHint slot={row} />
        </>
      }
      badges={
        <Badge scope="w" tone="blue">
          Go
        </Badge>
      }
    >
      {u ? (
        <div className="flex gap-2">
          <Meter
            label="R"
            name="Rolling usage"
            percent={u.rolling.percent}
            unknown={unknown("rolling")}
            dimmed={
              isExhaustedWindow(u.weekly, row.stale, now) ||
              isExhaustedWindow(u.monthly, row.stale, now)
            }
            increased={increased("rolling")}
            reset={reset("rolling", primary)}
          />
          <Meter
            label="W"
            name="Weekly usage"
            percent={u.weekly.percent}
            unknown={unknown("weekly")}
            dimmed={isExhaustedWindow(u.monthly, row.stale, now)}
            increased={increased("weekly")}
            reset={reset("weekly", primary)}
          />
          <Meter
            label="M"
            name="Monthly usage"
            percent={u.monthly.percent}
            unknown={unknown("monthly")}
            increased={increased("monthly")}
            reset={reset("monthly", true)}
          />
        </div>
      ) : (
        <Note>Unavailable</Note>
      )}
    </Card>
  );
}

// ---- Cursor ---------------------------------------------------------------

export function CursorCard({
  slot,
  settings,
  now,
}: {
  slot: ProviderSlot<CursorUsage>;
  settings: Settings;
  now: number;
}) {
  const c = slot.data;
  const title = nameFor(settings, "cursor", "Cursor", c?.accountName);
  const cyclePassed = slot.stale && isPassed(c?.billingCycleEnd, now);
  const exhausted = !!c && !cyclePassed && (c.totalPercentUsed ?? 0) >= 100;
  return (
    <Card
      label={`Cursor ${title}`}
      icon={<ProviderIcon src="/cursor.ico" alt="" />}
      title={title}
      titleHint={c?.accountName ?? undefined}
      state={exhausted ? "exhausted" : "normal"}
      hints={
        <>
          <ReauthHint expired={slot.authExpired} how="open Cursor" />
          <CachedHint slot={slot} />
        </>
      }
      badges={
        c?.planName ? (
          <Badge scope="w" tone="blue">
            {c.planName}
          </Badge>
        ) : null
      }
    >
      {c ? (
        <>
          <div className="flex gap-2">
            <Meter
              label="First Party"
              name="First-party usage"
              percent={c.autoPercentUsed}
              unknown={cyclePassed}
            />
            <Meter label="API" name="API usage" percent={c.apiPercentUsed} unknown={cyclePassed} />
          </div>
          {c.billingCycleEnd && !cyclePassed && (
            <ResetLine>Resets {formatUntil(c.billingCycleEnd, now)}</ResetLine>
          )}
        </>
      ) : (
        <Unavailable slot={slot} />
      )}
    </Card>
  );
}

// ---- Grok Bot -------------------------------------------------------------

export function GrokCard({
  slot,
  settings,
  now,
}: {
  slot: ProviderSlot<GrokUsage>;
  settings: Settings;
  now: number;
}) {
  const g = slot.data;
  const title = nameFor(settings, "grok", "Grok Bot");
  const passed = slot.stale && isPassed(g?.resetAt, now);
  const exhausted = !!g && !passed && (g.percentUsed ?? 0) >= 100;
  const plan = g?.planLabel ?? g?.cursorPlanName;
  const account = g?.email
    ? `${g.email}${g.source === "cursor" ? " (via Cursor sign-in)" : ""}`
    : undefined;
  return (
    <Card
      label={`Grok Bot ${title}`}
      icon={<ProviderIcon src="/grokbot.ico" alt="" />}
      title={title}
      titleHint={account}
      state={exhausted ? "exhausted" : "normal"}
      hints={
        <>
          <ReauthHint expired={slot.authExpired} how="open Grok Bot" />
          <CachedHint slot={slot} />
        </>
      }
      badges={
        plan ? (
          <Badge scope="w" tone="blue">
            {plan}
          </Badge>
        ) : null
      }
    >
      {g ? (
        <>
          <div className="flex gap-2">
            <Meter label="Weekly" name="Weekly usage" percent={g.percentUsed} unknown={passed} />
          </div>
          {g.resetAt && !passed && <ResetLine>Resets {formatUntil(g.resetAt, now)}</ResetLine>}
        </>
      ) : (
        <Unavailable slot={slot} />
      )}
    </Card>
  );
}

// ---- Codex ----------------------------------------------------------------

export function CodexCard({
  slot,
  settings,
  now,
  light,
}: {
  slot: ProviderSlot<CodexUsage>;
  settings: Settings;
  now: number;
  light: boolean;
}) {
  const x = slot.data;
  const title = nameFor(settings, "codex", "Codex");
  const passed = slot.stale && isPassed(x?.resetAt, now);
  const exhausted = !!x && !passed && (x.limitReached || (x.usedPercent ?? 0) >= 100);
  return (
    <Card
      label={`Codex ${title}`}
      icon={<ProviderIcon src="/openai.png" alt="" invert={light} />}
      title={title}
      titleHint={x?.email ?? undefined}
      state={exhausted ? "exhausted" : "normal"}
      hints={
        <>
          <ReauthHint expired={slot.authExpired} how="run codex" />
          <CachedHint slot={slot} />
        </>
      }
      badges={
        <>
          {x?.resetCredits != null && (
            <Badge
              scope="w"
              tone={x.resetCredits > 0 ? "violet" : "gray"}
              title={`${x.resetCredits} rate-limit reset${x.resetCredits === 1 ? "" : "s"} available`}
            >
              ↺ {x.resetCredits}
            </Badge>
          )}
          {x?.planType && (
            <Badge scope="w" tone={planTone(x.planType)}>
              {capitalize(x.planType)}
            </Badge>
          )}
        </>
      }
    >
      {x ? (
        <>
          <div className="flex gap-2">
            <Meter label="Usage" name="Codex usage" percent={x.usedPercent} unknown={passed} />
          </div>
          {x.resetAt && !passed && <ResetLine>Resets {formatUntil(x.resetAt, now)}</ResetLine>}
        </>
      ) : (
        <Unavailable slot={slot} />
      )}
    </Card>
  );
}

// ---- Claude ---------------------------------------------------------------

export function ClaudeCard({
  slot,
  settings,
  now,
}: {
  slot: ProviderSlot<ClaudeUsage>;
  settings: Settings;
  now: number;
}) {
  const c = slot.data;
  const title = nameFor(settings, "claude", "Claude");
  const badge = !c
    ? null
    : !c.signedIn
      ? "signed out"
      : (c.planName ?? (c.subscribed ? (c.subscriptionType ?? "subscribed") : "Free"));
  const badgeTone: Tone = !badge || badge === "signed out" ? "gray" : planTone(badge);
  const windowUnknown = (w: ClaudeUsage["fiveHour"] | undefined) =>
    w?.percent == null || (slot.stale && isPassed(w.resetsAt, now));
  const exhausted =
    !!c && [c.fiveHour, c.weekly].some((w) => !windowUnknown(w) && (w.percent ?? 0) >= 100);
  return (
    <Card
      label={`Claude ${title}`}
      icon={<ProviderIcon src="/claude.ico" alt="" />}
      title={title}
      titleHint={c?.email ?? undefined}
      state={exhausted ? "exhausted" : "normal"}
      hints={
        <>
          <ReauthHint expired={slot.authExpired} how="open Claude Code" />
          <CachedHint slot={slot} />
        </>
      }
      badges={
        <>
          {c?.resetCredits != null && (
            <Badge
              scope="w"
              tone={c.resetCredits > 0 ? "violet" : "gray"}
              title={`${c.resetCredits} reset${c.resetCredits === 1 ? "" : "s"} available`}
            >
              ↺ {c.resetCredits}
            </Badge>
          )}
          {badge && (
            <Badge scope="w" tone={badgeTone}>
              {badge}
            </Badge>
          )}
        </>
      }
    >
      {!c ? (
        <Unavailable slot={slot} />
      ) : !c.signedIn ? (
        <Note>Not signed in</Note>
      ) : (
        <div className="flex gap-2">
          {(
            [
              ["5h", "Five-hour usage", c.fiveHour],
              ["Weekly", "Weekly usage", c.weekly],
            ] as const
          ).map(([label, name, w]) => (
            <Meter
              key={label}
              label={label}
              name={name}
              percent={w.percent}
              unknown={windowUnknown(w)}
              reset={windowUnknown(w) ? null : formatUntil(w.resetsAt, now)}
            />
          ))}
        </div>
      )}
    </Card>
  );
}
