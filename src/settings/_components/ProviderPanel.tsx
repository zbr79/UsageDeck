import { useId, useState } from "react";
import { accountKey } from "@/lib/account-key";
import { Badge, IconButton, Switch } from "@/ui/controls";
import Dialog from "@/ui/Dialog";
import { CloseIcon, GearIcon } from "@/ui/Icons";
import type { Tone } from "@/ui/palette";
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
import type { CardExtras } from "@/widget/_components/Card";
import { ClaudeCard, CodexCard, CursorCard, GrokCard } from "@/widget/_components/cards";
import { slotFromAccount } from "@/widget/_lib/useWidgetData";
import type { Summaries } from "../_lib/hooks";
import { CardFooter, CardGrid, updatedNote, useNow } from "./CardGrid";
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

/** The widget's own card for one account, fed from its cached or live usage. */
function providerCard(
  id: ProviderId,
  account: AnyAccount,
  settings: Settings,
  now: number,
  extras: CardExtras,
) {
  // `multiple` makes every card title the account's own label (or nickname), not the provider.
  const common = { settings, now, accountId: account.id, multiple: true, extras };
  const label = account.label;
  switch (id) {
    case "cursor": {
      const a = account as TrackedAccount<CursorUsage>;
      return (
        <CursorCard
          slot={slotFromAccount(a)}
          accountLabel={label ?? a.usage?.accountName}
          {...common}
        />
      );
    }
    case "grok": {
      const a = account as TrackedAccount<GrokUsage>;
      return (
        <GrokCard slot={slotFromAccount(a)} accountLabel={label ?? a.usage?.email} {...common} />
      );
    }
    case "codex": {
      const a = account as TrackedAccount<CodexUsage>;
      return (
        <CodexCard
          slot={slotFromAccount(a)}
          accountLabel={label ?? a.usage?.email}
          light={settings.panelTheme === "light"}
          {...common}
        />
      );
    }
    case "claude": {
      const a = account as TrackedAccount<ClaudeUsage>;
      return (
        <ClaudeCard slot={slotFromAccount(a)} accountLabel={label ?? a.usage?.email} {...common} />
      );
    }
  }
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
  const now = useNow();
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
        <CardGrid>
          {sorted.map((account) => {
            const label = labelOf(id, account);
            const isHidden = hidden(account.id);
            const updated = updatedNote(account.cachedAt, now);
            const note = account.active
              ? account.stale && updated
                ? `Signed in · ${updated}`
                : "Signed in"
              : `Not signed in · ${updated ?? "nothing cached"}`;
            const extras: CardExtras = {
              active: account.active,
              dimmed: !account.active,
              dashed: isHidden,
              footer: (
                <CardFooter note={note} hidden={isHidden}>
                  <IconButton label={`Details for ${label}`} onClick={() => setDetail(account.id)}>
                    <GearIcon size={12} />
                  </IconButton>
                </CardFooter>
              ),
            };
            return (
              <li key={account.id} className="min-w-0">
                {providerCard(id, account, settings, now, extras)}
              </li>
            );
          })}
        </CardGrid>
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
