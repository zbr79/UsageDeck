import { Badge } from "@/ui/controls";
import { CopyIcon } from "@/ui/Icons";
import type { Tone } from "@/ui/palette";
import { capitalize, formatDuration, formatUntil } from "@/lib/format";
import type { PeakInfo } from "@/lib/peak";
import { isAccountExhausted, isExhaustedWindow, isPassed, isUnknownWindow } from "@/lib/ranking";
import type {
  ClaudeUsage,
  CodexUsage,
  CursorUsage,
  GrokUsage,
  OpenCodeAccountUsage,
  ProviderId,
  Settings,
} from "@/lib/types";
import type { ProviderSlot } from "../_lib/useWidgetData";
import {
  Card,
  Hint,
  Meter,
  Note,
  ProviderIcon,
  ResetLine,
  type CardExtras,
  type CardState,
} from "./Card";

type NameSettings = Pick<Settings, "displayNames" | "showProviderNames">;

function nameFor(s: NameSettings, key: string, providerName: string, fallback?: string | null) {
  if (s.showProviderNames) return providerName;
  return s.displayNames[key] ?? fallback ?? providerName;
}

/**
 * Per-account nickname wins. A single account keeps the old title (Cursor
 * shows the email, the others show the provider name) so one sign-in looks
 * the same as before. Extra accounts show their own email.
 */
export function accountTitle(
  s: NameSettings,
  provider: ProviderId,
  accountId: string | null,
  providerName: string,
  label: string | null | undefined,
  multiple: boolean,
) {
  if (s.showProviderNames) return providerName;
  if (accountId) {
    const nick = s.displayNames[`${provider}:${accountId}`];
    if (nick) return nick;
  }
  if (multiple && label) return label;
  if (s.displayNames[provider]) return s.displayNames[provider];
  if (provider === "cursor" && label) return label;
  return providerName;
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
  const normalized = plan.toLowerCase();
  if (normalized === "free") return "green";
  if (normalized === "max 20x") return "amber";
  return "blue";
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
  extras,
  fullEmail = false,
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
  /** settings-only additions; the widget passes none */
  extras?: CardExtras;
  /** without a nickname, title the card with the whole email (settings), not its first letters */
  fullEmail?: boolean;
}) {
  const title = nameFor(
    settings,
    row.email,
    "OpenCode",
    fullEmail ? row.email : row.email.slice(0, 4),
  );
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
      extras={extras}
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
        (row.authExpired || peakHint) && (
          // The widget is 238 px wide: when both hints show, they wrap onto a second line (this
          // group shrinks first, down to its widest hint) so the account name keeps its width.
          <span className="flex flex-wrap items-center gap-x-1.5 shrink-[100]">
            {row.authExpired && (
              <Hint color="warn" title="OpenCode rejected this API key · add a new key in Settings">
                key rejected
              </Hint>
            )}
            {peakHint}
          </span>
        )
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
  accountId = null,
  accountLabel = null,
  multiple = false,
  extras,
}: {
  slot: ProviderSlot<CursorUsage>;
  settings: Settings;
  now: number;
  accountId?: string | null;
  accountLabel?: string | null;
  multiple?: boolean;
  /** settings-only additions; the widget passes none */
  extras?: CardExtras;
}) {
  const c = slot.data;
  const title = accountTitle(
    settings,
    "cursor",
    accountId,
    "Cursor",
    accountLabel ?? c?.accountName,
    multiple,
  );
  const cyclePassed = slot.stale && isPassed(c?.billingCycleEnd, now);
  const exhausted = !!c && !cyclePassed && (c.totalPercentUsed ?? 0) >= 100;
  return (
    <Card
      extras={extras}
      label={`Cursor ${title}`}
      icon={<ProviderIcon src="/cursor.ico" alt="" />}
      title={title}
      titleHint={c?.accountName ?? undefined}
      state={exhausted ? "exhausted" : "normal"}
      hints={
        <>
          <ReauthHint expired={slot.authExpired} how="open Cursor" />
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
  accountId = null,
  accountLabel = null,
  multiple = false,
  extras,
}: {
  slot: ProviderSlot<GrokUsage>;
  settings: Settings;
  now: number;
  accountId?: string | null;
  accountLabel?: string | null;
  multiple?: boolean;
  /** settings-only additions; the widget passes none */
  extras?: CardExtras;
}) {
  const g = slot.data;
  const title = accountTitle(
    settings,
    "grok",
    accountId,
    "Grok Bot",
    accountLabel ?? g?.email,
    multiple,
  );
  const passed = slot.stale && isPassed(g?.resetAt, now);
  const exhausted = !!g && !passed && (g.percentUsed ?? 0) >= 100;
  const plan = g?.planLabel ?? g?.cursorPlanName;
  const account = g?.email
    ? `${g.email}${g.source === "cursor" ? " (via Cursor sign-in)" : ""}`
    : undefined;
  return (
    <Card
      extras={extras}
      label={`Grok Bot ${title}`}
      icon={<ProviderIcon src="/grokbot.ico" alt="" />}
      title={title}
      titleHint={account}
      state={exhausted ? "exhausted" : "normal"}
      hints={
        <>
          <ReauthHint expired={slot.authExpired} how="open Grok Bot" />
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
  accountId = null,
  accountLabel = null,
  multiple = false,
  extras,
}: {
  slot: ProviderSlot<CodexUsage>;
  settings: Settings;
  now: number;
  light: boolean;
  accountId?: string | null;
  accountLabel?: string | null;
  multiple?: boolean;
  /** settings-only additions; the widget passes none */
  extras?: CardExtras;
}) {
  const x = slot.data;
  const title = accountTitle(
    settings,
    "codex",
    accountId,
    "Codex",
    accountLabel ?? x?.email,
    multiple,
  );
  const passed = slot.stale && isPassed(x?.resetAt, now);
  const hasWeekly = x?.weeklyUsedPercent != null;
  const weeklyPassed = slot.stale && isPassed(x?.weeklyResetAt, now);
  const exhausted =
    !!x &&
    ((!passed && (x.limitReached || (x.usedPercent ?? 0) >= 100)) ||
      (hasWeekly && !weeklyPassed && (x.weeklyUsedPercent ?? 0) >= 100));
  return (
    <Card
      extras={extras}
      label={`Codex ${title}`}
      icon={<ProviderIcon src="/openai.png" alt="" invert={light} />}
      title={title}
      titleHint={x?.email ?? undefined}
      state={exhausted ? "exhausted" : "normal"}
      hints={
        <>
          <ReauthHint expired={slot.authExpired} how="run codex" />
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
            <Meter
              label={hasWeekly && x.limitWindowSeconds === 18000 ? "5h" : "Usage"}
              name="Codex usage"
              percent={x.usedPercent}
              unknown={passed}
              reset={hasWeekly && x.resetAt && !passed ? formatUntil(x.resetAt, now) : undefined}
            />
            {hasWeekly && (
              <Meter
                label="Weekly"
                name="Codex weekly usage"
                percent={x.weeklyUsedPercent}
                unknown={weeklyPassed}
                reset={x.weeklyResetAt && !weeklyPassed ? formatUntil(x.weeklyResetAt, now) : null}
              />
            )}
          </div>
          {!hasWeekly && x.resetAt && !passed && (
            <ResetLine>Resets {formatUntil(x.resetAt, now)}</ResetLine>
          )}
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
  accountId = null,
  accountLabel = null,
  multiple = false,
  extras,
}: {
  slot: ProviderSlot<ClaudeUsage>;
  settings: Settings;
  now: number;
  accountId?: string | null;
  accountLabel?: string | null;
  multiple?: boolean;
  /** settings-only additions; the widget passes none */
  extras?: CardExtras;
}) {
  const c = slot.data;
  const title = accountTitle(
    settings,
    "claude",
    accountId,
    "Claude",
    accountLabel ?? c?.email,
    multiple,
  );
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
      extras={extras}
      label={`Claude ${title}`}
      icon={<ProviderIcon src="/claude.ico" alt="" />}
      title={title}
      titleHint={c?.email ?? undefined}
      state={exhausted ? "exhausted" : "normal"}
      hints={
        <>
          <ReauthHint expired={slot.authExpired} how="open Claude Code" />
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
