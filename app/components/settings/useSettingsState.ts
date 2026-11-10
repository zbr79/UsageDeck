"use client";

import { useEffect, useReducer } from "react";
import { toastError, toastSuccess } from "@/app/components/toast/toast";
import { apiFetch } from "@/lib/api-fetch";
import type { Settings } from "@/lib/settings";
import { initialSettingsState, settingsReducer } from "./settings-reducer";
import type { AccountRow, AccountWindow, ProviderTab, SubProvider } from "./types";

type UsageWindowJson = { status?: string; percent?: number; resetsAt?: string };
type UsageBody = { weekly?: UsageWindowJson; monthly?: UsageWindowJson } | null;
type UsageAccountJson = { email?: string; usage?: UsageBody; prev?: UsageBody };

function readWindow(window: UsageWindowJson | undefined): AccountWindow | null {
  if (!window || typeof window.resetsAt !== "string" || typeof window.percent !== "number") return null;
  return {
    resetsAt: window.resetsAt,
    percent: window.percent,
    exhausted: window.percent >= 100 || window.status === "exhausted",
  };
}

function usageFacts(row: UsageAccountJson | undefined): Pick<AccountRow, "inactive" | "cached" | "weekly" | "monthly"> {
  const live = row?.usage ?? null;
  const last = row?.prev ?? null;
  const source = live ?? last;
  return {
    inactive: !live && !last,
    cached: !live && Boolean(last),
    weekly: readWindow(source?.weekly),
    monthly: readWindow(source?.monthly),
  };
}

function patch(dispatch: React.Dispatch<Parameters<typeof settingsReducer>[1]>) {
  return (next: Partial<typeof initialSettingsState>) => dispatch({ type: "patch", patch: next });
}

export function useSettingsState(onChanged: () => void) {
  const [state, dispatch] = useReducer(settingsReducer, initialSettingsState);
  const set = patch(dispatch);

  const refresh = async () => {
    const [u, s, usage] = await Promise.all([
      apiFetch("/api/accounts", { cache: "no-store" })
        .then((r): Promise<{ accounts?: AccountRow[] }> => (r.ok ? r.json() : Promise.resolve({})))
        .catch((): { accounts?: AccountRow[] } => ({})),
      apiFetch("/api/settings", { cache: "no-store" })
        .then((r): Promise<{ settings?: Partial<Settings> }> => (r.ok ? r.json() : Promise.resolve({})))
        .catch((): { settings?: Partial<Settings> } => ({})),
      apiFetch("/api/usage", { cache: "no-store" })
        .then(
          (r): Promise<{ accounts?: UsageAccountJson[] }> =>
            r.ok ? r.json() : Promise.resolve({}),
        )
        .catch((): { accounts?: UsageAccountJson[] } => ({})),
    ]);
    const usageKnown = Array.isArray(usage.accounts);
    const usageByEmail = new Map<string, UsageAccountJson>();
    if (usageKnown) {
      for (const row of usage.accounts ?? []) {
        if (row.email) usageByEmail.set(row.email.toLowerCase(), row);
      }
    }
    const disabled = s.settings?.disabledAccounts ?? [];
    const list = (u.accounts ?? []).map((a: AccountRow) => ({
      email: a.email,
      ...(usageKnown ? usageFacts(usageByEmail.get(a.email.toLowerCase())) : {}),
    }));
    list.sort(
      (a: AccountRow, b: AccountRow) =>
        (disabled.includes(a.email.toLowerCase()) ? 1 : 0) -
        (disabled.includes(b.email.toLowerCase()) ? 1 : 0),
    );
    dispatch({
      type: "loaded",
      patch: {
        accounts: list,
        cursorEnabled: s.settings?.cursorEnabled !== false,
        grokEnabled: s.settings?.grokEnabled !== false,
        codexEnabled: s.settings?.codexEnabled !== false,
        claudeEnabled: s.settings?.claudeEnabled !== false,
        deepseekPeakHourWarning: s.settings?.deepseekPeakHourWarning !== false,
        showProviderNames: s.settings?.showProviderNames === true,
        themeMode: s.settings?.themeMode ?? "auto",
        panelTheme: s.settings?.panelTheme === "light" ? "light" : "dark",
        displayNames: s.settings?.displayNames ?? {},
        disabledAccounts: disabled,
        usageAsOf: Date.now(),
      },
    });
    apiFetch("/api/cursor", { cache: "no-store" })
      .then((r) => r.json())
      .then((c) => {
        set({
          cursorAccount: c.usage?.accountName ?? null,
          cursorPlan: c.usage?.planName ?? null,
        });
      })
      .catch(() => {
        // Expected when Cursor is not signed in locally
      });
    apiFetch("/api/codex", { cache: "no-store" })
      .then((r) => r.json())
      .then((c) => {
        set({
          codexAccount: c.usage?.email ?? null,
          codexPlan: c.usage?.planType ?? null,
        });
      })
      .catch(() => {
        // Expected when Codex is not signed in locally
      });
    apiFetch("/api/claude", { cache: "no-store" })
      .then((r) => r.json())
      .then((c) => {
        set({
          claudeAccount: c.usage?.email ?? null,
          claudeStatus:
            c.usage == null
              ? "unavailable"
              : !c.usage.signedIn
                ? "not signed in"
                : c.usage.subscribed
                  ? c.usage.subscriptionType ?? "subscribed"
                  : "not subscribed",
        });
      })
      .catch(() => {
        // Expected when Claude Code is not signed in locally
      });
  };

  useEffect(() => {
    // Load accounts and settings once when the modal opens.
    void refresh();
    // The loader is intentionally run once when the modal mounts.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const openDetail = async (email: string) => {
    set({ detailEmail: email, detailKey: null, editKey: false });
    try {
      const res = await apiFetch(`/api/key?email=${encodeURIComponent(email)}`);
      if (!res.ok) {
        set({ detailKey: null });
        return;
      }
      const json = await res.json();
      set({ detailKey: typeof json.key === "string" ? json.key : null });
    } catch {
      set({ detailKey: null });
    }
  };

  const saveNewKey = async () => {
    if (!state.detailEmail) return;
    set({ keyBusy: true });
    try {
      const res = await apiFetch("/api/accounts", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ email: state.detailEmail, key: state.newKeyValue }),
      });
      if (res.ok) {
        toastSuccess("API key updated");
        set({ detailKey: state.newKeyValue, newKeyValue: "", editKey: false });
        onChanged();
      } else {
        const json = await res.json();
        toastError(json.error ?? "Update failed");
      }
    } catch {
      toastError("Update failed");
    } finally {
      set({ keyBusy: false });
    }
  };

  const saveSettingField = async (
    payload: Record<string, unknown>,
    successMsg: string,
    rollback?: () => void,
  ) => {
    try {
      const res = await apiFetch("/api/settings", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });
      if (res.ok) {
        toastSuccess(successMsg);
        onChanged();
      } else {
        toastError("Save failed");
        rollback?.();
      }
    } catch {
      toastError("Save failed");
      rollback?.();
    }
  };

  const saveDisplayName = async (key: string, name: string) => {
    const next = { ...state.displayNames, [key]: name };
    if (!name.trim()) delete next[key];
    set({ displayNames: next });
    try {
      const res = await apiFetch("/api/settings", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ cursorEnabled: state.cursorEnabled, displayNames: next }),
      });
      if (res.ok) {
        set({ savedFlash: key });
        setTimeout(() => dispatch({ type: "clearSavedFlash", key }), 1500);
        toastSuccess("Display name saved");
        onChanged();
      } else {
        toastError("Save failed");
      }
    } catch {
      toastError("Save failed");
    }
  };

  const toggleAccount = async (email: string, enabled: boolean) => {
    const lower = email.toLowerCase();
    const prev = state.disabledAccounts;
    const next = enabled
      ? state.disabledAccounts.filter((e) => e !== lower)
      : [...state.disabledAccounts.filter((e) => e !== lower), lower];
    set({ disabledAccounts: next });
    await saveSettingField(
      { cursorEnabled: state.cursorEnabled, displayNames: state.displayNames, disabledAccounts: next },
      enabled ? "Account enabled" : "Account disabled",
      () => set({ disabledAccounts: prev }),
    );
  };

  const addAccount = async (e: React.FormEvent) => {
    e.preventDefault();
    set({ busy: true });
    try {
      const res = await apiFetch("/api/accounts", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ email: state.newEmail, key: state.newKey }),
      });
      if (res.ok) {
        toastSuccess("Account added");
        set({ newEmail: "", newKey: "", showAdd: false });
        await refresh();
        onChanged();
      } else {
        const json = await res.json();
        toastError(json.error ?? "Add failed");
      }
    } catch {
      toastError("Add failed");
    } finally {
      set({ busy: false });
    }
  };

  const removeAccount = async (email: string) => {
    try {
      const res = await apiFetch("/api/accounts", {
        method: "DELETE",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ email }),
      });
      if (res.ok) {
        toastSuccess("Account removed");
        await refresh();
        onChanged();
      } else {
        toastError("Remove failed");
      }
    } catch {
      toastError("Remove failed");
    }
  };

  return {
    tab: state.tab,
    setTab: (tab: ProviderTab) => set({ tab }),
    accounts: state.accounts,
    cursorEnabled: state.cursorEnabled,
    setCursorEnabled: (cursorEnabled: boolean) => set({ cursorEnabled }),
    grokEnabled: state.grokEnabled,
    setGrokEnabled: (grokEnabled: boolean) => set({ grokEnabled }),
    codexEnabled: state.codexEnabled,
    setCodexEnabled: (codexEnabled: boolean) => set({ codexEnabled }),
    codexAccount: state.codexAccount,
    codexPlan: state.codexPlan,
    claudeEnabled: state.claudeEnabled,
    setClaudeEnabled: (claudeEnabled: boolean) => set({ claudeEnabled }),
    deepseekPeakHourWarning: state.deepseekPeakHourWarning,
    setDeepseekPeakHourWarning: (deepseekPeakHourWarning: boolean) => set({ deepseekPeakHourWarning }),
    claudeAccount: state.claudeAccount,
    claudeStatus: state.claudeStatus,
    showProviderNames: state.showProviderNames,
    setShowProviderNames: (showProviderNames: boolean) => set({ showProviderNames }),
    themeMode: state.themeMode,
    setThemeMode: (themeMode: Settings["themeMode"]) => set({ themeMode }),
    panelTheme: state.panelTheme,
    setPanelTheme: (panelTheme: Settings["panelTheme"]) => set({ panelTheme }),
    subDetail: state.subDetail,
    setSubDetail: (subDetail: SubProvider | null) => set({ subDetail }),
    displayNames: state.displayNames,
    disabledAccounts: state.disabledAccounts,
    cursorAccount: state.cursorAccount,
    cursorPlan: state.cursorPlan,
    newEmail: state.newEmail,
    setNewEmail: (newEmail: string) => set({ newEmail }),
    newKey: state.newKey,
    setNewKey: (newKey: string) => set({ newKey }),
    busy: state.busy,
    detailEmail: state.detailEmail,
    setDetailEmail: (detailEmail: string | null) => set({ detailEmail }),
    detailKey: state.detailKey,
    editKey: state.editKey,
    setEditKey: (editKey: boolean) => set({ editKey }),
    newKeyValue: state.newKeyValue,
    setNewKeyValue: (newKeyValue: string) => set({ newKeyValue }),
    keyBusy: state.keyBusy,
    showAdd: state.showAdd,
    usageAsOf: state.usageAsOf,
    setShowAdd: (showAdd: boolean) => set({ showAdd }),
    confirmRemove: state.confirmRemove,
    setConfirmRemove: (confirmRemove: string | null) => set({ confirmRemove }),
    savedFlash: state.savedFlash,
    openDetail,
    saveNewKey,
    saveSettingField,
    saveDisplayName,
    toggleAccount,
    addAccount,
    removeAccount,
  };
}
