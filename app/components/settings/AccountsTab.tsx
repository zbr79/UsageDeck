"use client";

import ProviderIcon from "@/app/components/ui/ProviderIcon";
import Tabs, { tabDomId, tabPanelDomId, type TabItem } from "@/app/components/ui/Tabs";
import OpenCodeAccountsPanel from "./OpenCodeAccountsPanel";
import ProviderAccountPanel from "./ProviderAccountPanel";
import type { AccountRow, ProviderTab, SubProvider } from "./types";

export default function AccountsTab({
  tab,
  onSelectTab,
  panelTheme,
  accounts,
  disabledAccounts,
  displayNames,
  cursorAccount,
  cursorPlan,
  codexAccount,
  codexPlan,
  claudeAccount,
  claudeStatus,
  confirmRemove,
  now,
  onAdd,
  onOpenDetail,
  onConfirmRemove,
  onRemoveAccount,
  onOpenSubDetail,
}: {
  tab: ProviderTab;
  onSelectTab: (t: ProviderTab) => void;
  panelTheme?: "light" | "dark";
  accounts: AccountRow[] | null;
  disabledAccounts: string[];
  displayNames: Record<string, string>;
  cursorAccount: string | null;
  cursorPlan: string | null;
  codexAccount: string | null;
  codexPlan: string | null;
  claudeAccount: string | null;
  claudeStatus: string | null;
  confirmRemove: string | null;
  now: number;
  onAdd: () => void;
  onOpenDetail: (email: string) => void;
  onConfirmRemove: (email: string | null) => void;
  onRemoveAccount: (email: string) => void;
  onOpenSubDetail: (p: SubProvider) => void;
}) {
  const providerTabs: TabItem<ProviderTab>[] = [
    {
      id: "opencode",
      label: "OpenCode",
      icon: <ProviderIcon src="/opencode.ico" className="h-3.5 w-3.5 rounded-[3px]" />,
    },
    {
      id: "cursor",
      label: "Cursor",
      icon: <ProviderIcon src="/cursor.ico" className="h-3.5 w-3.5 rounded-[3px]" />,
    },
    {
      id: "grok",
      label: "Grok Bot",
      icon: <ProviderIcon src="/grokbot.ico" className="h-3.5 w-3.5 rounded-[3px]" />,
    },
    {
      id: "codex",
      label: "Codex",
      icon: (
        <ProviderIcon
          src="/openai.png"
          className={`h-3.5 w-3.5 rounded-[3px] ${panelTheme === "light" ? "invert" : ""}`}
        />
      ),
    },
    {
      id: "claude",
      label: "Claude",
      icon: <ProviderIcon src="/claude.ico" className="h-3.5 w-3.5 rounded-[3px]" />,
    },
  ];

  return (
    <>
      <Tabs
        items={providerTabs}
        active={tab}
        onSelect={onSelectTab}
        ariaLabel="Provider tabs"
        idPrefix="settings-provider"
      />
      <div
        role="tabpanel"
        id={tabPanelDomId("settings-provider", tab)}
        aria-labelledby={tabDomId("settings-provider", tab)}
        className="min-h-0 flex-1 overflow-y-auto no-scrollbar px-5 pb-5"
      >
        {tab === "opencode" ? (
          <OpenCodeAccountsPanel
            accounts={accounts}
            disabledAccounts={disabledAccounts}
            displayNames={displayNames}
            confirmRemove={confirmRemove}
            now={now}
            onAdd={onAdd}
            onOpenDetail={onOpenDetail}
            onConfirmRemove={onConfirmRemove}
            onRemoveAccount={onRemoveAccount}
          />
        ) : tab === "cursor" ? (
          <ProviderAccountPanel
            provider="cursor"
            title="Cursor"
            account={cursorAccount}
            nickname={displayNames["cursor"]}
            badgeText={cursorPlan}
            badgeVariant="blue"
            onOpenDetail={onOpenSubDetail}
          />
        ) : tab === "grok" ? (
          <ProviderAccountPanel
            provider="grok"
            title="Grok Bot"
            account={cursorAccount}
            nickname={displayNames["grok"]}
            badgeText={cursorPlan}
            badgeVariant="blue"
            onOpenDetail={onOpenSubDetail}
          />
        ) : tab === "codex" ? (
          <ProviderAccountPanel
            provider="codex"
            title="Codex"
            account={codexAccount}
            nickname={displayNames["codex"]}
            badgeText={
              codexPlan ? codexPlan.charAt(0).toUpperCase() + codexPlan.slice(1) : null
            }
            badgeVariant={codexPlan?.toLowerCase() === "free" ? "emerald" : "blue"}
            onOpenDetail={onOpenSubDetail}
          />
        ) : (
          <ProviderAccountPanel
            provider="claude"
            title="Claude"
            account={claudeAccount}
            nickname={displayNames["claude"]}
            badgeText={
              claudeStatus === "not subscribed" ? "Free" : claudeStatus
            }
            badgeVariant={
              claudeStatus === "not subscribed"
                ? "emerald"
                : claudeStatus === "unavailable" || claudeStatus === "not signed in"
                  ? "zinc"
                  : "blue"
            }
            onOpenDetail={onOpenSubDetail}
          />
        )}
      </div>
    </>
  );
}
