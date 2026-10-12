"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toastError, toastSuccess } from "@/app/components/toast/toast";
import { apiFetch } from "@/lib/api-fetch";

interface UsageWindow {
  status: string;
  percent: number;
  resetsAt: string;
}

interface Usage {
  rolling: UsageWindow;
  weekly: UsageWindow;
  monthly: UsageWindow;
}

interface AccountRow {
  email: string;
  usage: Usage | null;
  prev: Usage | null;
  enabled: boolean;
  error: string | null;
  stale?: boolean;
  cachedAt?: number | null;
  subscriptionActive?: boolean | null;
  lastChangeAt: number | null;
  lastChangeDetail: string | null;
}

interface CursorUsage {
  autoPercentUsed: number | null;
  apiPercentUsed: number | null;
  totalPercentUsed: number | null;
  billingCycleEnd: string | null;
  accountName: string | null;
  planName: string | null;
  grokPercentUsed: number | null;
  grokResetAt: string | null;
  grokAccount: string | null;
  grokPlanLabel: string | null;
}

interface CodexUsage {
  email: string | null;
  planType: string | null;
  usedPercent: number | null;
  limitWindowSeconds: number | null;
  resetAt: string | null;
  creditsBalance: number | null;
  resetCredits: number | null;
  limitReached: boolean;
}

interface ClaudeWindow {
  percent: number | null;
  resetsAt: string | null;
}

interface ClaudeUsage {
  email: string | null;
  displayName: string | null;
  signedIn: boolean;
  subscribed: boolean;
  subscriptionType: string | null;
  planName: string | null;
  fiveHour: ClaudeWindow;
  weekly: ClaudeWindow;
  resetCredits: number | null;
  billingType: string | null;
  rateLimitTier: string | null;
  orgUuid: string | null;
}

const POLL_MS = 60_000;

function windowColor(w: UsageWindow): string {
  if (w.status === "ok") return w.percent >= 80 ? "bg-amber-400" : "bg-emerald-400";
  return "bg-red-400";
}

function isPassed(resetsAt: string | null | undefined): boolean {
  if (!resetsAt) return false;
  const t = new Date(resetsAt).getTime();
  return Number.isFinite(t) && t <= Date.now();
}

function isUnknownWindow(w: UsageWindow, stale: boolean | undefined): boolean {
  return !!stale && isPassed(w.resetsAt);
}

function isExhaustedWindow(w: UsageWindow, stale?: boolean): boolean {
  if (isUnknownWindow(w, stale)) return false;
  return w.percent >= 100 || w.status === "exhausted";
}

function isExhausted(r: AccountRow): boolean {
  return (
    !!r.usage &&
    (isExhaustedWindow(r.usage.monthly, r.stale) ||
      isExhaustedWindow(r.usage.weekly, r.stale))
  );
}

function earliestReset(r: AccountRow): number {
  const u = r.usage;
  if (!u) return 0;
  const times: number[] = [];
  if (isExhaustedWindow(u.monthly, r.stale)) times.push(new Date(u.monthly.resetsAt).getTime());
  if (isExhaustedWindow(u.weekly, r.stale)) times.push(new Date(u.weekly.resetsAt).getTime());
  return times.length ? Math.min(...times) : 0;
}

function resetsIn(resetsAt: string): string {
  const ms = new Date(resetsAt).getTime() - Date.now();
  if (ms <= 0) return "now";
  const h = Math.floor(ms / 3600000);
  const m = Math.floor((ms % 3600000) / 60000);
  if (h >= 24) return `${Math.floor(h / 24)}d ${h % 24}h`;
  return `${h}h ${m}m`;
}

function utcOf(d: Date, hour: number): number {
  return Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate(), hour, 0, 0);
}

function getPeakInfo(now: Date): { active: boolean; endAt: number | null; nextStartAt: number | null } {
  const h = now.getUTCHours();
  const day = now.getUTCDay();
  const isWeekday = day >= 1 && day <= 5;

  if (isWeekday) {
    if (h >= 1 && h < 4) return { active: true, endAt: utcOf(now, 4), nextStartAt: null };
    if (h >= 6 && h < 10) return { active: true, endAt: utcOf(now, 10), nextStartAt: null };
  }

  const candidates: number[] = [];
  if (isWeekday) {
    if (h < 1) candidates.push(utcOf(now, 1));
    if (h >= 4 && h < 6) candidates.push(utcOf(now, 6));
  }
  for (let i = 1; i <= 7; i++) {
    const d = new Date(now.getTime() + i * 86400000);
    if (d.getUTCDay() >= 1 && d.getUTCDay() <= 5) {
      candidates.push(utcOf(d, 1));
      break;
    }
  }
  const next = candidates
    .filter((c) => c > now.getTime())
    .sort((a, b) => a - b)[0] ?? null;
  return { active: false, endAt: null, nextStartAt: next };
}

function fmtDuration(ms: number): string {
  if (ms <= 0) return "0m";
  const m = Math.floor(ms / 60000);
  const h = Math.floor(m / 60);
  const rem = m % 60;
  if (h > 0) return `${h}h ${rem}m`;
  return `${rem}m`;
}

function resetFmt(resetsAt: string): string {
  const ms = new Date(resetsAt).getTime() - Date.now();
  if (ms <= 0) return "0m";
  const m = Math.floor(ms / 60000);
  const d = Math.floor(m / 1440);
  const h = Math.floor((m % 1440) / 60);
  const mm = m % 60;
  if (d > 0) return `${d}d ${h}h ${mm}m`;
  if (h > 0) return `${h}h ${mm}m`;
  return `${mm}m`;
}

function fmtPct(v: number): string {
  const r = Math.round(v * 10) / 10;
  if (r > 99 && r < 100) return r.toFixed(1);
  return String(Math.round(r));
}

function MiniBar({
  label,
  w,
  dimmed,
  increased,
  light,
  reset,
  unknown,
}: {
  label: string;
  w: UsageWindow;
  dimmed: boolean;
  increased: boolean;
  light: boolean;
  reset: string | null;
  unknown?: boolean;
}) {
  return (
    <div className={`pointer-events-none flex-1 min-w-0 transition-colors duration-700`}>
      <div className="flex justify-between text-[10px] leading-3 mb-0.5">
        <span className={`transition-colors duration-700 ${light ? "text-zinc-600" : "text-zinc-200"}`}>{label}</span>
        <span className={`transition-colors duration-700 ${dimmed || unknown ? "text-zinc-400" : light ? "text-zinc-800" : "text-zinc-100"}`}>
          {unknown ? (
            "--"
          ) : (
            <>
              {!dimmed && increased && (
                <span className={`arrow-pop transition-colors duration-700 ${light ? "text-emerald-600" : "text-emerald-300"}`}>
                  ▲{" "}
                </span>
              )}
              {fmtPct(w.percent)}%
            </>
          )}
        </span>
      </div>
      <div className={`h-1 rounded-full overflow-hidden transition-colors duration-700 ${light ? "bg-zinc-300/70" : "bg-white/15"}`}>
        <div
          className={`h-full rounded-full ${dimmed || unknown ? "bg-zinc-400" : windowColor(w)}`}
          style={{ width: unknown ? "0%" : `${Math.min(w.percent, 100)}%` }}
        />
      </div>
      {reset && (
        <div className={`text-[9px] leading-3 mt-0.5 transition-colors duration-700 ${light ? "text-zinc-500" : "text-zinc-400"}`}>
          {reset}
        </div>
      )}
    </div>
  );
}

export default function Widget() {
  const [rows, setRows] = useState<AccountRow[] | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const [refreshing, setRefreshing] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [bgBright, setBgBright] = useState<number | null>(null);
  const [cursor, setCursor] = useState<CursorUsage | null>(null);
  const [cursorError, setCursorError] = useState(false);
  const [cursorStale, setCursorStale] = useState(false);
  const [cursorCachedAt, setCursorCachedAt] = useState<number | null>(null);
  const [cursorEnabled, setCursorEnabled] = useState(true);
  const [grokEnabled, setGrokEnabled] = useState(true);
  const [codexEnabled, setCodexEnabled] = useState(true);
  const [deepseekPeakHourWarning, setDeepseekPeakHourWarning] = useState(true);
  const [codex, setCodex] = useState<CodexUsage | null>(null);
  const [codexStale, setCodexStale] = useState(false);
  const [codexCachedAt, setCodexCachedAt] = useState<number | null>(null);
  const [claudeEnabled, setClaudeEnabled] = useState(true);
  const [claude, setClaude] = useState<ClaudeUsage | null>(null);
  const [claudeAuthExpired, setClaudeAuthExpired] = useState(false);
  const [codexAuthExpired, setCodexAuthExpired] = useState(false);
  const [displayNames, setDisplayNames] = useState<Record<string, string>>({});
  const [showProviderNames, setShowProviderNames] = useState(false);
  const [themeMode, setThemeMode] = useState<"auto" | "light" | "dark" | "white">("auto");
  const lastChange = useRef<Map<string, { at: number }>>(new Map());
  const increaseAt = useRef<Map<string, number>>(new Map());
  const rootRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const off = (window as any).widget?.onBg?.((v: number) => {
      setBgBright((prev) => {
        const bright = v >= 0.45;
        const dark = v < 0.3;
        const wouldBeLight = prev !== null && prev < 0.5;
        const shouldSwitch =
          (dark && !wouldBeLight) ||
          (bright && wouldBeLight) ||
          prev === null;
        return shouldSwitch ? v : prev;
      });
    });
    return () => off?.();
  }, []);

  useEffect(() => {
    const off = (window as any).widget?.onToggleExpand?.(() => setExpanded((e) => !e));
    return () => off?.();
  }, []);

  useEffect(() => {
    const onCtx = (e: MouseEvent) => {
      e.preventDefault();
      (window as any).widget?.showMenu();
    };
    document.addEventListener("contextmenu", onCtx);
    return () => document.removeEventListener("contextmenu", onCtx);
  }, []);

  const onDragStart = (e: React.MouseEvent) => {
    if (e.button !== 0) return;
    if (
      (e.target as HTMLElement).closest(
        "button, input, textarea, select, a, [data-no-drag]",
      )
    )
      return;
    const w = window as any;
    w.widget?.startDrag(e.screenX, e.screenY);
    const move = (ev: MouseEvent) => w.widget?.moveDrag(ev.screenX, ev.screenY);
    const up = () => {
      w.widget?.endDrag();
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  const light =
    themeMode === "light" || themeMode === "white"
      ? true
      : themeMode === "dark"
        ? false
        : bgBright !== null && bgBright < 0.5;
  const cardBg =
    themeMode === "white" ? "bg-white" : light ? "bg-zinc-200" : "bg-zinc-900";

  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, []);

  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const send = () => {
      const h = Math.ceil(el.getBoundingClientRect().height);
      if (h > 0) (window as any).widget?.resize(238, h + 6);
    };
    const t = setTimeout(send, 80);
    const ro = new ResizeObserver(send);
    ro.observe(el);
    return () => {
      clearTimeout(t);
      ro.disconnect();
    };
  }, []);

  const load = useCallback(async () => {
    setRefreshing(true);
    try {
      const settingsRes = await apiFetch("/api/settings", { cache: "no-store" });
      const sjson = await settingsRes.json();
      const nextCursorEnabled = sjson.settings?.cursorEnabled !== false;
      const nextGrokEnabled = sjson.settings?.grokEnabled !== false;
      const nextCodexEnabled = sjson.settings?.codexEnabled !== false;
      const nextClaudeEnabled = sjson.settings?.claudeEnabled !== false;
      setDeepseekPeakHourWarning(sjson.settings?.deepseekPeakHourWarning !== false);
      setCodexEnabled(nextCodexEnabled);
      setClaudeEnabled(nextClaudeEnabled);
      setCursorEnabled(nextCursorEnabled);
      setGrokEnabled(nextGrokEnabled);
      setDisplayNames(sjson.settings?.displayNames ?? {});
      setShowProviderNames(sjson.settings?.showProviderNames === true);
      setThemeMode(sjson.settings?.themeMode ?? "auto");

      const [usageRes, cursorRes, codexRes, claudeRes] = await Promise.all([
        apiFetch("/api/usage", { cache: "no-store" }),
        apiFetch("/api/cursor", { cache: "no-store" }),
        apiFetch("/api/codex", { cache: "no-store" }),
        apiFetch("/api/claude", { cache: "no-store" }),
      ]);
      const json = await usageRes.json();
      {
        const cjson = await cursorRes.json();
        setCursorError(!!cjson.error);
        setCursor(cjson.usage ?? null);
        setCursorStale(cjson.stale === true);
        setCursorCachedAt(typeof cjson.cachedAt === "number" ? cjson.cachedAt : null);
      }
      {
        const xjson = await codexRes.json();
        setCodex(xjson.usage ?? null);
        setCodexStale(xjson.stale === true);
        setCodexCachedAt(typeof xjson.cachedAt === "number" ? xjson.cachedAt : null);
        setCodexAuthExpired(xjson.authExpired === true);
      }
      {
        const ajson = await claudeRes.json();
        setClaude(ajson.usage ?? null);
        setClaudeAuthExpired(ajson.authExpired === true);
      }
      const accounts: AccountRow[] = json.accounts ?? [];
      const ts = Date.now();
      for (const row of accounts) {
        if (row.lastChangeAt) {
          const prev = lastChange.current.get(row.email);
          if (!prev || row.lastChangeAt >= prev.at) {
            lastChange.current.set(row.email, { at: row.lastChangeAt });
          }
        }
        if (row.usage && row.prev) {
          for (const win of ["rolling", "weekly", "monthly"] as const) {
            const key = `${row.email}:${win}`;
            const cur = row.usage[win];
            const prev = row.prev[win];
            if (!isExhaustedWindow(cur) && cur.percent > prev.percent) {
              if (!increaseAt.current.has(key)) increaseAt.current.set(key, ts);
            } else {
              increaseAt.current.delete(key);
            }
          }
        }
      }
      setRows(accounts);
    } catch {
      // widget stays silent on transient errors
    } finally {
      setRefreshing(false);
    }
  }, []);

  useEffect(() => {
    load();
    const id = setInterval(load, POLL_MS);
    return () => clearInterval(id);
  }, [load]);

  useEffect(() => {
    const w = (window as any).widget;
    const offSettings = w?.onSettingsChanged?.(() => load());
    const offRefresh = w?.onRefresh?.(() => load());
    return () => {
      offSettings?.();
      offRefresh?.();
    };
  }, [load]);

  const sortedRows = useMemo(() => {
    if (!rows) return null;
    const lastChanged = (r: AccountRow) => lastChange.current.get(r.email)?.at ?? 0;
    return rows
      .filter((r) => r.enabled !== false && r.subscriptionActive !== false)
      .sort((a, b) => {
        const ea = isExhausted(a);
        const eb = isExhausted(b);
        if (ea !== eb) return ea ? 1 : -1;
        if (ea) return earliestReset(a) - earliestReset(b);
        return lastChanged(b) - lastChanged(a);
      });
  }, [rows]);

  const activeRow = sortedRows?.[0] ?? null;

  const copyKey = async (email: string) => {
    try {
      const res = await apiFetch(`/api/key?email=${encodeURIComponent(email)}`);
      if (!res.ok) {
        toastError("Copy failed");
        return;
      }
      const json = await res.json();
      if (typeof json.key !== "string" || !json.key) {
        toastError("Copy failed");
        return;
      }
      await navigator.clipboard.writeText(json.key);
      toastSuccess("Key copied");
    } catch {
      toastError("Copy failed");
    }
  };

  const reauthHint = (expired: boolean, how: string) =>
    !expired ? null : (
      <span
        className={`text-[9px] font-medium whitespace-nowrap ${light ? "text-amber-700" : "text-amber-300"}`}
        title={`Sign-in expired · ${how} to refresh it`}
      >
        re-auth
      </span>
    );

  const cachedHint = (stale: boolean, cachedAt: number | null) =>
    !stale ? null : (
      <span
        className={`text-[9px] font-medium whitespace-nowrap ${light ? "text-zinc-400" : "text-zinc-500"}`}
        title={
          cachedAt
            ? `Last known data · ${new Date(cachedAt).toLocaleString()}`
            : "Last known data"
        }
      >
        cached
      </span>
    );

  const renderCard = (row: AccountRow, withControls: boolean) => {
    const disabled = row.enabled === false;
    const isInUse = !disabled && sortedRows?.[0]?.email === row.email;
    const exhausted = isExhausted(row);
    const stale = row.stale === true;
    const rollingUnknown = !!row.usage && isUnknownWindow(row.usage.rolling, stale);
    const weeklyUnknown = !!row.usage && isUnknownWindow(row.usage.weekly, stale);
    const monthlyUnknown = !!row.usage && isUnknownWindow(row.usage.monthly, stale);
    const showInc = (win: string) => {
      const at = increaseAt.current.get(`${row.email}:${win}`);
      return !!at && Date.now() - at < 20000;
    };
    return (
      <div
        key={row.email}
        className={`relative rounded-lg p-2 transition-colors duration-700 ${cardBg} ${
          disabled
            ? `${light ? "border border-zinc-300/70" : "border border-white/10"} opacity-55`
            : exhausted
              ? light
                ? "border border-red-400/80 shadow-[0_0_14px_rgba(239,68,68,0.45)]"
                : "border border-red-400/70 shadow-[0_0_14px_rgba(239,68,68,0.45)]"
              : isInUse
                ? light
                  ? "border border-emerald-500/80 shadow-[0_0_14px_rgba(16,185,129,0.45)]"
                  : "border border-emerald-400/70 shadow-[0_0_14px_rgba(16,185,129,0.45)]"
                : light
                  ? "border border-zinc-400/50"
                  : "border border-white/15"
        }`}
      >
        <div
          className="absolute inset-0 z-0 cursor-move"
          onMouseDown={onDragStart}
        />
        <div className="relative z-10 pointer-events-none">
        <div
          className="flex items-center gap-1.5 mb-1.5"
        >
          <button
            onClick={() => copyKey(row.email)}
            title="Copy key"
            className="pointer-events-auto shrink-0 rounded-[3px] p-0.5 hover:bg-white/10 transition-colors"
            style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
          >
            <img
              src="/opencode.ico"
              alt="copy key"
              className="h-3 w-3 rounded-[3px]"
            />
          </button>
          <span className={`text-[11px] truncate font-medium ${light ? "text-zinc-800" : "text-zinc-100"}`}>
            {showProviderNames ? "OpenCode" : displayNames[row.email] ?? row.email.slice(0, 4)}
          </span>
          {deepseekPeakHourWarning && withControls && peakInfo.active && peakInfo.endAt ? (
            <span className={`text-[9px] font-medium whitespace-nowrap ${light ? "text-red-600" : "text-red-300"}`}>
              Peak Ends in {fmtDuration(peakInfo.endAt - now)}
            </span>
          ) : deepseekPeakHourWarning && withControls && peakInfo.nextStartAt ? (
            <span className={`text-[9px] font-medium whitespace-nowrap ${light ? "text-emerald-600" : "text-emerald-300"}`}>
              Peak Starts in {fmtDuration(peakInfo.nextStartAt - now)}
            </span>
          ) : null}
          {cachedHint(stale, row.cachedAt ?? null)}
          <span
            className={`ml-auto text-[9px] font-semibold px-1.5 py-0.5 rounded-full ${
              light
                ? "bg-blue-100 text-blue-700 border border-blue-300"
                : "bg-blue-500/15 text-blue-300 border border-blue-400/30"
            }`}
          >
            Go
          </span>
        </div>
        {row.error ? (
          <div className="text-[10px] text-red-400">unavailable</div>
        ) : row.usage ? (
          <div className="flex gap-2">
            <MiniBar
              label="R"
              w={row.usage.rolling}
              dimmed={
                isExhaustedWindow(row.usage.weekly, stale) ||
                isExhaustedWindow(row.usage.monthly, stale)
              }
              increased={showInc("rolling")}
              light={light}
              unknown={rollingUnknown}
              reset={
                withControls && !rollingUnknown
                  ? resetFmt(row.usage.rolling.resetsAt)
                  : null
              }
            />
            <MiniBar
              label="W"
              w={row.usage.weekly}
              dimmed={isExhaustedWindow(row.usage.monthly, stale)}
              increased={showInc("weekly")}
              light={light}
              unknown={weeklyUnknown}
              reset={
                withControls && !weeklyUnknown
                  ? resetFmt(row.usage.weekly.resetsAt)
                  : null
              }
            />
            <MiniBar
              label="M"
              w={row.usage.monthly}
              dimmed={false}
              increased={showInc("monthly")}
              light={light}
              unknown={monthlyUnknown}
              reset={monthlyUnknown ? null : resetFmt(row.usage.monthly.resetsAt)}
            />
          </div>
        ) : null}
        </div>
      </div>
    );
  };

  const renderCursorCard = () => {
    if (!cursorEnabled) return null;
    if (!cursor) {
      const cardClass = `relative rounded-lg p-2 transition-colors duration-700 ${cardBg} ${light ? "border border-zinc-400/50" : "border border-white/15"}`;
      return (
        <div className={cardClass}>
          <div className="absolute inset-0 z-0 cursor-move" onMouseDown={onDragStart} />
          <div className="relative z-10 pointer-events-none">
          <div className="flex items-center gap-1.5 mb-1.5">
            <img
              src="/cursor.ico"
              alt="cursor"
              className="h-3 w-3 shrink-0 rounded-[3px]"
            />
            <span className={`text-[11px] truncate flex-1 font-medium ${light ? "text-zinc-800" : "text-zinc-100"}`}>
              crsr
            </span>
          </div>
          <div className={`text-[10px] ${light ? "text-zinc-500" : "text-zinc-400"}`}>
            {cursorError ? "not signed in or unavailable" : "…"}
          </div>
          </div>
        </div>
      );
    }
    const total = cursor.totalPercentUsed ?? 0;
    const stale = cursorStale;
    const cyclePassed = stale && isPassed(cursor.billingCycleEnd);
    const exhausted = !cyclePassed && total >= 100;
    const cardClass = `relative rounded-lg p-2 transition-colors duration-700 ${cardBg} ${
      exhausted
        ? light
          ? "border border-red-400/80 shadow-[0_0_14px_rgba(239,68,68,0.45)]"
          : "border border-red-400/70 shadow-[0_0_14px_rgba(239,68,68,0.45)]"
        : light
          ? "border border-zinc-400/50"
          : "border border-white/15"
    }`;
    return (
      <div className={cardClass}>
        <div className="absolute inset-0 z-0 cursor-move" onMouseDown={onDragStart} />
        <div className="relative z-10 pointer-events-none">
        <div className="flex items-center gap-1.5 mb-1.5">
          <img
            src="/cursor.ico"
            alt="cursor"
            className="h-3 w-3 shrink-0 rounded-[3px]"
          />
          <span
            className={`text-[11px] truncate flex-1 font-medium ${light ? "text-zinc-800" : "text-zinc-100"}`}
            title={cursor?.accountName ?? ""}
          >
            {showProviderNames ? "Cursor" : displayNames["cursor"] ?? cursor?.accountName ?? "Cursor"}
          </span>
          {cachedHint(stale, cursorCachedAt)}
          {cursor?.planName && (
            <span
              className={`ml-auto text-[9px] font-semibold px-1.5 py-0.5 rounded-full ${
                light
                  ? "bg-blue-100 text-blue-700 border border-blue-300"
                  : "bg-blue-500/15 text-blue-300 border border-blue-400/30"
              }`}
            >
              {cursor.planName}
            </span>
          )}
        </div>
        {cursor ? (
          <>
            <div className="flex gap-2">
              <MiniBar
                label="First Party"
                w={{ status: "ok", percent: cursor.autoPercentUsed ?? 0, resetsAt: "" }}
                dimmed={false}
                increased={false}
                light={light}
                unknown={cyclePassed}
                reset={null}
              />
              <MiniBar
                label="API"
                w={{ status: "ok", percent: cursor.apiPercentUsed ?? 0, resetsAt: "" }}
                dimmed={false}
                increased={false}
                light={light}
                unknown={cyclePassed}
                reset={null}
              />
            </div>
            {cursor.billingCycleEnd && !cyclePassed && (
              <div className={`text-[9px] leading-3 mt-1 transition-colors duration-700 ${light ? "text-zinc-500" : "text-zinc-400"}`}>
                Resets {resetFmt(cursor.billingCycleEnd)}
              </div>
            )}
          </>
        ) : (
          <div className="text-[10px] text-zinc-400">unavailable</div>
        )}
        </div>
      </div>
    );
  };

  const renderGrokCard = () => {
    if (!grokEnabled) return null;
    if (cursor?.grokPercentUsed == null) return null;
    const pct = cursor.grokPercentUsed;
    const stale = cursorStale;
    const grokPassed = stale && isPassed(cursor.grokResetAt);
    const exhausted = !grokPassed && pct >= 100;
    const cardClass = `relative rounded-lg p-2 transition-colors duration-700 ${cardBg} ${
      exhausted
        ? light
          ? "border border-red-400/80 shadow-[0_0_14px_rgba(239,68,68,0.45)]"
          : "border border-red-400/70 shadow-[0_0_14px_rgba(239,68,68,0.45)]"
        : light
          ? "border border-zinc-400/50"
          : "border border-white/15"
    }`;
    return (
      <div className={cardClass}>
        <div className="absolute inset-0 z-0 cursor-move" onMouseDown={onDragStart} />
        <div className="relative z-10 pointer-events-none">
        <div
          className="flex items-center gap-1.5 mb-1.5"
        >
          <img
            src="/grokbot.ico"
            alt="grok bot"
            className="h-3 w-3 shrink-0 rounded-[3px]"
          />
          <span
            className={`text-[11px] truncate flex-1 font-medium ${light ? "text-zinc-800" : "text-zinc-100"}`}
            title={cursor.grokAccount ?? ""}
          >
            {showProviderNames ? "Grok Bot" : displayNames["grok"] ?? "Grok Bot"}
          </span>
          {cachedHint(stale, cursorCachedAt)}
          {(cursor.grokPlanLabel ?? cursor.planName) && (
            <span
              className={`ml-auto text-[9px] font-semibold px-1.5 py-0.5 rounded-full ${
                light
                  ? "bg-blue-100 text-blue-700 border border-blue-300"
                  : "bg-blue-500/15 text-blue-300 border border-blue-400/30"
              }`}
            >
              {cursor.grokPlanLabel ?? cursor.planName}
            </span>
          )}
        </div>
        <div className="flex gap-2">
          <MiniBar
            label="Weekly"
            w={{ status: "ok", percent: pct, resetsAt: "" }}
            dimmed={false}
            increased={false}
            light={light}
            unknown={grokPassed}
            reset={null}
          />
        </div>
        {cursor.grokResetAt && !grokPassed && (
          <div className={`text-[9px] leading-3 mt-1 transition-colors duration-700 ${light ? "text-zinc-500" : "text-zinc-400"}`}>
            Resets {resetFmt(cursor.grokResetAt)}
          </div>
        )}
        </div>
      </div>
    );
  };

  const renderCodexCard = () => {
    if (!codexEnabled) return null;
    const pct = codex?.usedPercent ?? 0;
    const stale = codexStale;
    const resetPassed = stale && isPassed(codex?.resetAt);
    const exhausted = !resetPassed && (codex?.limitReached === true || pct >= 100);
    const cardClass = `relative rounded-lg p-2 transition-colors duration-700 ${cardBg} ${
      exhausted
        ? light
          ? "border border-red-400/80 shadow-[0_0_14px_rgba(239,68,68,0.45)]"
          : "border border-red-400/70 shadow-[0_0_14px_rgba(239,68,68,0.45)]"
        : light
          ? "border border-zinc-400/50"
          : "border border-white/15"
    }`;
    return (
      <div className={cardClass}>
        <div className="absolute inset-0 z-0 cursor-move" onMouseDown={onDragStart} />
        <div className="relative z-10 pointer-events-none">
        <div
          className="flex items-center gap-1.5 mb-1.5"
        >
          <img
            src="/openai.png"
            alt="codex"
            className={`h-3 w-3 shrink-0 ${light ? "invert" : ""}`}
          />
          <span className={`text-[11px] truncate flex-1 font-medium ${light ? "text-zinc-800" : "text-zinc-100"}`}>
            {showProviderNames ? "Codex" : displayNames["codex"] ?? "Codex"}
          </span>
          {reauthHint(codexAuthExpired, "run codex")}
          {cachedHint(stale, codexCachedAt)}
          <span className="ml-auto flex items-center gap-1 shrink-0">
            {codex?.resetCredits != null && (
              <span
                title={`${codex.resetCredits} rate-limit reset${codex.resetCredits === 1 ? "" : "s"} available`}
                className={`text-[9px] font-semibold px-1.5 py-0.5 rounded-full border ${
                  codex.resetCredits > 0
                    ? light
                      ? "bg-violet-100 text-violet-700 border-violet-300"
                      : "bg-violet-500/15 text-violet-300 border-violet-400/30"
                    : light
                      ? "bg-zinc-200 text-zinc-500 border-zinc-300"
                      : "bg-zinc-500/15 text-zinc-400 border-zinc-500/30"
                }`}
              >
                ↺ {codex.resetCredits}
              </span>
            )}
            {codex?.planType && (
              <span
                className={`text-[9px] font-semibold px-1.5 py-0.5 rounded-full ${
                  codex.planType.toLowerCase() === "free"
                    ? light
                      ? "bg-emerald-100 text-emerald-700 border border-emerald-300"
                      : "bg-emerald-500/15 text-emerald-300 border border-emerald-400/30"
                    : light
                      ? "bg-blue-100 text-blue-700 border border-blue-300"
                      : "bg-blue-500/15 text-blue-300 border border-blue-400/30"
                }`}
              >
                {codex.planType.charAt(0).toUpperCase() + codex.planType.slice(1)}
              </span>
            )}
          </span>
        </div>
        {codex ? (
          <>
            <div className="flex gap-2">
              <MiniBar
                label="Usage"
                w={{ status: "ok", percent: pct, resetsAt: "" }}
                dimmed={false}
                increased={false}
                light={light}
                unknown={resetPassed}
                reset={null}
              />
            </div>
            {codex.resetAt && !resetPassed && (
              <div className={`text-[9px] leading-3 mt-1 transition-colors duration-700 ${light ? "text-zinc-500" : "text-zinc-400"}`}>
                Resets {resetFmt(codex.resetAt)}
              </div>
            )}
          </>
        ) : (
          <div className="text-[10px] text-zinc-400">not signed in or unavailable</div>
        )}
        </div>
      </div>
    );
  };

  const renderClaudeCard = () => {
    if (!claudeEnabled) return null;
    const signedIn = !!claude?.signedIn;
    const badge = !claude
      ? "n/a"
      : !claude.signedIn
        ? "signed out"
        : claude.planName ?? (!claude.subscribed ? "Free" : claude.subscriptionType ?? "subscribed");
    const badgeClass =
      badge === "Free"
        ? light
          ? "bg-emerald-100 text-emerald-700 border border-emerald-300"
          : "bg-emerald-500/15 text-emerald-300 border border-emerald-400/30"
        : badge === "n/a" || badge === "signed out"
          ? light
            ? "bg-zinc-200 text-zinc-500 border border-zinc-300"
            : "bg-zinc-500/15 text-zinc-400 border border-zinc-500/30"
          : light
            ? "bg-blue-100 text-blue-700 border border-blue-300"
            : "bg-blue-500/15 text-blue-300 border border-blue-400/30";
    const fh = claude?.fiveHour;
    const wk = claude?.weekly;
    const cardClass = `relative rounded-lg p-2 transition-colors duration-700 ${cardBg} ${light ? "border border-zinc-400/50" : "border border-white/15"}`;
    return (
      <div className={cardClass}>
        <div className="absolute inset-0 z-0 cursor-move" onMouseDown={onDragStart} />
        <div className="relative z-10 pointer-events-none">
        <div
          className="flex items-center gap-1.5 mb-1.5"
        >
          <img
            src="/claude.ico"
            alt="claude"
            className="h-3 w-3 shrink-0 rounded-[3px]"
          />
          <span
            className={`text-[11px] truncate flex-1 font-medium ${light ? "text-zinc-800" : "text-zinc-100"}`}
            title={claude?.email ?? ""}
          >
            {showProviderNames ? "Claude" : displayNames["claude"] ?? "Claude"}
          </span>
          {reauthHint(claudeAuthExpired, "open Claude Code")}
          <span className="ml-auto flex items-center gap-1 shrink-0">
            {claude?.resetCredits != null && (
              <span
                title={`${claude.resetCredits} reset${claude.resetCredits === 1 ? "" : "s"} available`}
                className={`text-[9px] font-semibold px-1.5 py-0.5 rounded-full border ${
                  claude.resetCredits > 0
                    ? light
                      ? "bg-violet-100 text-violet-700 border-violet-300"
                      : "bg-violet-500/15 text-violet-300 border-violet-400/30"
                    : light
                      ? "bg-zinc-200 text-zinc-500 border-zinc-300"
                      : "bg-zinc-500/15 text-zinc-400 border-zinc-500/30"
                }`}
              >
                ↺ {claude.resetCredits}
              </span>
            )}
            <span className={`text-[9px] font-semibold px-1.5 py-0.5 rounded-full ${badgeClass}`}>
              {badge}
            </span>
          </span>
        </div>
        {!signedIn ? (
          <div className={`text-[10px] ${light ? "text-zinc-500" : "text-zinc-400"}`}>
            {!claude ? "unavailable" : "not signed in"}
          </div>
        ) : (
          <div className="flex gap-2">
            <MiniBar
              label="5h"
              w={{ status: "ok", percent: fh?.percent ?? 0, resetsAt: "" }}
              dimmed={false}
              increased={false}
              light={light}
              unknown={fh?.percent == null}
              reset={fh?.resetsAt && fh.percent != null ? resetFmt(fh.resetsAt) : null}
            />
            <MiniBar
              label="Weekly"
              w={{ status: "ok", percent: wk?.percent ?? 0, resetsAt: "" }}
              dimmed={false}
              increased={false}
              light={light}
              unknown={wk?.percent == null}
              reset={wk?.resetsAt && wk.percent != null ? resetFmt(wk.resetsAt) : null}
            />
          </div>
        )}
        </div>
      </div>
    );
  };

  const peakInfo = useMemo(() => getPeakInfo(new Date(now)), [now]);

  return (
    <div
      ref={rootRef}
      className="w-screen p-1.5 select-none cursor-move"
      onMouseDown={onDragStart}
    >
      <div
        className={
          expanded
            ? "space-y-1.5"
            : ""
        }
      >
          {rows === null ? (
            <div className={`text-[11px] px-1 py-2 ${light ? "text-zinc-500" : "text-zinc-400"}`}>
              Loading…
            </div>
          ) : expanded ? (
            <>
              {sortedRows?.map((row, i) => renderCard(row, i === 0))}
              {renderCursorCard()}
              {renderGrokCard()}
              {renderCodexCard()}
              {renderClaudeCard()}
            </>
          ) : (
            <>
              {activeRow ? renderCard(activeRow, true) : null}
              {renderCursorCard()}
              {renderGrokCard()}
              {renderCodexCard()}
              {renderClaudeCard()}
            </>
          )}
      </div>
    </div>
  );
}
