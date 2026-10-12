"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { getJson, onSettingsChanged } from "@/lib/api-fetch";
import { increasedWindows } from "@/lib/ranking";
import type {
  ClaudeUsage,
  CodexUsage,
  CursorUsage,
  GrokUsage,
  OpenCodeAccountUsage,
  ProviderResponse,
  Settings,
} from "@/lib/types";

export const POLL_MS = 60_000;
const INCREASE_VISIBLE_MS = 20_000;

export interface ProviderSlot<T> {
  data: T | null;
  stale: boolean;
  cachedAt: number | null;
  authExpired: boolean;
  /** set once the first response (or failure) has arrived */
  loaded: boolean;
}

const emptySlot = <T,>(): ProviderSlot<T> => ({
  data: null,
  stale: false,
  cachedAt: null,
  authExpired: false,
  loaded: false,
});

export interface WidgetData {
  settings: Settings | null;
  accounts: OpenCodeAccountUsage[] | null;
  cursor: ProviderSlot<CursorUsage>;
  grok: ProviderSlot<GrokUsage>;
  codex: ProviderSlot<CodexUsage>;
  claude: ProviderSlot<ClaudeUsage>;
  /** last time at least one request succeeded */
  updatedAt: number | null;
  /** the most recent refresh could not reach the local server at all */
  offline: boolean;
  /** first refresh has finished (successfully or not) */
  ready: boolean;
}

/**
 * Polls every endpoint independently: one slow or failing provider never
 * blocks or blanks the others.
 */
export function useWidgetData() {
  const [data, setData] = useState<WidgetData>({
    settings: null,
    accounts: null,
    cursor: emptySlot(),
    grok: emptySlot(),
    codex: emptySlot(),
    claude: emptySlot(),
    updatedAt: null,
    offline: false,
    ready: false,
  });
  const lastUsedAt = useRef(new Map<string, number>());
  const increaseAt = useRef(new Map<string, number>());
  const inflight = useRef(false);

  const load = useCallback(async () => {
    if (inflight.current) return;
    inflight.current = true;
    try {
      const [settings, usage, cursor, grok, codex, claude] = await Promise.allSettled([
        getJson<{ settings: Settings }>("/api/settings"),
        getJson<{ accounts: OpenCodeAccountUsage[] }>("/api/usage"),
        getJson<ProviderResponse<CursorUsage>>("/api/cursor"),
        getJson<ProviderResponse<GrokUsage>>("/api/grok"),
        getJson<ProviderResponse<CodexUsage>>("/api/codex"),
        getJson<ProviderResponse<ClaudeUsage>>("/api/claude"),
      ]);
      const now = Date.now();

      if (usage.status === "fulfilled") {
        for (const row of usage.value.accounts) {
          if (row.lastChangeAt) {
            const prev = lastUsedAt.current.get(row.email) ?? 0;
            if (row.lastChangeAt >= prev) lastUsedAt.current.set(row.email, row.lastChangeAt);
          }
          if (!row.usage) continue;
          const up = new Set(increasedWindows(row.usage, row.prev));
          for (const win of ["rolling", "weekly", "monthly"] as const) {
            const key = `${row.email}:${win}`;
            if (!up.has(win)) increaseAt.current.delete(key);
            else if (!increaseAt.current.has(key)) increaseAt.current.set(key, now);
          }
        }
      }

      const slot = <T,>(
        prev: ProviderSlot<T>,
        r: PromiseSettledResult<ProviderResponse<T>>,
      ): ProviderSlot<T> =>
        r.status === "fulfilled"
          ? {
              data: r.value.usage,
              stale: r.value.stale,
              cachedAt: r.value.cachedAt,
              authExpired: r.value.authExpired,
              loaded: true,
            }
          : { ...prev, loaded: true }; // keep showing what we had

      const all = [settings, usage, cursor, grok, codex, claude];
      const anyOk = all.some((r) => r.status === "fulfilled");
      setData((prev) => ({
        settings: settings.status === "fulfilled" ? settings.value.settings : prev.settings,
        accounts: usage.status === "fulfilled" ? usage.value.accounts : prev.accounts,
        cursor: slot(prev.cursor, cursor),
        grok: slot(prev.grok, grok),
        codex: slot(prev.codex, codex),
        claude: slot(prev.claude, claude),
        updatedAt: anyOk ? now : prev.updatedAt,
        offline: !anyOk,
        ready: true,
      }));
    } finally {
      inflight.current = false;
    }
  }, []);

  useEffect(() => {
    void load();
    const id = setInterval(load, POLL_MS);
    const w = window.widget;
    const offs = [
      w?.onSettingsChanged(() => void load()),
      w?.onRefresh(() => void load()),
      onSettingsChanged(() => void load()),
    ];
    return () => {
      clearInterval(id);
      offs.forEach((off) => off?.());
    };
  }, [load]);

  const lastUsed = useCallback((email: string) => lastUsedAt.current.get(email) ?? 0, []);
  const increased = useCallback((email: string, win: string, now: number) => {
    const at = increaseAt.current.get(`${email}:${win}`);
    return !!at && now - at < INCREASE_VISIBLE_MS;
  }, []);

  return { data, reload: load, lastUsed, increased };
}
