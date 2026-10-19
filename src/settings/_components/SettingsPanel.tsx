import { useMemo, useRef, useState, type MouseEvent } from "react";
import { IconButton } from "@/ui/controls";
import Dialog from "@/ui/Dialog";
import { CloseIcon } from "@/ui/Icons";
import { cssVars, PANEL_PALETTES, WIDGET_PALETTES } from "@/ui/palette";
import { TabList, TabPanel } from "@/ui/Tabs";
import type { WidgetProvider } from "@/lib/types";
import { useSettings, useSummaries } from "../_lib/hooks";
import GeneralTab from "./GeneralTab";
import OpenCodeAccounts from "./OpenCodeAccounts";
import ProviderPanel, { PROVIDERS } from "./ProviderPanel";

type MainTab = "general" | "accounts";

const ACCOUNT_TABS: readonly WidgetProvider[] = ["opencode", "cursor", "grok", "codex", "claude"];

/** `settings.html?tab=<general|opencode|claude|…>` opens on that tab (for screenshots). */
function initialTabs(): { main: MainTab; account: WidgetProvider } {
  const tab = new URLSearchParams(window.location.search).get("tab");
  if (tab === "general") return { main: "general", account: "opencode" };
  const account = ACCOUNT_TABS.find((id) => id === tab) ?? "opencode";
  return { main: "accounts", account };
}

function providerTabs(order: readonly WidgetProvider[]) {
  return order.map((id) => {
    if (id === "opencode") return { id, name: "OpenCode", icon: "/opencode.ico" };
    const provider = PROVIDERS[id];
    return { id, name: provider.name, icon: provider.icon };
  });
}

function useDrag() {
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const start = useRef<{ sx: number; sy: number; ox: number; oy: number } | null>(null);
  const onMouseDown = (e: MouseEvent) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest("button, input, select, a")) return;
    start.current = { sx: e.clientX, sy: e.clientY, ox: offset.x, oy: offset.y };
    const move = (ev: globalThis.MouseEvent) => {
      const s = start.current;
      if (s) setOffset({ x: s.ox + ev.clientX - s.sx, y: s.oy + ev.clientY - s.sy });
    };
    const up = () => {
      start.current = null;
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };
  return { offset, onMouseDown };
}

export default function SettingsPanel({ onClose }: { onClose: () => void }) {
  const { settings, update } = useSettings();
  const summaries = useSummaries();
  const [initial] = useState(initialTabs);
  const [mainTab, setMainTab] = useState<MainTab>(initial.main);
  const [accountTab, setAccountTab] = useState<WidgetProvider>(initial.account);
  const drag = useDrag();
  const theme = settings?.panelTheme ?? "dark";
  // The account cards are the widget's own, so they also need its colours, in the panel's theme.
  const vars = useMemo(
    () => ({ ...cssVars("p", PANEL_PALETTES[theme]), ...cssVars("w", WIDGET_PALETTES[theme]) }),
    [theme],
  );

  return (
    <div style={{ ...vars, colorScheme: theme }}>
      <Dialog
        labelledBy="settings-title"
        onClose={onClose}
        style={{ transform: `translate(${drag.offset.x}px, ${drag.offset.y}px)` }}
      >
        <div className="bg-(--p-bg) text-(--p-text) rounded-xl shadow-2xl w-[720px] h-[640px] max-w-[calc(100vw-32px)] max-h-[calc(100vh-32px)] flex flex-col border border-(--p-border) overflow-hidden">
          <div
            className="flex items-center justify-between px-5 pt-5 pb-2 cursor-move select-none"
            onMouseDown={drag.onMouseDown}
          >
            <h2 id="settings-title" className="text-lg font-bold">
              Settings
            </h2>
            <IconButton label="Close settings" onClick={onClose}>
              <CloseIcon size={16} />
            </IconButton>
          </div>

          <TabList
            label="Settings sections"
            idPrefix="main"
            className="px-5 mb-3"
            value={mainTab}
            onChange={setMainTab}
            items={[
              { id: "general", label: "General" },
              { id: "accounts", label: "Accounts" },
            ]}
          />

          {!settings ? (
            <p className="px-5 text-sm text-(--p-muted)">Loading…</p>
          ) : mainTab === "general" ? (
            <TabPanel
              idPrefix="main"
              id="general"
              className="flex-1 overflow-y-auto no-scrollbar px-5 pb-5"
            >
              <GeneralTab settings={settings} update={update} />
            </TabPanel>
          ) : (
            <TabPanel idPrefix="main" id="accounts" className="flex-1 min-h-0 flex">
              <aside className="w-[170px] shrink-0 border-r border-(--p-divider) px-3 pb-5 overflow-y-auto no-scrollbar">
                <div className="px-2 pb-2 text-[11px] font-medium uppercase tracking-wide text-(--p-muted)">
                  Providers
                </div>
                <p
                  title="Drag providers up or down to change their order on the widget"
                  className="px-2 pb-2 text-[10px] leading-3 text-(--p-muted)"
                >
                  Drag up or down to reorder on the widget
                </p>
                <TabList
                  label="Providers"
                  idPrefix="acct"
                  value={accountTab}
                  onChange={setAccountTab}
                  orientation="vertical"
                  onReorder={(providerOrder) => void update({ providerOrder })}
                  items={providerTabs(settings.providerOrder).map((t) => ({
                    id: t.id,
                    label: (
                      <>
                        <img
                          src={t.icon}
                          alt=""
                          draggable={false}
                          className={`h-3.5 w-3.5 rounded-[3px] ${t.id === "codex" && theme === "light" ? "invert" : ""}`}
                        />
                        {t.name}
                      </>
                    ),
                  }))}
                />
              </aside>
              <TabPanel
                idPrefix="acct"
                id={accountTab}
                className="flex-1 min-w-0 overflow-y-auto no-scrollbar px-6 pb-5"
              >
                <div className="pt-1">
                  {accountTab === "opencode" ? (
                    <OpenCodeAccounts
                      accounts={summaries.accounts}
                      usage={summaries.usage}
                      settings={settings}
                      update={update}
                      reload={summaries.reloadAccounts}
                    />
                  ) : (
                    <ProviderPanel
                      id={accountTab}
                      summaries={summaries}
                      settings={settings}
                      update={update}
                    />
                  )}
                </div>
              </TabPanel>
            </TabPanel>
          )}
        </div>
      </Dialog>
    </div>
  );
}
