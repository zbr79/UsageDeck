import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getJson, onSettingsChanged } from "@/lib/api";
import { onRefresh } from "@/lib/bridge";
import { increasedWindows } from "@/lib/ranking";
import type {
  ClaudeUsage,
  CodexUsage,
  CursorUsage,
  GrokUsage,
  OpenCodeAccountUsage,
  ProviderErrorKind,
  Settings,
  TrackedAccount,
} from "@/lib/types";

export const POLL_MS = 60_000;
const INCREASE_VISIBLE_MS = 20_000;

export interface ProviderSlot<T> {
  data: T | null;
  stale: boolean;
  cachedAt: number | null;
  authExpired: boolean;
  /** why the last fetch failed; null when it worked */
  error: ProviderErrorKind | null;
  /** set once the first response (or failure) has arrived */
  loaded: boolean;
}

/** The card slot for one tracked account; `null` gives the empty "not signed in" slot. */
export function slotFromAccount<T>(
  account: TrackedAccount<T> | null,
  loaded = true,
): ProviderSlot<T> {
  return {
    data: account?.usage ?? null,
    stale: account?.stale ?? false,
    cachedAt: account?.cachedAt ?? null,
    authExpired: account?.authExpired ?? false,
    error: account?.error ?? null,
    loaded,
  };
}

export interface AccountList<T> {
  accounts: TrackedAccount<T>[];
  loaded: boolean;
}

const emptyList = <T>(): AccountList<T> => ({ accounts: [], loaded: false });

export interface WidgetData {
  settings: Settings | null;
  accounts: OpenCodeAccountUsage[] | null;
  cursor: AccountList<CursorUsage>;
  grok: AccountList<GrokUsage>;
  codex: AccountList<CodexUsage>;
  claude: AccountList<ClaudeUsage>;
  /** last time at least one request succeeded */
  updatedAt: number | null;
  /** every request of the most recent refresh failed */
  offline: boolean;
  /** first refresh has finished (successfully or not) */
  ready: boolean;
}

/**
 * Wrap `task` so only one run is in flight. A call made while a run is in flight queues one
 * follow-up run (several calls fold into one) instead of being dropped, so a reload asked for
 * after settings were saved always reads them. Every caller's promise settles when the queue drains.
 */
export function queuedRunner(task: () => Promise<void>): () => Promise<void> {
  let inflight: Promise<void> | null = null;
  let again = false;
  return () => {
    if (inflight) {
      again = true;
      return inflight;
    }
    inflight = (async () => {
      try {
        do {
          again = false;
          await task().catch(() => undefined);
        } while (again);
      } finally {
        inflight = null;
      }
    })();
    return inflight;
  };
}

/**
 * Polls every endpoint independently: one slow or failing provider never
 * blocks or blanks the others.
 */
export function useWidgetData() {
  const [data, setData] = useState<WidgetData>({
    settings: null,
    accounts: null,
    cursor: emptyList(),
    grok: emptyList(),
    codex: emptyList(),
    claude: emptyList(),
    updatedAt: null,
    offline: false,
    ready: false,
  });
  const lastUsedAt = useRef(new Map<string, number>());
  const increaseAt = useRef(new Map<string, number>());

  const loadOnce = useCallback(async () => {
    const [settings, usage, cursor, grok, codex, claude] = await Promise.allSettled([
      getJson<{ settings: Settings }>("/api/settings"),
      getJson<{ accounts: OpenCodeAccountUsage[] }>("/api/usage"),
      getJson<{ accounts: TrackedAccount<CursorUsage>[] }>("/api/cursor"),
      getJson<{ accounts: TrackedAccount<GrokUsage>[] }>("/api/grok"),
      getJson<{ accounts: TrackedAccount<CodexUsage>[] }>("/api/codex"),
      getJson<{ accounts: TrackedAccount<ClaudeUsage>[] }>("/api/claude"),
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

    const list = <T>(
      prev: AccountList<T>,
      r: PromiseSettledResult<{ accounts: TrackedAccount<T>[] }>,
    ): AccountList<T> =>
      r.status === "fulfilled"
        ? { accounts: r.value.accounts ?? [], loaded: true }
        : { ...prev, loaded: true };

    const all = [settings, usage, cursor, grok, codex, claude];
    const anyOk = all.some((r) => r.status === "fulfilled");
    setData((prev) => ({
      settings: settings.status === "fulfilled" ? settings.value.settings : prev.settings,
      accounts: usage.status === "fulfilled" ? usage.value.accounts : prev.accounts,
      cursor: list(prev.cursor, cursor),
      grok: list(prev.grok, grok),
      codex: list(prev.codex, codex),
      claude: list(prev.claude, claude),
      updatedAt: anyOk ? now : prev.updatedAt,
      offline: !anyOk,
      ready: true,
    }));
  }, []);

  const load = useMemo(() => queuedRunner(loadOnce), [loadOnce]);

  useEffect(() => {
    void load();
    const id = setInterval(load, POLL_MS);
    // onSettingsChanged covers both the BroadcastChannel and the native
    // `widget-settings-changed` event.
    const offs = [onRefresh(() => void load()), onSettingsChanged(() => void load())];
    return () => {
      clearInterval(id);
      offs.forEach((off) => off());
    };
  }, [load]);

  const lastUsed = useCallback((email: string) => lastUsedAt.current.get(email) ?? 0, []);
  const increased = useCallback((email: string, win: string, now: number) => {
    const at = increaseAt.current.get(`${email}:${win}`);
    return !!at && now - at < INCREASE_VISIBLE_MS;
  }, []);

  return { data, reload: load, lastUsed, increased };
}
