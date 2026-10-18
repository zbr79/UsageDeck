"use client";

import { useId, useState } from "react";
import { accountKey } from "@/lib/account-key";
import { Badge, IconButton, Switch } from "@/app/_ui/controls";
import Dialog from "@/app/_ui/Dialog";
import { CloseIcon, GearIcon } from "@/app/_ui/Icons";
import type { Tone } from "@/app/_ui/palette";
import { capitalize } from "@/lib/format";
import type {
  ClaudeUsage,
  CodexUsage,
  CursorUsage,
  GrokUsage,
  ProviderId,
  Settings,
  TrackedAccount,
} from "@/lib/types";
import type { Summaries } from "../_lib/hooks";
import { NicknameField } from "./fields";

type Update = (patch: Partial<Settings>, success?: string) => Promise<boolean>;
type AnyAccount = TrackedAccount<CursorUsage | GrokUsage | CodexUsage | ClaudeUsage>;

export const PROVIDERS: Record<
  ProviderId,
  { name: string; icon: string; enabledKey: keyof Settings & `${string}Enabled` }
> = {
  cursor: { name: "Cursor", icon: "/cursor.ico", enabledKey: "cursorEnabled" },
  grok: { name: "Grok Bot", icon: "/grokbot.ico", enabledKey: "grokEnabled" },
  codex: { name: "Codex", icon: "/openai.png", enabledKey: "codexEnabled" },
  claude: { name: "Claude", icon: "/claude.ico", enabledKey: "claudeEnabled" },
};

function labelOf(id: ProviderId, account: AnyAccount): string {
  if (account.label) return account.label;
  const usage = account.usage;
  if (!usage) return "Signed-in account";
  if (id === "cursor") return (usage as CursorUsage).accountName ?? "Signed-in account";
  if (id === "grok") return (usage as GrokUsage).email ?? "Signed-in account";
  if (id === "codex") return (usage as CodexUsage).email ?? "Signed-in account";
  return (usage as ClaudeUsage).email ?? (usage as ClaudeUsage).displayName ?? "Signed-in account";
}

function planOf(id: ProviderId, account: AnyAccount): { text: string; tone: Tone } | null {
  if (id === "cursor") {
    const plan = (account.usage as CursorUsage | null)?.planName;
    return plan ? { text: plan, tone: "blue" } : null;
  }
  if (id === "grok") {
    const g = account.usage as GrokUsage | null;
    const plan = g?.planLabel ?? g?.cursorPlanName;
    return plan ? { text: plan, tone: "blue" } : null;
  }
  if (id === "codex") {
    const plan = (account.usage as CodexUsage | null)?.planType;
    if (!plan) return null;
    return { text: capitalize(plan), tone: plan.toLowerCase() === "free" ? "green" : "blue" };
  }
  const c = account.usage as ClaudeUsage | null;
  if (!account.usage) return null;
  const plan = !c
    ? null
    : !c.signedIn
      ? "not signed in"
      : (c.planName ?? (c.subscribed ? (c.subscriptionType ?? "subscribed") : "Free"));
  if (!plan) return null;
  const tone: Tone =
    plan === "Free"
      ? "green"
      : plan === "not signed in"
        ? "gray"
        : plan.toLowerCase() === "max 20x"
          ? "amber"
          : "blue";
  return { text: plan, tone };
}

function AccountDialog({
  providerId,
  account,
  settings,
  update,
  onClose,
}: {
  providerId: ProviderId;
  account: AnyAccount;
  settings: Settings;
  update: Update;
  onClose: () => void;
}) {
  const titleId = useId();
  const switchId = useId();
  const { name } = PROVIDERS[providerId];
  const key = accountKey(providerId, account.id);
  const label = labelOf(providerId, account);
  const plan = planOf(providerId, account);
  const shown = !settings.disabledAccounts.includes(key);
  const toggle = (next: boolean) =>
    void update(
      {
        disabledAccounts: next
          ? settings.disabledAccounts.filter((e) => e !== key)
          : [...settings.disabledAccounts, key],
      },
      next ? "Account shown on widget" : "Account hidden from widget",
    );
  return (
    <Dialog labelledBy={titleId} onClose={onClose}>
      <div className="bg-(--p-bg) rounded-xl shadow-2xl w-[420px] border border-(--p-border) p-5">
        <div className="flex items-start justify-between mb-4 gap-2">
          <div className="min-w-0">
            <div className="flex items-center gap-1.5">
              <h3 id={titleId} className="text-base font-bold">
                {name}
              </h3>
              {plan && <Badge tone={plan.tone}>{plan.text}</Badge>}
            </div>
            <div className="text-xs text-(--p-muted) truncate">{label}</div>
          </div>
          <IconButton label="Close" onClick={onClose}>
            <CloseIcon size={16} />
          </IconButton>
        </div>
        <div className="space-y-3">
          <div className="flex items-center gap-2">
            <Switch labelledBy={switchId} checked={shown} onChange={toggle} />
            <span id={switchId} className="text-sm text-(--p-textSecondary)">
              Show this account
            </span>
          </div>
          <NicknameField
            value={settings.displayNames[key] ?? ""}
            onSave={(nick) => update({ displayNames: { [key]: nick } }, "Nickname saved")}
          />
          {account.authExpired && (
            <p className="text-xs text-(--p-muted)">
              The sign-in this app reads has expired. Open {name} to refresh it — this monitor never
              refreshes or writes another app&apos;s credentials.
            </p>
          )}
          {!account.active && (
            <p className="text-xs text-(--p-muted)">
              {name} is signed into a different account, so this one stays off the widget until you
              switch to it.
            </p>
          )}
        </div>
      </div>
    </Dialog>
  );
}

export default function ProviderPanel({
  id,
  summaries,
  settings,
  update,
}: {
  id: ProviderId;
  summaries: Summaries;
  settings: Settings;
  update: Update;
}) {
  const showId = useId();
  const [detail, setDetail] = useState<string | null>(null);
  const { name, enabledKey } = PROVIDERS[id];
  const accounts = summaries[id]?.accounts ?? null;
  const hidden = (accountId: string) =>
    settings.disabledAccounts.includes(accountKey(id, accountId));
  const sorted = accounts
    ? [...accounts].sort(
        (a, b) =>
          Number(hidden(a.id)) - Number(hidden(b.id)) || Number(b.active) - Number(a.active),
      )
    : null;
  const detailAccount = accounts?.find((a) => a.id === detail) ?? null;

  return (
    <div>
      <div className="flex items-center gap-2 mb-3">
        <Switch
          labelledBy={showId}
          checked={settings[enabledKey]}
          onChange={(next) =>
            void update(
              { [enabledKey]: next },
              `${name} ${next ? "shown on" : "hidden from"} widget`,
            )
          }
        />
        <span id={showId} className="text-sm text-(--p-textSecondary)">
          Active
        </span>
      </div>
      {sorted === null ? (
        <p className="text-sm text-(--p-muted)">Loading…</p>
      ) : sorted.length === 0 ? (
        <p className="text-sm text-(--p-muted)">
          No {name} account yet. Sign in with {name} and it will show up here.
        </p>
      ) : (
        <ul className="space-y-2">
          {sorted.map((account) => {
            const label = labelOf(id, account);
            const plan = planOf(id, account);
            return (
              <li
                key={account.id}
                className={`border rounded-lg px-3 py-2 ${
                  account.active
                    ? "bg-(--p-accountActiveBg) border-(--p-accountActiveBorder)"
                    : "bg-(--p-accountInactiveBg) border-transparent"
                }`}
              >
                <div className="flex items-center gap-2">
                  <button
                    type="button"
                    onClick={() => setDetail(account.id)}
                    className={`flex-1 min-w-0 text-left text-sm truncate hover:underline ${
                      account.active ? "text-(--p-text)" : "text-(--p-muted)"
                    }`}
                  >
                    {label}
                  </button>
                  {hidden(account.id) && account.active && <Badge tone="gray">hidden</Badge>}
                  {account.authExpired && (
                    <Badge tone="amber" title={`Sign-in expired · open ${name} to refresh it`}>
                      re-auth
                    </Badge>
                  )}
                  {plan && <Badge tone={plan.tone}>{plan.text}</Badge>}
                  <IconButton label={`Details for ${label}`} onClick={() => setDetail(account.id)}>
                    <GearIcon />
                  </IconButton>
                </div>
                <div
                  aria-hidden={!settings.displayNames[accountKey(id, account.id)]}
                  className="mt-1 h-4 text-xs text-(--p-text) truncate"
                >
                  {settings.displayNames[accountKey(id, account.id)] ?? ""}
                </div>
              </li>
            );
          })}
        </ul>
      )}
      {detailAccount && (
        <AccountDialog
          providerId={id}
          account={detailAccount}
          settings={settings}
          update={update}
          onClose={() => setDetail(null)}
        />
      )}
    </div>
  );
}
