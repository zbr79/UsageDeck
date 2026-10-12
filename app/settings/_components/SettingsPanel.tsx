"use client";

import { useRef, useState, type MouseEvent } from "react";
import { IconButton } from "@/app/_ui/controls";
import Dialog from "@/app/_ui/Dialog";
import { CloseIcon } from "@/app/_ui/Icons";
import { cssVars, PANEL_PALETTES } from "@/app/_ui/palette";
import { TabList, TabPanel } from "@/app/_ui/Tabs";
import type { ProviderId } from "@/lib/types";
import { useSettings, useSummaries } from "../_lib/hooks";
import GeneralTab from "./GeneralTab";
import OpenCodeAccounts from "./OpenCodeAccounts";
import ProviderPanel, { PROVIDERS } from "./ProviderPanel";

type MainTab = "general" | "accounts";
type AccountTab = "opencode" | ProviderId;

const ACCOUNT_TABS: { id: AccountTab; name: string; icon: string }[] = [
  { id: "opencode", name: "OpenCode", icon: "/opencode.ico" },
  ...(Object.entries(PROVIDERS) as [ProviderId, (typeof PROVIDERS)[ProviderId]][]).map(
    ([id, p]) => ({ id, name: p.name, icon: p.icon }),
  ),
];

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
  const [mainTab, setMainTab] = useState<MainTab>("accounts");
  const [accountTab, setAccountTab] = useState<AccountTab>("opencode");
  const drag = useDrag();
  const theme = settings?.panelTheme ?? "dark";

  return (
    <div style={{ ...cssVars("p", PANEL_PALETTES[theme]), colorScheme: theme }}>
      <Dialog
        labelledBy="settings-title"
        onClose={onClose}
        style={{ transform: `translate(${drag.offset.x}px, ${drag.offset.y}px)` }}
      >
        <div className="bg-(--p-bg) text-(--p-text) rounded-xl shadow-2xl w-[576px] h-[608px] max-h-[calc(100vh-32px)] flex flex-col border border-(--p-border) overflow-hidden">
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
            <TabPanel idPrefix="main" id="accounts" className="flex-1 min-h-0 flex flex-col">
              <TabList
                label="Providers"
                idPrefix="acct"
                className="px-5 mb-3"
                value={accountTab}
                onChange={setAccountTab}
                items={ACCOUNT_TABS.map((t) => ({
                  id: t.id,
                  label: (
                    <>
                      <img
                        src={t.icon}
                        alt=""
                        className={`h-3.5 w-3.5 rounded-[3px] ${t.id === "codex" && theme === "light" ? "invert" : ""}`}
                      />
                      {t.name}
                    </>
                  ),
                }))}
              />
              <TabPanel
                idPrefix="acct"
                id={accountTab}
                className="flex-1 overflow-y-auto no-scrollbar px-5 pb-5"
              >
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
              </TabPanel>
            </TabPanel>
          )}
        </div>
      </Dialog>
    </div>
  );
}
