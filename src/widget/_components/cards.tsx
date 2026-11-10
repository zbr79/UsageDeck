import type { ReactNode } from "react";
import { Badge, HIT_24 } from "@/ui/controls";
import { CopyIcon, EyeOffIcon } from "@/ui/Icons";
import { WIDGET_SPEND_TONE, type Tone } from "@/ui/palette";
import { errorHint, isAuthRejected } from "@/lib/errors";
import {
  capitalize,
  claudePlanLabel,
  formatDuration,
  formatResetDuration,
  formatUntil,
} from "@/lib/format";
import { headerRate, meterPaceText, SPEND_BAND_TONE, spendBand, spendRateTitle } from "@/lib/pace";
import type { PeakInfo } from "@/lib/peak";
import { isExhaustedWindow, isPassed, isUnknownWindow } from "@/lib/ranking";
import { openCodeUsedUp, usedUp, type UsedUp } from "@/lib/used-up";
import type {
  ClaudePace,
  ClaudeUsage,
  CodexUsage,
  CursorUsage,
  GrokUsage,
  OpenCodeAccountUsage,
  PaceWindow,
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

/**
 * Header hints next to the account name. The widget is 238 px wide: when several hints show,
 * they wrap onto a second line (this group shrinks first, down to its widest hint) so the
 * account name keeps its width.
 */
function HintGroup({ children }: { children: ReactNode }) {
  return <span className="flex flex-wrap items-center gap-x-1.5 shrink-[100]">{children}</span>;
}

/** Re-auth hint for a rejected sign-in, else a short hint for why the last fetch failed. */
function SlotHints<T>({ slot, how }: { slot: ProviderSlot<T>; how: string }) {
  if (isAuthRejected(slot.authExpired, slot.error)) {
    return (
      <Hint color="warn" title={`Sign-in expired · ${how} to refresh it`}>
        re-auth
      </Hint>
    );
  }
  const failure = errorHint(slot.error, how);
  if (!failure) return null;
  return (
    <HintGroup>
      <Hint color="warn" title={failure.long}>
        {failure.short}
      </Hint>
    </HintGroup>
  );
}

function planTone(plan: string): Tone {
  const normalized = plan.toLowerCase();
  if (normalized === "free") return "green";
  if (normalized === "ultra") return "amber";
  return "blue";
}

/**
 * Widget: on a used-up card, hide it until it resets. Shown only when the reset time is known
 * and still ahead, so the card is sure to come back.
 */
function HideButton({
  name,
  used,
  now,
  onHide,
}: {
  name: string;
  used: UsedUp | null;
  now: number;
  onHide: ((until: number) => void) | undefined;
}) {
  const until = used?.until;
  if (!onHide || until == null || until <= now) return null;
  const label = `Hide ${name} until it resets in ${formatResetDuration(until - now)}`;
  return (
    <button
      type="button"
      onClick={() => onHide(until)}
      aria-label={label}
      title={label}
      className={`${HIT_24} shrink-0 rounded-[3px] p-0.5 -m-0.5 text-(--w-muted) hover:bg-(--w-hover) hover:text-(--w-text) transition-colors`}
    >
      <EyeOffIcon size={12} />
    </button>
  );
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
  onHide,
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
  /** widget: hide this used-up card until the given time */
  onHide?: (until: number) => void;
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
  const used = openCodeUsedUp(row, now);
  const state: CardState = used ? "exhausted" : active ? "active" : "normal";
  const unknown = (w: "rolling" | "weekly" | "monthly") =>
    !!u && isUnknownWindow(u[w], row.stale, now);
  const reset = (w: "rolling" | "weekly" | "monthly", show: boolean) =>
    u && show && !unknown(w) ? formatUntil(u[w].resetsAt, now) : null;

  const rejected = isAuthRejected(row.authExpired, row.error);
  const failure = errorHint(row.error, "add the key again in Settings");
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
          className={`group/copy ${HIT_24} shrink-0 rounded-[3px] p-0.5 -m-0.5 hover:bg-(--w-hover)`}
        >
          <ProviderIcon src="/opencode.ico" alt="" />
          <span className="absolute inset-0 flex items-center justify-center rounded-[3px] bg-(--w-card) text-(--w-text) opacity-0 group-hover/copy:opacity-100 group-focus-visible/copy:opacity-100 transition-opacity">
            <CopyIcon size={10} />
          </span>
        </button>
      }
      title={title}
      titleHint={row.email}
      titleAction={<HideButton name={title} used={used} now={now} onHide={onHide} />}
      hints={
        (rejected || failure || peakHint) && (
          <HintGroup>
            {rejected && (
              <Hint color="warn" title="OpenCode rejected this API key · add a new key in Settings">
                key rejected
              </Hint>
            )}
            {failure && (
              <Hint color="warn" title={failure.long}>
                {failure.short}
              </Hint>
            )}
            {peakHint}
          </HintGroup>
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
            name="Rolling 5-hour window"
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
            name="Weekly"
            percent={u.weekly.percent}
            unknown={unknown("weekly")}
            dimmed={isExhaustedWindow(u.monthly, row.stale, now)}
            increased={increased("weekly")}
            reset={reset("weekly", primary)}
          />
          <Meter
            label="M"
            name="Monthly"
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
  onHide,
  extras,
}: {
  slot: ProviderSlot<CursorUsage>;
  settings: Settings;
  now: number;
  accountId?: string | null;
  accountLabel?: string | null;
  multiple?: boolean;
  /** widget: hide this used-up card until the given time */
  onHide?: (until: number) => void;
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
  const used = usedUp("cursor", { usage: c, stale: slot.stale }, now);
  return (
    <Card
      extras={extras}
      label={`Cursor ${title}`}
      icon={<ProviderIcon src="/cursor.ico" alt="" />}
      title={title}
      titleHint={c?.accountName ?? undefined}
      state={used ? "exhausted" : "normal"}
      titleAction={<HideButton name={title} used={used} now={now} onHide={onHide} />}
      hints={<SlotHints slot={slot} how="open Cursor" />}
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
  onHide,
  extras,
}: {
  slot: ProviderSlot<GrokUsage>;
  settings: Settings;
  now: number;
  accountId?: string | null;
  accountLabel?: string | null;
  multiple?: boolean;
  /** widget: hide this used-up card until the given time */
  onHide?: (until: number) => void;
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
  const used = usedUp("grok", { usage: g, stale: slot.stale }, now);
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
      state={used ? "exhausted" : "normal"}
      titleAction={<HideButton name={title} used={used} now={now} onHide={onHide} />}
      hints={<SlotHints slot={slot} how="open Grok Bot" />}
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
            <Meter label="Weekly" name="Weekly" percent={g.percentUsed} unknown={passed} />
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
  onHide,
  extras,
}: {
  slot: ProviderSlot<CodexUsage>;
  settings: Settings;
  now: number;
  light: boolean;
  accountId?: string | null;
  accountLabel?: string | null;
  multiple?: boolean;
  /** widget: hide this used-up card until the given time */
  onHide?: (until: number) => void;
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
  const fiveHour = hasWeekly && x?.limitWindowSeconds === 18000;
  const weeklyPassed = slot.stale && isPassed(x?.weeklyResetAt, now);
  const used = usedUp("codex", { usage: x, stale: slot.stale }, now);
  return (
    <Card
      extras={extras}
      label={`Codex ${title}`}
      icon={<ProviderIcon src="/openai.png" alt="" invert={light} />}
      title={title}
      titleHint={x?.email ?? undefined}
      state={used ? "exhausted" : "normal"}
      titleAction={<HideButton name={title} used={used} now={now} onHide={onHide} />}
      hints={<SlotHints slot={slot} how="run codex" />}
      badges={
        <>
          {x?.resetCredits != null && (
            <Badge
              scope="w"
              tone={x.resetCredits > 0 ? "violet" : "gray"}
              label={`Reset credits available: ${x.resetCredits}`}
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
              label={fiveHour ? "5h" : "Usage"}
              name={fiveHour ? "Rolling 5-hour window" : "Codex usage"}
              percent={x.usedPercent}
              unknown={passed}
              reset={hasWeekly && x.resetAt && !passed ? formatUntil(x.resetAt, now) : undefined}
            />
            {hasWeekly && (
              <Meter
                label="Weekly"
                name="Weekly"
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

/**
 * A Claude meter's pace: the fill severity and the tooltip details. No even-pace tick: the owner
 * found it confusing (nobody aims to use a whole window exactly by its reset).
 */
export function ClaudeCard({
  slot,
  settings,
  now,
  accountId = null,
  accountLabel = null,
  multiple = false,
  pace = null,
  onHide,
  extras,
}: {
  slot: ProviderSlot<ClaudeUsage>;
  /** spend and pace of Claude Code on this PC; one card only shows the rate, null shows none */
  pace?: ClaudePace | null;
  settings: Settings;
  now: number;
  accountId?: string | null;
  accountLabel?: string | null;
  multiple?: boolean;
  /** widget: hide this used-up card until the given time */
  onHide?: (until: number) => void;
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
      : claudePlanLabel(
          c.planName ?? (c.subscribed ? (c.subscriptionType ?? "subscribed") : "Free"),
        );
  const badgeTone: Tone = !badge || badge === "signed out" ? "gray" : planTone(badge);
  const windowUnknown = (w: ClaudeUsage["fiveHour"] | undefined) =>
    w?.percent == null || (slot.stale && isPassed(w.resetsAt, now));
  const used = usedUp("claude", { usage: c, stale: slot.stale }, now);
  const rate = headerRate(pace);
  const target = settings.claudeHourlyTarget;
  const rate60m = pace?.spend.rate60m ?? 0;
  const rateTone = SPEND_BAND_TONE[spendBand(rate60m, target)];
  const rateTitle = spendRateTitle(rate60m, target);
  return (
    <Card
      extras={extras}
      label={`Claude ${title}`}
      icon={<ProviderIcon src="/claude.ico" alt="" />}
      title={title}
      titleHint={c?.email ?? undefined}
      titleAfter={
        rate && (
          <span
            data-testid="claude-rate"
            data-tone={rateTone}
            title={rateTitle}
            className="shrink-0 whitespace-nowrap text-[12px] leading-4 tabular-nums text-(--w-muted) transition-colors duration-700"
          >
            <span
              aria-hidden="true"
              style={
                rateTone === "muted"
                  ? undefined
                  : { color: `var(--w-${WIDGET_SPEND_TONE[rateTone]})` }
              }
            >
              {rate}
            </span>
            <span className="sr-only">{`${rate}: ${rateTitle}`}</span>
          </span>
        )
      }
      state={used ? "exhausted" : "normal"}
      titleAction={<HideButton name={title} used={used} now={now} onHide={onHide} />}
      hints={<SlotHints slot={slot} how="open Claude Code" />}
      badges={
        <>
          {c?.resetCredits != null && (
            <Badge
              scope="w"
              tone={c.resetCredits > 0 ? "violet" : "gray"}
              label={`Reset credits available: ${c.resetCredits}`}
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
        <>
          <div className="flex gap-2">
            {(
              [
                ["5h", "Rolling 5-hour window", c.fiveHour, "fiveHour"],
                ["Weekly", "Weekly", c.weekly, "weekly"],
              ] as const
            ).map(([label, name, w, key]) => {
              const unknown = windowUnknown(w);
              const pw = pace?.windows[key] ?? null;
              return (
                <Meter
                  key={label}
                  label={label}
                  name={name}
                  percent={w.percent}
                  unknown={unknown}
                  reset={unknown ? null : formatUntil(w.resetsAt, now)}
                  detail={unknown ? null : meterPaceText(w.percent!, null, pw, now)}
                />
              );
            })}
          </div>
        </>
      )}
    </Card>
  );
}
