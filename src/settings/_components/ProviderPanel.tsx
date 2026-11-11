import { useState } from "react";
import type { CardHeader } from "@/widget/_components/Card";
import { accountKey } from "@/lib/account-key";
import type {
  ClaudeUsage,
  CodexUsage,
  CursorAttemptNote,
  CursorUsage,
  GrokUsage,
  ProviderId,
  Settings,
  TrackedAccount,
} from "@/lib/types";
import { ClaudeCard, CodexCard, CursorCard, GrokCard } from "@/widget/_components/cards";
import { slotFromAccount as widgetSlot } from "@/widget/_lib/useWidgetData";
import { cardErrorKind } from "@/lib/errors";
import type { SettingsPatch } from "@/lib/settings-schema";
import { PAGE_TITLES, PROVIDER_ICONS } from "../_lib/pages";
import { refreshWidget, type Summaries } from "../_lib/hooks";
import { orderTracked } from "../_lib/order";
import { usePanelTheme } from "../_lib/theme";
import { AccountCell, CardGrid, EmptyState, ProviderLogo, useNow } from "./CardGrid";
import { RenameDialog } from "./dialogs";
import { CursorDiagnostics } from "./Diagnostics";
import PaceSection from "./PaceSection";
import { Page } from "./fields";

type Update = (patch: SettingsPatch, success?: string) => Promise<boolean>;
type AnyAccount = TrackedAccount<CursorUsage | GrokUsage | CodexUsage | ClaudeUsage>;

/** The account's own name: the label Rust gives it, else the provider's account name / email. */
function labelOf(id: ProviderId, account: AnyAccount): string {
  if (account.label) return account.label;
  const usage = account.usage;
  if (!usage) return "Signed-in account";
  if (id === "cursor") return (usage as CursorUsage).accountName ?? "Signed-in account";
  if (id === "grok") return (usage as GrokUsage).email ?? "Signed-in account";
  if (id === "codex") return (usage as CodexUsage).email ?? "Signed-in account";
  return (usage as ClaudeUsage).email ?? (usage as ClaudeUsage).displayName ?? "Signed-in account";
}

/** The widget's slot for an account; a missing sign-in is named only on the signed-in account. */
function slotFromAccount<T>(account: TrackedAccount<T>) {
  return { ...widgetSlot(account), error: cardErrorKind(account.error, account.active) };
}

/** The widget's own card for one account, fed from its cached or live usage. */
function providerCard(
  id: ProviderId,
  account: AnyAccount,
  settings: Settings,
  now: number,
  header: CardHeader,
  hidden: boolean,
  light: boolean,
  cursorNote: CursorAttemptNote | null,
) {
  // Cached usage looks like fresh usage: no status note and no dimming. A hidden account is gray.
  const extras = { active: account.active, dimmed: hidden, hidden, ...header };
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
          note={a.active ? cursorNote : null}
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
          light={light}
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

/** "1 account" / "3 accounts". */
export function accountCount(n: number | null): string | null {
  if (n === null) return null;
  return `${n} ${n === 1 ? "account" : "accounts"}`;
}

/** Account page for a provider whose sign-in UsageDeck reads (Claude, Codex, Cursor, Grok Bot). */
export default function ProviderPanel({
  id,
  summaries,
  settings,
  update,
}: {
  id: ProviderId;
  summaries: Summaries & { reloadAll: () => void };
  settings: Settings;
  update: Update;
}) {
  const now = useNow();
  const light = usePanelTheme() === "light";
  const [renaming, setRenaming] = useState<string | null>(null);
  const name = PAGE_TITLES[id];
  const accounts = summaries[id]?.accounts ?? null;
  const hidden = (accountId: string) =>
    settings.disabledAccounts.includes(accountKey(id, accountId));
  const sorted = accounts ? orderTracked(id, accounts, hidden, now) : null;
  const nickname = (accountId: string) => settings.displayNames[accountKey(id, accountId)] ?? "";
  const renameAccount = accounts?.find((a) => a.id === renaming) ?? null;

  return (
    <Page
      title={name}
      lead={<ProviderLogo src={PROVIDER_ICONS[id]} invert={id === "codex" && light} />}
      aside={accountCount(sorted?.length ?? null)}
    >
      {sorted === null ? (
        <p className="text-sm text-(--p-muted)">Loading…</p>
      ) : sorted.length === 0 ? (
        <EmptyState>
          UsageDeck reads {name}&apos;s own sign-in. Sign in with {name} and the account shows up
          here.
        </EmptyState>
      ) : (
        <CardGrid>
          {sorted.map((account) => {
            const key = accountKey(id, account.id);
            const isHidden = hidden(account.id);
            const shownName = nickname(account.id) || labelOf(id, account);
            return (
              <AccountCell
                key={account.id}
                name={shownName}
                shown={!isHidden}
                onShownChange={(next) =>
                  void update(
                    {
                      disabledAccounts: next
                        ? settings.disabledAccounts.filter((e) => e !== key)
                        : [...settings.disabledAccounts, key],
                    },
                    next ? "Account shown on widget" : "Account hidden from widget",
                  )
                }
                menuItems={[
                  { id: "rename", label: "Rename…", onSelect: () => setRenaming(account.id) },
                  {
                    id: "refresh",
                    label: "Refresh now",
                    onSelect: () => refreshWidget(summaries.reloadAll),
                  },
                ]}
                card={(header) =>
                  providerCard(
                    id,
                    account,
                    settings,
                    now,
                    header,
                    isHidden,
                    light,
                    summaries.cursor?.attempt ?? null,
                  )
                }
              />
            );
          })}
        </CardGrid>
      )}
      {id === "cursor" && <CursorDiagnostics />}
      {id === "claude" && <PaceSection target={settings.claudeHourlyTarget} update={update} />}
      {renameAccount && (
        <RenameDialog
          name={labelOf(id, renameAccount)}
          nickname={nickname(renameAccount.id)}
          onSave={(next) =>
            update(
              { displayNames: { [accountKey(id, renameAccount.id)]: next } },
              next ? "Account renamed" : "Name reset",
            )
          }
          onClose={() => setRenaming(null)}
        />
      )}
    </Page>
  );
}
