import { useState, type ReactNode } from "react";
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
import { CursorCard, GrokCard, planTone } from "@/widget/_components/cards";
import { slotFromAccount as widgetSlot } from "@/widget/_lib/useWidgetData";
import { cardErrorKind } from "@/lib/errors";
import type { SettingsPatch } from "@/lib/settings-schema";
import { PAGE_TITLES, PROVIDER_ICONS } from "../_lib/pages";
import { refreshWidget, type Summaries } from "../_lib/hooks";
import { orderTracked } from "../_lib/order";
import { usePanelTheme } from "../_lib/theme";
import { AccountCell, CardGrid, cardSettings, EmptyState, ProviderLogo, useNow } from "./CardGrid";
import BarsSection from "./BarsSection";
import { AccountRow, AccountTable } from "./AccountTable";
import { claudeRow, codexRow, tableColumns, type TableRowData } from "../_lib/table-rows";
import { RenameDialog } from "./dialogs";
import { CursorDiagnostics } from "./Diagnostics";
import PaceSection, { usePace } from "./PaceSection";
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

/** The widget's own card for a Cursor or Grok Bot account, fed from its cached or live usage. */
function providerCard(
  id: "cursor" | "grok",
  account: AnyAccount,
  settings: Settings,
  now: number,
  header: CardHeader,
  hidden: boolean,
  cursorNote: CursorAttemptNote | null,
) {
  // Cached usage looks like fresh usage: no status note and no dimming. A hidden account is gray.
  const extras = { active: account.active, dimmed: hidden, hidden, ...header };
  // `multiple` makes every card title the account's own label (or nickname), not the provider.
  const common = {
    settings: cardSettings(settings),
    now,
    accountId: account.id,
    multiple: true,
    extras,
  };
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
  }
}

/** A Claude or Codex account's table row; null for the providers shown as cards. */
function tableRowOf(id: ProviderId, account: AnyAccount, now: number): TableRowData | null {
  if (id === "claude") return claudeRow(account as TrackedAccount<ClaudeUsage>, now);
  if (id === "codex") return codexRow(account as TrackedAccount<CodexUsage>, now);
  return null;
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
  const pace = usePace(id === "claude");
  const name = PAGE_TITLES[id];
  const accounts = summaries[id]?.accounts ?? null;
  const hidden = (accountId: string) =>
    settings.disabledAccounts.includes(accountKey(id, accountId));
  const sorted = accounts ? orderTracked(id, accounts, hidden, settings.hiddenBars, now) : null;
  const nickname = (accountId: string) => settings.displayNames[accountKey(id, accountId)] ?? "";
  const renameAccount = accounts?.find((a) => a.id === renaming) ?? null;
  const tableProvider = id === "claude" || id === "codex" ? id : null;
  const columns =
    tableProvider && sorted
      ? tableColumns(
          tableProvider,
          sorted as TrackedAccount<ClaudeUsage | CodexUsage>[],
          settings.hiddenBars,
        )
      : [];
  const scopedLabel =
    id === "claude"
      ? accounts?.map((a) => (a.usage as ClaudeUsage | null)?.weeklyScoped[0]?.label).find(Boolean)
      : undefined;

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
        (() => {
          const cells = sorted.map((account) => {
            const key = accountKey(id, account.id);
            const isHidden = hidden(account.id);
            const shownName = nickname(account.id) || labelOf(id, account);
            const row = tableRowOf(id, account, now);
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
                  row ? (
                    <AccountRow
                      name={shownName}
                      hint={labelOf(id, account)}
                      plan={row.plan ? { label: row.plan, tone: planTone(row.plan) } : null}
                      active={account.active}
                      hidden={isHidden}
                      columns={columns}
                      cells={row.cells}
                      note={row.note}
                      resets={row.resets}
                      warning={row.warning}
                      actions={header.headerActions}
                    />
                  ) : (
                    providerCard(
                      id as "cursor" | "grok",
                      account,
                      settings,
                      now,
                      header,
                      isHidden,
                      summaries.cursor?.attempt ?? null,
                    )
                  )
                }
              />
            );
          });
          return tableProvider ? (
            <AccountTable columns={columns}>{cells}</AccountTable>
          ) : (
            <CardGrid>{cells}</CardGrid>
          );
        })()
      )}
      <BarsSection
        provider={id}
        settings={settings}
        update={update}
        scopedName={scopedLabel && `${scopedLabel} weekly`}
      />
      {id === "cursor" && <CursorDiagnostics />}
      {id === "claude" && (
        <PaceSection target={settings.claudeHourlyTarget} update={update} state={pace} />
      )}
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
