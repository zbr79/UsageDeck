import { useState } from "react";
import { toastSuccess } from "@/components/toast/toast";
import { copyKey } from "@/lib/copy-key";
import type { PeakInfo } from "@/lib/peak";
import type { OpenCodeAccountSummary, OpenCodeAccountUsage, Settings } from "@/lib/types";
import { OpenCodeCard } from "@/widget/_components/cards";
import { orderOpenCode } from "../_lib/order";
import { PROVIDER_ICONS } from "../_lib/pages";
import { refreshWidget, removeAccount } from "../_lib/hooks";
import { AccountCell, CardGrid, EmptyState, ProviderLogo, useNow } from "./CardGrid";
import { KeyDialog, RemoveDialog, RenameDialog } from "./dialogs";
import { Page, primaryButton } from "./fields";
import { accountCount } from "./ProviderPanel";

type Update = (patch: Partial<Settings>, success?: string) => Promise<boolean>;

/** Settings cards show no peak-hour hint: that is a live widget cue. */
const NO_PEAK: PeakInfo = { active: false, endAt: null, nextStartAt: null };

/** A row for an account whose usage hasn't arrived yet (the card says "Unavailable"). */
function emptyRow(email: string): OpenCodeAccountUsage {
  return {
    email,
    usage: null,
    prev: null,
    enabled: true,
    error: null,
    stale: false,
    cachedAt: null,
    subscriptionActive: null,
    lastChangeAt: null,
    lastChangeDetail: null,
  };
}

type Pending = { kind: "add" } | { kind: "rename" | "key" | "remove"; email: string };

export default function OpenCodeAccounts({
  accounts,
  usage,
  settings,
  update,
  reload,
  reloadAll,
}: {
  accounts: OpenCodeAccountSummary[] | null;
  usage: Record<string, OpenCodeAccountUsage>;
  settings: Settings;
  update: Update;
  reload: () => Promise<unknown>;
  reloadAll: () => void;
}) {
  const [pending, setPending] = useState<Pending | null>(null);
  const now = useNow();
  const disabled = (email: string) => settings.disabledAccounts.includes(email.toLowerCase());
  // The widget's order first, then hidden, lapsed and no-data accounts (../_lib/order.ts).
  const sorted = accounts
    ? orderOpenCode(
        accounts.map((a) => usage[a.email] ?? emptyRow(a.email)),
        disabled,
        now,
      ).map((row) => accounts.find((a) => a.email === row.email)!)
    : null;
  const nickname = (email: string) => settings.displayNames[email] ?? "";
  const close = () => setPending(null);
  const addButton = (
    <button type="button" onClick={() => setPending({ kind: "add" })} className={primaryButton}>
      Add account
    </button>
  );

  return (
    <Page
      title="OpenCode"
      lead={<ProviderLogo src={PROVIDER_ICONS.opencode} />}
      aside={accountCount(sorted?.length ?? null)}
      action={sorted && sorted.length > 0 ? addButton : undefined}
    >
      {sorted === null ? (
        <p className="text-sm text-(--p-muted)">Loading…</p>
      ) : sorted.length === 0 ? (
        <EmptyState>
          <p>Add an OpenCode Go API key to track its usage.</p>
          <div className="mt-4">{addButton}</div>
        </EmptyState>
      ) : (
        <CardGrid>
          {sorted.map((a) => {
            const row = usage[a.email] ?? emptyRow(a.email);
            const off = disabled(a.email);
            const lapsed = row.subscriptionActive === false;
            const lower = a.email.toLowerCase();
            return (
              <AccountCell
                key={a.email}
                name={nickname(a.email) || a.email}
                shown={!off}
                onShownChange={(next) =>
                  void update(
                    {
                      disabledAccounts: next
                        ? settings.disabledAccounts.filter((e) => e !== lower)
                        : [...settings.disabledAccounts, lower],
                    },
                    next ? "Account shown on widget" : "Account hidden from widget",
                  )
                }
                menuItems={[
                  {
                    id: "rename",
                    label: "Rename…",
                    onSelect: () => setPending({ kind: "rename", email: a.email }),
                  },
                  { id: "copy", label: "Copy API key", onSelect: () => void copyKey(a.email) },
                  {
                    id: "key",
                    label: "Replace API key…",
                    onSelect: () => setPending({ kind: "key", email: a.email }),
                  },
                  { id: "refresh", label: "Refresh now", onSelect: () => refreshWidget(reloadAll) },
                  {
                    id: "remove",
                    label: "Remove account…",
                    danger: true,
                    separatorBefore: true,
                    onSelect: () => setPending({ kind: "remove", email: a.email }),
                  },
                ]}
                card={(header) => (
                  <OpenCodeCard
                    row={row}
                    active={!off && !lapsed}
                    primary
                    settings={settings}
                    peak={NO_PEAK}
                    now={now}
                    increased={() => false}
                    onCopyKey={() => void copyKey(a.email)}
                    extras={{ dimmed: off || lapsed, hidden: off, plain: lapsed, ...header }}
                    fullEmail
                  />
                )}
              />
            );
          })}
        </CardGrid>
      )}
      {pending?.kind === "add" && <KeyDialog onClose={close} onDone={() => void reload()} />}
      {pending?.kind === "key" && (
        <KeyDialog email={pending.email} onClose={close} onDone={() => void reload()} />
      )}
      {pending?.kind === "rename" && (
        <RenameDialog
          name={pending.email}
          nickname={nickname(pending.email)}
          onSave={(next) =>
            update(
              { displayNames: { [pending.email]: next } },
              next ? "Account renamed" : "Name reset",
            )
          }
          onClose={close}
        />
      )}
      {pending?.kind === "remove" && (
        <RemoveDialog
          name={nickname(pending.email) || pending.email}
          onConfirm={async () => {
            if (!(await removeAccount(pending.email))) return false;
            toastSuccess("Account removed");
            void reload();
            return true;
          }}
          onClose={close}
        />
      )}
    </Page>
  );
}
