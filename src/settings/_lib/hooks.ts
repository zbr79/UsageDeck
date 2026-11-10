import { useCallback, useEffect, useRef, useState } from "react";
import { emitTo } from "@tauri-apps/api/event";
import { toastError, toastSuccess } from "@/components/toast/toast";
import { announceSettingsChanged, getJson, sendJson } from "@/lib/api";
import { inTauri } from "@/lib/bridge";
import { applySettingsPatch } from "@/lib/settings-schema";
import type {
  ClaudeUsage,
  CodexUsage,
  CursorUsage,
  GrokUsage,
  OpenCodeAccountSummary,
  OpenCodeAccountUsage,
  Settings,
  TrackedAccount,
} from "@/lib/types";

/**
 * Settings with optimistic updates: the UI changes immediately, the server
 * response (validated settings) wins, and failures roll back by reloading.
 */
export function useSettings() {
  const [settings, setSettings] = useState<Settings | null>(null);

  const reload = useCallback(
    () =>
      getJson<{ settings: Settings }>("/api/settings").then(
        (r) => setSettings(r.settings),
        () => toastError("Couldn't load settings"),
      ),
    [],
  );

  useEffect(() => {
    let live = true;
    getJson<{ settings: Settings }>("/api/settings").then(
      (r) => live && setSettings(r.settings),
      () => toastError("Couldn't load settings"),
    );
    return () => {
      live = false;
    };
  }, []);

  const update = useCallback(
    async (patch: Partial<Settings>, success?: string): Promise<boolean> => {
      setSettings((prev) => (prev ? applySettingsPatch(prev, patch) : prev));
      try {
        const res = await sendJson<{ settings: Settings }>("/api/settings", "POST", patch);
        setSettings(res.settings);
        announceSettingsChanged();
        if (success) toastSuccess(success);
        return true;
      } catch {
        toastError("Save failed");
        void reload();
        return false;
      }
    },
    [reload],
  );

  return { settings, update };
}

export interface Summaries {
  accounts: OpenCodeAccountSummary[] | null;
  /** The last account-list load failed (shown as an error, never as "no accounts"). */
  accountsFailed: boolean;
  usage: Record<string, OpenCodeAccountUsage>;
  cursor: { accounts: TrackedAccount<CursorUsage>[] } | null;
  grok: { accounts: TrackedAccount<GrokUsage>[] } | null;
  codex: { accounts: TrackedAccount<CodexUsage>[] } | null;
  claude: { accounts: TrackedAccount<ClaudeUsage>[] } | null;
}

/** Fetch the account list + usage; resolves to a state updater. */
async function fetchAccounts(): Promise<(s: Summaries) => Summaries> {
  const [accounts, usage] = await Promise.allSettled([
    getJson<{ accounts: OpenCodeAccountSummary[] }>("/api/accounts"),
    getJson<{ accounts: OpenCodeAccountUsage[] }>("/api/usage"),
  ]);
  if (accounts.status === "rejected")
    console.error("loading OpenCode accounts failed", accounts.reason);
  return (s) => ({
    ...s,
    // A failed load keeps what was there (null until the first success) instead of an empty
    // list, which the page would show as "no accounts yet".
    accounts: accounts.status === "fulfilled" ? accounts.value.accounts : s.accounts,
    accountsFailed: accounts.status === "rejected",
    usage:
      usage.status === "fulfilled"
        ? Object.fromEntries(usage.value.accounts.map((a) => [a.email, a]))
        : s.usage,
  });
}

/** Account lists and provider status shown in the Accounts tabs. */
export function useSummaries() {
  const [state, setState] = useState<Summaries>({
    accounts: null,
    accountsFailed: false,
    usage: {},
    cursor: null,
    grok: null,
    codex: null,
    claude: null,
  });

  const live = useRef(true);
  const reloadAccounts = useCallback(
    () => fetchAccounts().then((apply) => live.current && setState(apply)),
    [],
  );

  /** Reload every provider's accounts (after "Refresh now"). */
  const reloadAll = useCallback(() => {
    void reloadAccounts();
    const load = <K extends "cursor" | "grok" | "codex" | "claude">(key: K) =>
      getJson<Summaries[K]>(`/api/${key}`)
        .then((value) => live.current && setState((s) => ({ ...s, [key]: value })))
        .catch(() => {});
    void load("cursor");
    void load("grok");
    void load("codex");
    void load("claude");
  }, [reloadAccounts]);

  useEffect(() => {
    live.current = true;
    reloadAll();
    return () => {
      live.current = false;
    };
  }, [reloadAll]);

  return { ...state, reloadAccounts, reloadAll };
}

/**
 * "Refresh now": ask the widget to refetch every provider (the tray's Refresh event), then
 * reload this window's copy a few seconds later. A no-op in a plain browser.
 */
export function refreshWidget(reload: () => void): void {
  toastSuccess("Refreshing usage…");
  if (inTauri()) {
    emitTo("widget", "widget-refresh").catch((err) => console.error("refresh failed", err));
  }
  setTimeout(reload, 5000);
}

export async function saveAccount(email: string, key: string): Promise<boolean> {
  try {
    await sendJson("/api/accounts", "POST", { email, key });
    announceSettingsChanged();
    return true;
  } catch (e) {
    toastError(e instanceof Error ? e.message : "Save failed");
    return false;
  }
}

export async function removeAccount(email: string): Promise<boolean> {
  try {
    await sendJson("/api/accounts", "DELETE", { email });
    announceSettingsChanged();
    return true;
  } catch {
    toastError("Remove failed");
    return false;
  }
}
