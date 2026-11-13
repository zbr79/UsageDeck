import { useState } from "react";
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
import type { SettingsPatch } from "@/lib/settings-schema";
import { PAGE_TITLES, PROVIDER_ICONS } from "../_lib/pages";
import { refreshWidget, type Summaries } from "../_lib/hooks";
import { orderTracked } from "../_lib/order";
import { usePanelTheme } from "../_lib/theme";
import { AccountCell, EmptyState, ProviderLogo, useNow } from "./CardGrid";
import BarsSection from "./BarsSection";
import { AccountRow, AccountTable } from "./AccountTable";
import {
  claudeRow,
  codexRow,
  cursorRow,
  EXTRA_COLUMN,
  fixedColumns,
  grokRow,
  tableColumns,
  type TableRowData,
} from "../_lib/table-rows";
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

/** An account's table row, from its cached or live usage. */
function rowOf(
  id: ProviderId,
  account: AnyAccount,
  now: number,
  cursorNote: CursorAttemptNote | null,
): TableRowData {
  switch (id) {
    case "claude":
      return claudeRow(account as TrackedAccount<ClaudeUsage>, now);
    case "codex":
      return codexRow(account as TrackedAccount<CodexUsage>, now);
    case "cursor":
      return cursorRow(
        account as TrackedAccount<CursorUsage>,
        now,
        account.active ? cursorNote : null,
      );
    case "grok":
      return grokRow(account as TrackedAccount<GrokUsage>, now);
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
  const pace = usePace(id === "claude");
  const name = PAGE_TITLES[id];
  const accounts = summaries[id]?.accounts ?? null;
  const hidden = (accountId: string) =>
    settings.disabledAccounts.includes(accountKey(id, accountId));
  const sorted = accounts ? orderTracked(id, accounts, hidden, settings.hiddenBars, now) : null;
  const nickname = (accountId: string) => settings.displayNames[accountKey(id, accountId)] ?? "";
  const renameAccount = accounts?.find((a) => a.id === renaming) ?? null;
  const columns = !sorted
    ? []
    : id === "claude" || id === "codex"
      ? tableColumns(id, sorted as TrackedAccount<ClaudeUsage | CodexUsage>[], settings.hiddenBars)
      : fixedColumns(id, settings.hiddenBars);
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
        <AccountTable columns={columns} extra={EXTRA_COLUMN[id]}>
          {sorted.map((account) => {
            const key = accountKey(id, account.id);
            const isHidden = hidden(account.id);
            const shownName = nickname(account.id) || labelOf(id, account);
            const row = rowOf(id, account, now, summaries.cursor?.attempt ?? null);
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
                row={(header) => (
                  <AccountRow
                    name={shownName}
                    hint={labelOf(id, account)}
                    plan={row.plan}
                    active={account.active}
                    hidden={isHidden}
                    dimmed={row.dimmed}
                    columns={columns}
                    cells={row.cells}
                    note={row.note}
                    extra={row.extra}
                    warning={row.warning}
                    actions={header.headerActions}
                  />
                )}
              />
            );
          })}
        </AccountTable>
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
