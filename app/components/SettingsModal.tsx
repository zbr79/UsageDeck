"use client";

import { useEffect, useRef, useState, type CSSProperties } from "react";
import AccountsTab from "@/app/components/settings/AccountsTab";
import AddAccountForm from "@/app/components/settings/AddAccountForm";
import GeneralTab from "@/app/components/settings/GeneralTab";
import OpenCodeDetailDialog from "@/app/components/settings/OpenCodeDetailDialog";
import ProviderDetailDialog from "@/app/components/settings/ProviderDetailDialog";
import UpdateKeyDialog from "@/app/components/settings/UpdateKeyDialog";
import type { MainTab, SubProvider } from "@/app/components/settings/types";
import { useSettingsState } from "@/app/components/settings/useSettingsState";
import Dialog, { useDialogFocusTrap } from "@/app/components/ui/Dialog";
import IconButton, { CloseIcon } from "@/app/components/ui/IconButton";
import Tabs, { tabDomId, tabPanelDomId, type TabItem } from "@/app/components/ui/Tabs";
import { SETTINGS_THEMES, SETTINGS_TYPE_SCALE, themeStyleVars } from "@/lib/theme";

const MAIN_TABS: TabItem<MainTab>[] = [
  { id: "system", label: "General" },
  { id: "accounts", label: "Accounts" },
];

export default function SettingsModal({
  onClose,
  onChanged,
}: {
  onClose: () => void;
  onChanged: () => void;
}) {
  const s = useSettingsState(onChanged);
  const [mainTab, setMainTab] = useState<MainTab>("accounts");
  const [dragPos, setDragPos] = useState({ x: 0, y: 0 });
  const dragState = useRef<{ sx: number; sy: number; ox: number; oy: number } | null>(null);
  const modalRef = useRef<HTMLDivElement | null>(null);

  const onHeaderMouseDown = (e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest("button, input, select")) return;
    dragState.current = { sx: e.clientX, sy: e.clientY, ox: dragPos.x, oy: dragPos.y };
    const move = (ev: MouseEvent) => {
      const d = dragState.current;
      if (!d) return;
      const width = modalRef.current?.offsetWidth ?? 576;
      const height = modalRef.current?.offsetHeight ?? 420;
      const margin = 24;
      const maxX = Math.max(0, (window.innerWidth - width) / 2 - margin);
      const maxY = Math.max(0, (window.innerHeight - height) / 2 - margin);
      const x = d.ox + ev.clientX - d.sx;
      const y = d.oy + ev.clientY - d.sy;
      setDragPos({
        x: Math.min(maxX, Math.max(-maxX, x)),
        y: Math.min(maxY, Math.max(-maxY, y)),
      });
    };
    const up = () => {
      dragState.current = null;
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  const { editKey, setEditKey, subDetail, setSubDetail, detailEmail, setDetailEmail, showAdd, setShowAdd } = s;
  const hasChildDialog = Boolean(editKey || subDetail || detailEmail || showAdd);
  useDialogFocusTrap(modalRef, !hasChildDialog);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (editKey) setEditKey(false);
      else if (showAdd) setShowAdd(false);
      else if (subDetail) setSubDetail(null);
      else if (detailEmail) setDetailEmail(null);
      else onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [editKey, setEditKey, showAdd, setShowAdd, subDetail, setSubDetail, detailEmail, setDetailEmail, onClose]);

  const subDetailConfig = (p: SubProvider) => {
    if (p === "cursor") {
      return {
        enabled: s.cursorEnabled,
        account: s.cursorAccount,
        badgeText: s.cursorPlan,
        badgeVariant: "blue" as const,
        onToggle: (next: boolean) => {
          s.setCursorEnabled(next);
          void s.saveSettingField(
            { cursorEnabled: next },
            next ? "Cursor monitoring on" : "Cursor monitoring off",
            () => s.setCursorEnabled(!next),
          );
        },
      };
    }
    if (p === "grok") {
      return {
        enabled: s.grokEnabled,
        account: s.cursorAccount,
        badgeText: s.cursorPlan,
        badgeVariant: "blue" as const,
        onToggle: (next: boolean) => {
          s.setGrokEnabled(next);
          void s.saveSettingField(
            { grokEnabled: next },
            next ? "Grok Bot monitoring on" : "Grok Bot monitoring off",
            () => s.setGrokEnabled(!next),
          );
        },
      };
    }
    if (p === "codex") {
      return {
        enabled: s.codexEnabled,
        account: s.codexAccount,
        badgeText: s.codexPlan
          ? s.codexPlan.charAt(0).toUpperCase() + s.codexPlan.slice(1)
          : null,
        badgeVariant:
          s.codexPlan?.toLowerCase() === "free" ? ("emerald" as const) : ("blue" as const),
        onToggle: (next: boolean) => {
          s.setCodexEnabled(next);
          void s.saveSettingField(
            { codexEnabled: next },
            next ? "Codex monitoring on" : "Codex monitoring off",
            () => s.setCodexEnabled(!next),
          );
        },
      };
    }
    return {
      enabled: s.claudeEnabled,
      account: s.claudeAccount,
      badgeText: s.claudeStatus === "not subscribed" ? "Free" : s.claudeStatus,
      badgeVariant:
        s.claudeStatus === "not subscribed"
          ? ("emerald" as const)
          : s.claudeStatus === "unavailable" || s.claudeStatus === "not signed in"
            ? ("zinc" as const)
            : ("blue" as const),
      onToggle: (next: boolean) => {
        s.setClaudeEnabled(next);
        void s.saveSettingField(
          { claudeEnabled: next },
          next ? "Claude monitoring on" : "Claude monitoring off",
          () => s.setClaudeEnabled(!next),
        );
      },
    };
  };

  const themeVars = themeStyleVars(SETTINGS_THEMES[s.panelTheme]) as CSSProperties;

  return (
    <div
      className="fixed inset-0 bg-transparent flex items-center justify-center p-4 z-50"
      style={themeVars}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        ref={modalRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-modal-title"
        tabIndex={-1}
        className="bg-role-bg rounded-xl shadow-2xl w-[576px] min-h-[420px] max-h-[min(640px,calc(100vh-2rem))] flex flex-col border border-role-border overflow-hidden"
        style={{ transform: `translate(${dragPos.x}px, ${dragPos.y}px)` }}
        onClick={(e) => e.stopPropagation()}
      >
        <div
          className="flex items-center justify-between px-5 pt-5 pb-2 cursor-move select-none"
          onMouseDown={onHeaderMouseDown}
        >
          <h2
            id="settings-modal-title"
            className={`${SETTINGS_TYPE_SCALE.heading} font-bold text-role-text`}
          >
            Settings
          </h2>
          <IconButton
            onClick={onClose}
            title="Close"
            aria-label="Close settings"
            className="p-1.5 rounded-md text-role-muted hover:text-role-text hover:bg-role-surface"
          >
            <CloseIcon />
          </IconButton>
        </div>

        <Tabs
          items={MAIN_TABS}
          active={mainTab}
          onSelect={setMainTab}
          ariaLabel="Settings sections"
          idPrefix="settings-main"
          boldActive
        />

        <div
          role="tabpanel"
          id={tabPanelDomId("settings-main", mainTab)}
          aria-labelledby={tabDomId("settings-main", mainTab)}
          className="flex flex-1 flex-col min-h-0"
        >
        {mainTab === "system" ? (
          <GeneralTab
            panelTheme={s.panelTheme}
            themeMode={s.themeMode}
            showProviderNames={s.showProviderNames}
            deepseekPeakHourWarning={s.deepseekPeakHourWarning}
            onSetPanel={(t) => {
              s.setPanelTheme(t);
              void s.saveSettingField({ panelTheme: t }, `Panel theme: ${t}`);
            }}
            onSetTheme={(m) => {
              s.setThemeMode(m);
              void s.saveSettingField({ themeMode: m }, `Theme: ${m}`);
            }}
            onToggleProviderNames={(enabled) => {
              s.setShowProviderNames(enabled);
              void s.saveSettingField(
                { showProviderNames: enabled },
                enabled ? "Cards show provider names" : "Cards show nicknames",
                () => s.setShowProviderNames(!enabled),
              );
            }}
            onToggleDeepseekPeakHourWarning={(enabled) => {
              s.setDeepseekPeakHourWarning(enabled);
              void s.saveSettingField(
                { deepseekPeakHourWarning: enabled },
                enabled ? "DeepSeek peak-hour warning on" : "DeepSeek peak-hour warning off",
                () => s.setDeepseekPeakHourWarning(!enabled),
              );
            }}
          />
        ) : (
          <AccountsTab
            tab={s.tab}
            onSelectTab={s.setTab}
            panelTheme={s.panelTheme}
            accounts={s.accounts}
            disabledAccounts={s.disabledAccounts}
            displayNames={s.displayNames}
            cursorAccount={s.cursorAccount}
            cursorPlan={s.cursorPlan}
            codexAccount={s.codexAccount}
            codexPlan={s.codexPlan}
            claudeAccount={s.claudeAccount}
            claudeStatus={s.claudeStatus}
            confirmRemove={s.confirmRemove}
            now={s.usageAsOf}
            onAdd={() => s.setShowAdd(true)}
            onOpenDetail={s.openDetail}
            onConfirmRemove={s.setConfirmRemove}
            onRemoveAccount={s.removeAccount}
            onOpenSubDetail={s.setSubDetail}
          />
        )}
        </div>
        {s.showAdd && (
          <Dialog
            onClose={() => s.setShowAdd(false)}
            ariaLabel="Add account"
            widthClass="w-[320px]"
            paddingClass="p-4"
            zIndexClass="z-20"
          >
            <AddAccountForm
              email={s.newEmail}
              apiKey={s.newKey}
              busy={s.busy}
              onChangeEmail={s.setNewEmail}
              onChangeKey={s.setNewKey}
              onCancel={() => s.setShowAdd(false)}
              onSubmit={s.addAccount}
            />
          </Dialog>
        )}
      </div>

      {s.detailEmail && (
        <OpenCodeDetailDialog
          email={s.detailEmail}
          detailKey={s.detailKey}
          disabled={s.disabledAccounts.includes(s.detailEmail.toLowerCase())}
          nickname={s.displayNames[s.detailEmail] ?? ""}
          savedFlash={s.savedFlash === s.detailEmail}
          onClose={() => s.setDetailEmail(null)}
          onToggleAccount={s.toggleAccount}
          onSaveNickname={s.saveDisplayName}
          onStartEditKey={() => {
            s.setNewKeyValue("");
            s.setEditKey(true);
          }}
        />
      )}

      {s.editKey && s.detailEmail && (
        <UpdateKeyDialog
          email={s.detailEmail}
          value={s.newKeyValue}
          busy={s.keyBusy}
          onChange={s.setNewKeyValue}
          onCancel={() => s.setEditKey(false)}
          onSave={s.saveNewKey}
        />
      )}

      {s.subDetail && (() => {
        const cfg = subDetailConfig(s.subDetail);
        return (
          <ProviderDetailDialog
            provider={s.subDetail}
            enabled={cfg.enabled}
            account={cfg.account}
            badgeText={cfg.badgeText}
            badgeVariant={cfg.badgeVariant}
            nickname={s.displayNames[s.subDetail] ?? ""}
            savedFlash={s.savedFlash === s.subDetail}
            onClose={() => s.setSubDetail(null)}
            onToggleMonitoring={cfg.onToggle}
            onSaveNickname={s.saveDisplayName}
          />
        );
      })()}
    </div>
  );
}
