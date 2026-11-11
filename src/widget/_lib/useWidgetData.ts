import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { announceSettingsChanged, getJson, onSettingsChanged, sendJson } from "@/lib/api";
import { onRefresh } from "@/lib/bridge";
import { isAuthRejected } from "@/lib/errors";
import { increasedWindows } from "@/lib/ranking";
import { applySettingsPatch, type SettingsPatch } from "@/lib/settings-schema";
import type {
  ClaudePace,
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
/** After "Open Cursor": poll this often, for this long, to show the sign-in Cursor renews. */
export const FAST_POLL_MS = 10_000;
export const FAST_POLL_FOR_MS = 120_000;
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
  /** Claude spend and pace; null until loaded or when the last call failed (the card shows nothing) */
  pace: ClaudePace | null;
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
 * Runs `load` every `everyMs` for `forMs` after `start()`, and stops early once `done()` is true.
 * `start()` while running begins a new window; `stop()` ends it.
 */
export function burstPoller(
  load: () => unknown,
  done: () => boolean,
  everyMs = FAST_POLL_MS,
  forMs = FAST_POLL_FOR_MS,
) {
  let timer: ReturnType<typeof setInterval> | null = null;
  const stop = () => {
    if (timer) clearInterval(timer);
    timer = null;
  };
  const start = () => {
    stop();
    const until = Date.now() + forMs;
    timer = setInterval(() => {
      if (done() || Date.now() >= until) stop();
      else void load();
    }, everyMs);
  };
  return { start, stop };
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
    pace: null,
    updatedAt: null,
    offline: false,
    ready: false,
  });
  const lastUsedAt = useRef(new Map<string, number>());
  const increaseAt = useRef(new Map<string, number>());
  const cursorRejected = useRef(false);

  const loadOnce = useCallback(async () => {
    const [settings, usage, cursor, grok, codex, claude, pace] = await Promise.allSettled([
      getJson<{ settings: Settings }>("/api/settings"),
      getJson<{ accounts: OpenCodeAccountUsage[] }>("/api/usage"),
      getJson<{ accounts: TrackedAccount<CursorUsage>[] }>("/api/cursor"),
      getJson<{ accounts: TrackedAccount<GrokUsage>[] }>("/api/grok"),
      getJson<{ accounts: TrackedAccount<CodexUsage>[] }>("/api/codex"),
      getJson<{ accounts: TrackedAccount<ClaudeUsage>[] }>("/api/claude"),
      getJson<ClaudePace>("/api/claude/pace"),
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

    if (cursor.status === "fulfilled") {
      cursorRejected.current = (cursor.value.accounts ?? []).some((a) =>
        isAuthRejected(a.authExpired, a.error),
      );
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
      // Its own slot: a failed pace call never counts toward "offline" or touches the Claude card.
      pace: pace.status === "fulfilled" ? pace.value : null,
      updatedAt: anyOk ? now : prev.updatedAt,
      offline: !anyOk,
      ready: true,
    }));
  }, []);

  const load = useMemo(() => queuedRunner(loadOnce), [loadOnce]);
  const cursorWatch = useMemo(() => burstPoller(load, () => !cursorRejected.current), [load]);
  useEffect(() => cursorWatch.stop, [cursorWatch]);

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

  /** Save a settings change from the widget: shown at once, then whatever the store kept. */
  const updateSettings = useCallback(
    async (patch: SettingsPatch) => {
      setData((prev) =>
        prev.settings ? { ...prev, settings: applySettingsPatch(prev.settings, patch) } : prev,
      );
      try {
        await sendJson("/api/settings", "POST", patch);
        announceSettingsChanged();
      } finally {
        void load();
      }
    },
    [load],
  );

  return {
    data,
    reload: load,
    lastUsed,
    increased,
    updateSettings,
    /** after "Open Cursor": poll fast until Cursor's renewed sign-in shows */
    watchCursorSignIn: cursorWatch.start,
  };
}
