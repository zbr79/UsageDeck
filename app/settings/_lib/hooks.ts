"use client";

import { useCallback, useEffect, useState } from "react";
import { toastError, toastSuccess } from "@/app/components/toast/toast";
import { announceSettingsChanged, getJson, sendJson } from "@/lib/api-fetch";
import { applySettingsPatch } from "@/lib/settings-schema";
import type {
  ClaudeUsage,
  CodexUsage,
  CursorUsage,
  GrokUsage,
  OpenCodeAccountSummary,
  OpenCodeAccountUsage,
  ProviderResponse,
  Settings,
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
  usage: Record<string, OpenCodeAccountUsage>;
  cursor: ProviderResponse<CursorUsage> | null;
  grok: ProviderResponse<GrokUsage> | null;
  codex: ProviderResponse<CodexUsage> | null;
  claude: ProviderResponse<ClaudeUsage> | null;
}

/** Fetch the account list + usage; resolves to a state updater. */
async function fetchAccounts(): Promise<(s: Summaries) => Summaries> {
  const [accounts, usage] = await Promise.allSettled([
    getJson<{ accounts: OpenCodeAccountSummary[] }>("/api/accounts"),
    getJson<{ accounts: OpenCodeAccountUsage[] }>("/api/usage"),
  ]);
  return (s) => ({
    ...s,
    accounts: accounts.status === "fulfilled" ? accounts.value.accounts : (s.accounts ?? []),
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
    usage: {},
    cursor: null,
    grok: null,
    codex: null,
    claude: null,
  });

  const reloadAccounts = useCallback(() => fetchAccounts().then((apply) => setState(apply)), []);

  useEffect(() => {
    let live = true;
    void fetchAccounts().then((apply) => live && setState(apply));
    const load = <K extends "cursor" | "grok" | "codex" | "claude">(key: K) =>
      getJson<Summaries[K]>(`/api/${key}`)
        .then((value) => live && setState((s) => ({ ...s, [key]: value })))
        .catch(() => {});
    void load("cursor");
    void load("grok");
    void load("codex");
    void load("claude");
    return () => {
      live = false;
    };
  }, []);

  return { ...state, reloadAccounts };
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
