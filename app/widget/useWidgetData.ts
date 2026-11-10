"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { apiFetch } from "@/lib/api-fetch";
import { usageWidget } from "@/lib/desktop-bridge";
import { isExhaustedWindow, sortAccounts } from "@/lib/ranking";
import type { AccountRow, ClaudeUsage, CodexUsage, CursorUsage } from "./types";

const POLL_MS = 60_000;

export function useWidgetData() {
  const [rows, setRows] = useState<AccountRow[] | null>(null);
  const [cursor, setCursor] = useState<CursorUsage | null>(null);
  const [cursorError, setCursorError] = useState(false);
  const [cursorEnabled, setCursorEnabled] = useState(true);
  const [grokEnabled, setGrokEnabled] = useState(true);
  const [codexEnabled, setCodexEnabled] = useState(true);
  const [deepseekPeakHourWarning, setDeepseekPeakHourWarning] = useState(true);
  const [codex, setCodex] = useState<CodexUsage | null>(null);
  const [claudeEnabled, setClaudeEnabled] = useState(true);
  const [claude, setClaude] = useState<ClaudeUsage | null>(null);
  const [displayNames, setDisplayNames] = useState<Record<string, string>>({});
  const [showProviderNames, setShowProviderNames] = useState(false);
  const [themeMode, setThemeMode] = useState<"auto" | "light" | "dark" | "white">("auto");
  const [changedAt, setChangedAt] = useState<Map<string, number>>(() => new Map());
  const [increaseAt, setIncreaseAt] = useState<Map<string, number>>(() => new Map());
  const [failedPolls, setFailedPolls] = useState(0);
  const [serverState, setServerState] = useState<"online" | "restarting" | "stopped">("online");

  const load = useCallback(async () => {
    try {
      const settingsRes = await apiFetch("/api/settings", { cache: "no-store" });
      if (!settingsRes.ok) throw new Error(`settings HTTP ${settingsRes.status}`);
      const sjson = await settingsRes.json();
      setDeepseekPeakHourWarning(sjson.settings?.deepseekPeakHourWarning !== false);
      setCodexEnabled(sjson.settings?.codexEnabled !== false);
      setClaudeEnabled(sjson.settings?.claudeEnabled !== false);
      setCursorEnabled(sjson.settings?.cursorEnabled !== false);
      setGrokEnabled(sjson.settings?.grokEnabled !== false);
      setDisplayNames(sjson.settings?.displayNames ?? {});
      setShowProviderNames(sjson.settings?.showProviderNames === true);
      setThemeMode(sjson.settings?.themeMode ?? "auto");

      const [usageRes, cursorRes, codexRes, claudeRes] = await Promise.all([
        apiFetch("/api/usage", { cache: "no-store" }),
        apiFetch("/api/cursor", { cache: "no-store" }),
        apiFetch("/api/codex", { cache: "no-store" }),
        apiFetch("/api/claude", { cache: "no-store" }),
      ]);
      if (!usageRes.ok) throw new Error(`usage HTTP ${usageRes.status}`);
      const json = await usageRes.json();
      {
        const cjson = await cursorRes.json();
        setCursorError(!!cjson.error);
        setCursor(cjson.usage ?? null);
      }
      setCodex((await codexRes.json()).usage ?? null);
      setClaude((await claudeRes.json()).usage ?? null);
      const accounts: AccountRow[] = json.accounts ?? [];
      const ts = Date.now();
      setChangedAt((prev) => {
        let next = prev;
        for (const row of accounts) {
          if (row.lastChangeAt == null) continue;
          const existing = next.get(row.email);
          if (existing != null && row.lastChangeAt < existing) continue;
          if (next === prev) next = new Map(prev);
          next.set(row.email, row.lastChangeAt);
        }
        return next;
      });
      setIncreaseAt((prev) => {
        let next = prev;
        for (const row of accounts) {
          if (!row.usage || !row.prev) continue;
          for (const win of ["rolling", "weekly", "monthly"] as const) {
            const key = `${row.email}:${win}`;
            const cur = row.usage[win];
            const previous = row.prev[win];
            const rising = !isExhaustedWindow(cur) && cur.percent > previous.percent;
            if (rising) {
              if (next.has(key)) continue;
              if (next === prev) next = new Map(prev);
              next.set(key, ts);
            } else if (next.has(key)) {
              if (next === prev) next = new Map(prev);
              next.delete(key);
            }
          }
        }
        return next;
      });
      setRows(accounts);
      setFailedPolls(0);
      setServerState("online");
    } catch {
      setFailedPolls((n) => n + 1);
    }
  }, []);

  useEffect(() => {
    // Poll provider usage after mount. The fetch updates state when it resolves.
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data load on mount
    load();
    const id = setInterval(load, POLL_MS);
    return () => clearInterval(id);
  }, [load]);

  useEffect(() => {
    const off = usageWidget()?.onSettingsChanged?.(() => load());
    return () => off?.();
  }, [load]);

  useEffect(() => {
    const off = usageWidget()?.onServerStatus?.((s) => {
      if (s?.state) setServerState(s.state);
    });
    return () => off?.();
  }, []);

  const sortedRows = useMemo(() => {
    if (!rows) return null;
    return sortAccounts(rows, (r) => changedAt.get(r.email) ?? 0);
  }, [rows, changedAt]);

  return {
    rows,
    sortedRows,
    activeRow: sortedRows?.[0] ?? null,
    cursor,
    cursorError,
    cursorEnabled,
    grokEnabled,
    codexEnabled,
    deepseekPeakHourWarning,
    codex,
    claudeEnabled,
    claude,
    displayNames,
    showProviderNames,
    themeMode,
    increaseAt,
    failedPolls,
    serverState,
    reload: load,
  };
}
