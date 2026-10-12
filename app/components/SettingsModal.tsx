"use client";

import { useEffect, useRef, useState } from "react";
import { toastError, toastSuccess } from "@/app/components/toast/toast";
import { apiFetch } from "@/lib/api-fetch";

interface AccountRow {
  email: string;
  keyHint: string;
}

interface UsageWindowInfo {
  status: string;
  percent: number;
  resetsAt: string;
}

interface AccountUsage {
  usage: {
    rolling: UsageWindowInfo;
    weekly: UsageWindowInfo;
    monthly: UsageWindowInfo;
  } | null;
  stale: boolean;
  cachedAt: number | null;
  subscriptionActive?: boolean | null;
}

const CACHED_PILL = "bg-zinc-500/15 text-zinc-400 border border-zinc-500/30";

function resetInfo(resetsAt: string): { short: string; full: string } | null {
  const ms = new Date(resetsAt).getTime() - Date.now();
  if (!Number.isFinite(ms) || ms <= 0) return null;
  const totalM = Math.floor(ms / 60000);
  const d = Math.floor(totalM / 1440);
  const h = Math.floor((totalM % 1440) / 60);
  const m = totalM % 60;
  const short = d > 0 ? `${d}d` : h > 0 ? `${h}h` : `${m}m`;
  const full = d > 0 ? `${d}d ${h}h` : h > 0 ? `${h}h ${m}m` : `${m}m`;
  return { short, full };
}

type Tab = "opencode" | "cursor" | "grok" | "codex" | "claude";

export default function SettingsModal({
  onClose,
  onChanged,
}: {
  onClose: () => void;
  onChanged: () => void;
}) {
  const [tab, setTab] = useState<Tab>("opencode");
  const [accounts, setAccounts] = useState<AccountRow[] | null>(null);
  const [cursorEnabled, setCursorEnabled] = useState(true);
  const [grokEnabled, setGrokEnabled] = useState(true);
  const [codexEnabled, setCodexEnabled] = useState(true);
  const [codexAccount, setCodexAccount] = useState<string | null>(null);
  const [codexPlan, setCodexPlan] = useState<string | null>(null);
  const [claudeEnabled, setClaudeEnabled] = useState(true);
  const [deepseekPeakHourWarning, setDeepseekPeakHourWarning] = useState(true);
  const [claudeAccount, setClaudeAccount] = useState<string | null>(null);
  const [claudeStatus, setClaudeStatus] = useState<string | null>(null);
  const [showProviderNames, setShowProviderNames] = useState(false);
  const [themeMode, setThemeMode] = useState<"auto" | "light" | "dark" | "white">("auto");
  const [panelTheme, setPanelTheme] = useState<"light" | "dark">("dark");
  const [mainTab, setMainTab] = useState<"system" | "accounts">("accounts");
  const [dragPos, setDragPos] = useState({ x: 0, y: 0 });
  const dragState = useRef<{ sx: number; sy: number; ox: number; oy: number } | null>(null);

  const onHeaderMouseDown = (e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest("button, input, select")) return;
    dragState.current = { sx: e.clientX, sy: e.clientY, ox: dragPos.x, oy: dragPos.y };
    const move = (ev: MouseEvent) => {
      const d = dragState.current;
      if (!d) return;
      setDragPos({ x: d.ox + ev.clientX - d.sx, y: d.oy + ev.clientY - d.sy });
    };
    const up = () => {
      dragState.current = null;
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };
  const [subDetail, setSubDetail] = useState<"cursor" | "grok" | "codex" | "claude" | null>(null);
  const [displayNames, setDisplayNames] = useState<Record<string, string>>({});
  const [disabledAccounts, setDisabledAccounts] = useState<string[]>([]);
  const [cursorAccount, setCursorAccount] = useState<string | null>(null);
  const [cursorPlan, setCursorPlan] = useState<string | null>(null);
  const [cursorStale, setCursorStale] = useState(false);
  const [grokAccount, setGrokAccount] = useState<string | null>(null);
  const [grokPlan, setGrokPlan] = useState<string | null>(null);
  const [codexStale, setCodexStale] = useState(false);
  const [usageByEmail, setUsageByEmail] = useState<Record<string, AccountUsage>>({});
  const [newEmail, setNewEmail] = useState("");
  const [newKey, setNewKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [detailEmail, setDetailEmail] = useState<string | null>(null);
  const [editKey, setEditKey] = useState(false);
  const [newKeyValue, setNewKeyValue] = useState("");
  const [keyBusy, setKeyBusy] = useState(false);
  const [showAdd, setShowAdd] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);
  const [savedFlash, setSavedFlash] = useState<string | null>(null);

  const refresh = async () => {
    const [u, s] = await Promise.all([
      apiFetch("/api/accounts", { cache: "no-store" }).then((r) => r.json()),
      apiFetch("/api/settings", { cache: "no-store" }).then((r) => r.json()),
    ]);
    const disabled = s.settings?.disabledAccounts ?? [];
    const list = (u.accounts ?? []).map((a: AccountRow) => ({
      email: a.email,
      keyHint: a.keyHint,
    }));
    list.sort(
      (a: AccountRow, b: AccountRow) =>
        (disabled.includes(a.email.toLowerCase()) ? 1 : 0) -
        (disabled.includes(b.email.toLowerCase()) ? 1 : 0),
    );
    setAccounts(list);
    setCursorEnabled(s.settings?.cursorEnabled !== false);
    setGrokEnabled(s.settings?.grokEnabled !== false);
    setCodexEnabled(s.settings?.codexEnabled !== false);
    setClaudeEnabled(s.settings?.claudeEnabled !== false);
    setDeepseekPeakHourWarning(s.settings?.deepseekPeakHourWarning !== false);
    setShowProviderNames(s.settings?.showProviderNames === true);
    setThemeMode(s.settings?.themeMode ?? "auto");
    setPanelTheme(s.settings?.panelTheme === "light" ? "light" : "dark");
    setDisplayNames(s.settings?.displayNames ?? {});
    setDisabledAccounts(disabled);
    apiFetch("/api/cursor", { cache: "no-store" })
      .then((r) => r.json())
      .then((c) => {
        setCursorAccount(c.usage?.accountName ?? null);
        setCursorPlan(c.usage?.planName ?? null);
        setCursorStale(c.stale === true);
        setGrokAccount(c.usage?.grokAccount ?? null);
        setGrokPlan(c.usage?.grokPlanLabel ?? null);
      })
      .catch(() => {});
    apiFetch("/api/codex", { cache: "no-store" })
      .then((r) => r.json())
      .then((c) => {
        setCodexAccount(c.usage?.email ?? null);
        setCodexPlan(c.usage?.planType ?? null);
        setCodexStale(c.stale === true);
      })
      .catch(() => {});
    apiFetch("/api/usage", { cache: "no-store" })
      .then((r) => r.json())
      .then((u) => {
        const map: Record<string, AccountUsage> = {};
        for (const row of (u.accounts ?? []) as Array<{
          email?: string;
          usage?: AccountUsage["usage"];
          stale?: boolean;
          cachedAt?: number;
          subscriptionActive?: boolean | null;
        }>) {
          if (typeof row?.email !== "string") continue;
          map[row.email] = {
            usage: row.usage ?? null,
            stale: row.stale === true,
            cachedAt: typeof row.cachedAt === "number" ? row.cachedAt : null,
            subscriptionActive: row.subscriptionActive ?? null,
          };
        }
        setUsageByEmail(map);
      })
      .catch(() => {});
    apiFetch("/api/claude", { cache: "no-store" })
      .then((r) => r.json())
      .then((c) => {
        setClaudeAccount(c.usage?.email ?? null);
        setClaudeStatus(
          c.usage == null
            ? "unavailable"
            : !c.usage.signedIn
              ? "not signed in"
              : c.usage.planName ??
                (c.usage.subscribed ? c.usage.subscriptionType ?? "subscribed" : "not subscribed"),
        );
      })
      .catch(() => {});
  };

  useEffect(() => {
    refresh();
  }, []);

  const resetBadges = (email: string) => {
    const info = usageByEmail[email] ?? usageByEmail[email.toLowerCase()];
    if (!info) return null;
    const weekly = info.usage ? resetInfo(info.usage.weekly?.resetsAt ?? "") : null;
    const monthly = info.usage ? resetInfo(info.usage.monthly?.resetsAt ?? "") : null;
    if (!weekly && !monthly && !info.stale) return null;
    const freshPill = "bg-blue-500/15 text-blue-300 border border-blue-400/30";
    const cachedNote = info.cachedAt
      ? ` · cached ${new Date(info.cachedAt).toLocaleString()}`
      : " · last known";
    const pillClass = `text-[9px] font-semibold px-1.5 py-0.5 rounded-full border shrink-0 ${
      info.stale ? CACHED_PILL : freshPill
    }`;
    return (
      <span className="flex items-center gap-1 shrink-0">
        {weekly && (
          <span title={`Weekly resets in ${weekly.full}${info.stale ? cachedNote : ""}`} className={pillClass}>
            W {weekly.short}
          </span>
        )}
        {monthly && (
          <span title={`Monthly resets in ${monthly.full}${info.stale ? cachedNote : ""}`} className={pillClass}>
            M {monthly.short}
          </span>
        )}
        {info.stale && (
          <span
            title={`Showing last known data${info.cachedAt ? ` · ${new Date(info.cachedAt).toLocaleString()}` : ""}`}
            className={`text-[9px] font-semibold px-1.5 py-0.5 rounded-full border shrink-0 ${CACHED_PILL}`}
          >
            cached
          </span>
        )}
      </span>
    );
  };

  const openDetail = (email: string) => {
    setDetailEmail(email);
    setEditKey(false);
  };

  const detailKeyHint = accounts?.find((a) => a.email === detailEmail)?.keyHint ?? null;

  const saveNewKey = async () => {
    if (!detailEmail) return;
    setKeyBusy(true);
    try {
      const res = await apiFetch("/api/accounts", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ email: detailEmail, key: newKeyValue }),
      });
      if (res.ok) {
        toastSuccess("API key updated");
        setNewKeyValue("");
        setEditKey(false);
        await refresh();
        onChanged();
      } else {
        const json = await res.json();
        toastError(json.error ?? "Update failed");
      }
    } catch {
      toastError("Update failed");
    } finally {
      setKeyBusy(false);
    }
  };

  const saveDisplayName = async (key: string, name: string) => {
    const next = { ...displayNames, [key]: name };
    if (!name.trim()) delete next[key];
    setDisplayNames(next);
    try {
      const res = await apiFetch("/api/settings", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ cursorEnabled, displayNames: next }),
      });
      if (res.ok) {
        setSavedFlash(key);
        setTimeout(() => setSavedFlash((k) => (k === key ? null : k)), 1500);
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
    const next = enabled
      ? disabledAccounts.filter((e) => e !== lower)
      : [...disabledAccounts.filter((e) => e !== lower), lower];
    setDisabledAccounts(next);
    try {
      const res = await apiFetch("/api/settings", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ cursorEnabled, displayNames, disabledAccounts: next }),
      });
      if (res.ok) {
        toastSuccess(enabled ? "Account enabled" : "Account disabled");
        onChanged();
      } else {
        toastError("Save failed");
        setDisabledAccounts(disabledAccounts);
      }
    } catch {
      toastError("Save failed");
      setDisabledAccounts(disabledAccounts);
    }
  };

  const setPanel = async (theme: "light" | "dark") => {
    setPanelTheme(theme);
    try {
      const res = await apiFetch("/api/settings", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ panelTheme: theme }),
      });
      if (res.ok) {
        toastSuccess(`Panel theme: ${theme}`);
        onChanged();
      } else {
        toastError("Save failed");
      }
    } catch {
      toastError("Save failed");
    }
  };

  const setTheme = async (mode: "auto" | "light" | "dark" | "white") => {
    setThemeMode(mode);
    try {
      const res = await apiFetch("/api/settings", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ themeMode: mode }),
      });
      if (res.ok) {
        toastSuccess(`Theme: ${mode}`);
        onChanged();
      } else {
        toastError("Save failed");
      }
    } catch {
      toastError("Save failed");
    }
  };

  const toggleProviderNames = async (enabled: boolean) => {
    setShowProviderNames(enabled);
    try {
      const res = await apiFetch("/api/settings", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ showProviderNames: enabled }),
      });
      if (res.ok) {
        toastSuccess(enabled ? "Cards show provider names" : "Cards show nicknames");
        onChanged();
      } else {
        toastError("Save failed");
        setShowProviderNames(!enabled);
      }
    } catch {
      toastError("Save failed");
      setShowProviderNames(!enabled);
    }
  };

  const toggleDeepseekPeakHourWarning = async (enabled: boolean) => {
    setDeepseekPeakHourWarning(enabled);
    try {
      const res = await apiFetch("/api/settings", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ deepseekPeakHourWarning: enabled }),
      });
      if (res.ok) {
        toastSuccess(enabled ? "DeepSeek peak-hour warning on" : "DeepSeek peak-hour warning off");
        onChanged();
      } else {
        toastError("Save failed");
        setDeepseekPeakHourWarning(!enabled);
      }
    } catch {
      toastError("Save failed");
      setDeepseekPeakHourWarning(!enabled);
    }
  };

  const toggleClaude = async (enabled: boolean) => {
    setClaudeEnabled(enabled);
    try {
      const res = await apiFetch("/api/settings", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ claudeEnabled: enabled }),
      });
      if (res.ok) {
        toastSuccess(enabled ? "Claude monitoring on" : "Claude monitoring off");
        onChanged();
      } else {
        toastError("Save failed");
        setClaudeEnabled(!enabled);
      }
    } catch {
      toastError("Save failed");
      setClaudeEnabled(!enabled);
    }
  };

  const toggleCodex = async (enabled: boolean) => {
    setCodexEnabled(enabled);
    try {
      const res = await apiFetch("/api/settings", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ codexEnabled: enabled }),
      });
      if (res.ok) {
        toastSuccess(enabled ? "Codex monitoring on" : "Codex monitoring off");
        onChanged();
      } else {
        toastError("Save failed");
        setCodexEnabled(!enabled);
      }
    } catch {
      toastError("Save failed");
      setCodexEnabled(!enabled);
    }
  };

  const toggleGrok = async (enabled: boolean) => {
    setGrokEnabled(enabled);
    try {
      const res = await apiFetch("/api/settings", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ grokEnabled: enabled }),
      });
      if (res.ok) {
        toastSuccess(enabled ? "Grok Bot monitoring on" : "Grok Bot monitoring off");
        onChanged();
      } else {
        toastError("Save failed");
        setGrokEnabled(!enabled);
      }
    } catch {
      toastError("Save failed");
      setGrokEnabled(!enabled);
    }
  };

  const addAccount = async (e: React.FormEvent) => {
    e.preventDefault();
    setBusy(true);
    try {
      const res = await apiFetch("/api/accounts", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ email: newEmail, key: newKey }),
      });
      if (res.ok) {
        toastSuccess("Account added");
        setNewEmail("");
        setNewKey("");
        await refresh();
        onChanged();
      } else {
        const json = await res.json();
        toastError(json.error ?? "Add failed");
      }
    } catch {
      toastError("Add failed");
    } finally {
      setBusy(false);
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

  const toggleCursor = async (enabled: boolean) => {
    setCursorEnabled(enabled);
    try {
      const res = await apiFetch("/api/settings", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ cursorEnabled: enabled }),
      });
      if (res.ok) {
        toastSuccess(enabled ? "Cursor monitoring on" : "Cursor monitoring off");
        onChanged();
      } else {
        toastError("Save failed");
        setCursorEnabled(!enabled);
      }
    } catch {
      toastError("Save failed");
      setCursorEnabled(!enabled);
    }
  };

  const tabClass = (t: Tab) =>
    `flex items-center gap-1.5 px-0.5 pb-2 pt-1 -mb-px text-sm border-b-2 transition-colors ${
      tab === t
        ? "border-emerald-400 text-zinc-100"
        : "border-transparent text-zinc-500 hover:text-zinc-300"
    }`;

  const mainTabClass = (t: "system" | "accounts") =>
    `px-0.5 pb-2 pt-1 -mb-px text-sm border-b-2 transition-colors ${
      mainTab === t
        ? "border-emerald-400 text-zinc-100 font-medium"
        : "border-transparent text-zinc-500 hover:text-zinc-300"
    }`;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (editKey) setEditKey(false);
      else if (subDetail) setSubDetail(null);
      else if (detailEmail) setDetailEmail(null);
      else onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [editKey, subDetail, detailEmail, onClose]);

  return (
    <div
      className={`fixed inset-0 bg-transparent flex items-center justify-center p-4 z-50 ${
        panelTheme === "light" ? "settings-light" : ""
      }`}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        className="bg-zinc-900 rounded-xl shadow-2xl w-[576px] h-[608px] flex flex-col border border-zinc-700 overflow-hidden"
        style={{ transform: `translate(${dragPos.x}px, ${dragPos.y}px)` }}
        onClick={(e) => e.stopPropagation()}
      >
        <div
          className="flex items-center justify-between px-5 pt-5 pb-2 cursor-move select-none"
          onMouseDown={onHeaderMouseDown}
        >
          <h2 className="text-lg font-bold text-zinc-100">Settings</h2>
          <button
            onClick={onClose}
            title="Close"
            className="p-1.5 rounded-md text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
              className="h-4 w-4"
            >
              <path d="M18 6 6 18" />
              <path d="m6 6 12 12" />
            </svg>
          </button>
        </div>

        <div className="flex gap-5 px-5 border-b border-zinc-800 mb-3">
          <button
            className={mainTabClass("system")}
            onClick={() => setMainTab("system")}
          >
            General
          </button>
          <button
            className={mainTabClass("accounts")}
            onClick={() => setMainTab("accounts")}
          >
            Accounts
          </button>
        </div>

        {mainTab === "system" ? (
          <div className="flex-1 overflow-y-auto no-scrollbar px-5 pb-5 space-y-2">
            <div className="flex items-center justify-between bg-zinc-800 border border-zinc-700 rounded-lg px-3 py-3">
              <div>
                <div className="text-sm font-medium text-zinc-200">Panel theme</div>
                <div className="text-xs text-zinc-500">Appearance of this settings panel</div>
              </div>
              <div className="flex rounded-lg overflow-hidden border border-zinc-700 shrink-0">
                {(["light", "dark"] as const).map((t) => (
                  <button
                    key={t}
                    onClick={() => setPanel(t)}
                    className={`px-2.5 py-1 text-xs capitalize transition-colors ${
                      panelTheme === t
                        ? "bg-zinc-100 text-zinc-900"
                        : "bg-zinc-800 text-zinc-400 hover:text-zinc-200"
                    }`}
                  >
                    {t}
                  </button>
                ))}
              </div>
            </div>

            <div className="flex items-center justify-between bg-zinc-800 border border-zinc-700 rounded-lg px-3 py-3">
              <div>
                <div className="text-sm font-medium text-zinc-200">Widget theme</div>
                <div className="text-xs text-zinc-500">
                  Auto follows screen brightness (uses CPU); Light/Dark are static
                </div>
              </div>
              <div className="flex rounded-lg overflow-hidden border border-zinc-700 shrink-0">
                {(["auto", "light", "white", "dark"] as const).map((m) => (
                  <button
                    key={m}
                    onClick={() => setTheme(m)}
                    className={`px-2.5 py-1 text-xs capitalize transition-colors ${
                      themeMode === m
                        ? "bg-zinc-100 text-zinc-900"
                        : "bg-zinc-800 text-zinc-400 hover:text-zinc-200"
                    }`}
                  >
                    {m}
                  </button>
                ))}
              </div>
            </div>

            <div className="flex items-center justify-between bg-zinc-800 border border-zinc-700 rounded-lg px-3 py-3">
              <div>
                <div className="text-sm font-medium text-zinc-200">Show provider names</div>
                <div className="text-xs text-zinc-500">
                  Display provider names on widget cards instead of nicknames
                </div>
              </div>
              <button
                onClick={() => toggleProviderNames(!showProviderNames)}
                className={`h-4 w-4 rounded-[4px] border flex items-center justify-center shrink-0 transition-colors ${showProviderNames ? "bg-emerald-500 border-emerald-400" : "bg-zinc-800 border-zinc-600"}`}
              >
                {showProviderNames && (
                    <svg
                      xmlns="http://www.w3.org/2000/svg"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="white"
                      strokeWidth="3"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      className="h-2.5 w-2.5"
                    >
                      <path d="M20 6 9 17l-5-5" />
                    </svg>
                  )}
              </button>
            </div>

            <div className="flex items-center justify-between bg-zinc-800 border border-zinc-700 rounded-lg px-3 py-3">
              <div>
                <div className="text-sm font-medium text-zinc-200">DeepSeek Peak Hour Warning</div>
                <div className="text-xs text-zinc-500">
                  Show peak-hour start/end countdowns on the OpenCode card
                </div>
              </div>
              <button
                type="button"
                aria-label="Toggle DeepSeek Peak Hour Warning"
                aria-pressed={deepseekPeakHourWarning}
                onClick={() => toggleDeepseekPeakHourWarning(!deepseekPeakHourWarning)}
                className={`h-4 w-4 rounded-[4px] border flex items-center justify-center shrink-0 transition-colors ${deepseekPeakHourWarning ? "bg-emerald-500 border-emerald-400" : "bg-zinc-800 border-zinc-600"}`}
              >
                {deepseekPeakHourWarning && (
                  <svg
                    xmlns="http://www.w3.org/2000/svg"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="white"
                    strokeWidth="3"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    className="h-2.5 w-2.5"
                  >
                    <path d="M20 6 9 17l-5-5" />
                  </svg>
                )}
              </button>
            </div>
          </div>
        ) : (
          <>
            <div className="flex gap-5 px-5 border-b border-zinc-800 mb-3">
              <button className={tabClass("opencode")} onClick={() => setTab("opencode")}>
                <img src="/opencode.ico" alt="" className="h-3.5 w-3.5 rounded-[3px]" />
                OpenCode
              </button>
              <button className={tabClass("cursor")} onClick={() => setTab("cursor")}>
                <img src="/cursor.ico" alt="" className="h-3.5 w-3.5 rounded-[3px]" />
                Cursor
              </button>
              <button className={tabClass("grok")} onClick={() => setTab("grok")}>
                <img src="/grokbot.ico" alt="" className="h-3.5 w-3.5 rounded-[3px]" />
                Grok Bot
              </button>
              <button className={tabClass("codex")} onClick={() => setTab("codex")}>
                <img src="/openai.png" alt="" className="h-3.5 w-3.5 rounded-[3px]" />
                Codex
              </button>
              <button className={tabClass("claude")} onClick={() => setTab("claude")}>
                <img src="/claude.ico" alt="" className="h-3.5 w-3.5 rounded-[3px]" />
                Claude
              </button>
            </div>

            <div className="flex-1 overflow-y-auto no-scrollbar px-5 pb-5">
        {tab === "opencode" ? (
          <div>
            <div className="flex items-center justify-between mb-3">
              <span className="text-sm font-medium text-zinc-300">Accounts</span>
              <button
                onClick={() => setShowAdd(!showAdd)}
                className={`px-3 py-1.5 rounded-lg text-sm ${
                  showAdd
                    ? "bg-zinc-700 text-zinc-200 hover:bg-zinc-600"
                    : "bg-zinc-200 text-zinc-800 hover:bg-zinc-300"
                }`}
              >
                {showAdd ? "Cancel" : "Add"}
              </button>
            </div>
            {showAdd && (
              <form onSubmit={addAccount} className="flex flex-col gap-2 mb-3">
                <input
                  value={newEmail}
                  onChange={(e) => setNewEmail(e.target.value)}
                  placeholder="email or label"
                  required
                  className="rounded-lg bg-zinc-800 border border-zinc-600 px-3 py-1 text-sm text-zinc-100 placeholder:text-zinc-500"
                />
                <input
                  type="password"
                  autoComplete="off"
                  value={newKey}
                  onChange={(e) => setNewKey(e.target.value)}
                  placeholder="sk-..."
                  required
                  className="rounded-lg bg-zinc-800 border border-zinc-600 px-3 py-1 text-sm font-mono text-zinc-100 placeholder:text-zinc-500"
                />
                <button
                  type="submit"
                  disabled={busy}
                  className="px-4 py-2 rounded-lg bg-zinc-200 hover:bg-zinc-300 text-zinc-800 text-sm disabled:opacity-50"
                >
                  {busy ? "Adding…" : "Add account"}
                </button>
              </form>
            )}
            <div className="space-y-2">
              {accounts === null ? (
                <div className="text-sm text-zinc-500">Loading…</div>
              ) : accounts.length === 0 ? (
                <div className="text-sm text-zinc-500">No accounts yet.</div>
              ) : (
                accounts.map((a) => (
                  <div
                    key={a.email}
                    className={`bg-zinc-800 border border-zinc-700 rounded-lg px-3 py-2 ${
                      disabledAccounts.includes(a.email.toLowerCase()) ? "opacity-45" : ""
                    }`}
                  >
                    <div className="flex items-center gap-2">
                      <button
                        onClick={() => openDetail(a.email)}
                        className="flex-1 min-w-0 text-left text-sm text-zinc-200 truncate"
                      >
                        {a.email}
                      </button>
                      {usageByEmail[a.email]?.subscriptionActive === false && (
                        <span className="text-[9px] font-semibold px-1.5 py-0.5 rounded-full border shrink-0 bg-red-500/15 text-red-300 border-red-400/30">
                          inactive
                        </span>
                      )}
                      {resetBadges(a.email)}
                      <div className="flex items-center gap-0.5 shrink-0">
                      <button
                        onClick={() => openDetail(a.email)}
                        title="Account details"
                        className="p-1 rounded text-zinc-400 hover:text-zinc-100 hover:bg-zinc-700 shrink-0"
                      >
                        <svg
                          xmlns="http://www.w3.org/2000/svg"
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          strokeWidth="2"
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          className="h-3.5 w-3.5"
                        >
                          <circle cx="12" cy="12" r="3" />
                          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
                        </svg>
                      </button>
                      {confirmRemove === a.email ? (
                        <>
                          <button
                            onClick={() => {
                              removeAccount(a.email);
                              setConfirmRemove(null);
                            }}
                            className="text-[10px] px-2 py-1 rounded bg-red-600 hover:bg-red-500 text-white shrink-0"
                          >
                            Remove
                          </button>
                          <button
                            onClick={() => setConfirmRemove(null)}
                            className="text-[10px] px-2 py-1 rounded text-zinc-300 hover:bg-zinc-700 shrink-0"
                          >
                            Cancel
                          </button>
                        </>
                      ) : (
                        <button
                          onClick={() => setConfirmRemove(a.email)}
                          title="Remove account"
                          className="p-1 rounded text-zinc-400 hover:text-red-300 hover:bg-zinc-700 shrink-0"
                        >
                          <svg
                            xmlns="http://www.w3.org/2000/svg"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            strokeWidth="2"
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            className="h-3.5 w-3.5"
                          >
                            <path d="M3 6h18" />
                            <path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6" />
                            <path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2" />
                          </svg>
                        </button>
                      )}
                      </div>
                    </div>
                    <div className="mt-1 text-xs text-zinc-500 truncate">
                      {displayNames[a.email] ?? "no nickname"}
                    </div>
                  </div>
                ))
              )}
            </div>
          </div>
        ) : tab === "cursor" ? (
          <div className="space-y-2">
            <div className="bg-zinc-800 border border-zinc-700 rounded-lg px-3 py-2">
              <div className="flex items-center gap-2">
<span className="flex-1 min-w-0 text-sm text-zinc-200 truncate">
                  {cursorAccount ?? "account unavailable"}
                </span>
                {cursorStale && (
                  <span className={`text-[9px] font-semibold px-1.5 py-0.5 rounded-full border shrink-0 ${CACHED_PILL}`}>
                    cached
                  </span>
                )}
                <button
                  onClick={() => setSubDetail("cursor")}
                  title="Cursor details"
                  className="p-1 rounded text-zinc-400 hover:text-zinc-100 hover:bg-zinc-700 shrink-0"
                >
                    <svg
                      xmlns="http://www.w3.org/2000/svg"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      className="h-3.5 w-3.5"
                    >
                      <circle cx="12" cy="12" r="3" />
                      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
                    </svg>
                  </button>
              </div>
<div className="mt-1 flex items-center gap-2">
                <span className="flex-1 min-w-0 text-xs text-zinc-500 truncate">
                  {displayNames["cursor"] ?? "no nickname"}
                </span>
                {cursorPlan && (
                  <span className="text-[10px] font-semibold px-1.5 py-0.5 rounded-full bg-blue-500/15 text-blue-300 border border-blue-400/30 shrink-0">
                    {cursorPlan}
                  </span>
                )}
              </div>
            </div>
          </div>
        ) : tab === "grok" ? (
          <div className="space-y-2">
            <div className="bg-zinc-800 border border-zinc-700 rounded-lg px-3 py-2">
              <div className="flex items-center gap-2">
  <span className="flex-1 min-w-0 text-sm text-zinc-200 truncate">
                  {grokAccount ?? cursorAccount ?? "account unavailable"}
                </span>
                  {cursorStale && (
                    <span className={`text-[9px] font-semibold px-1.5 py-0.5 rounded-full border shrink-0 ${CACHED_PILL}`}>
                      cached
                    </span>
                  )}
                  <button
                    onClick={() => setSubDetail("grok")}
                    title="Grok Bot details"
                    className="p-1 rounded text-zinc-400 hover:text-zinc-100 hover:bg-zinc-700 shrink-0"
                  >
                    <svg
                      xmlns="http://www.w3.org/2000/svg"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      className="h-3.5 w-3.5"
                    >
                      <circle cx="12" cy="12" r="3" />
                      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
                    </svg>
                  </button>
              </div>
<div className="mt-1 flex items-center gap-2">
                <span className="flex-1 min-w-0 text-xs text-zinc-500 truncate">
                  {displayNames["grok"] ?? "no nickname"}
                </span>
                {(grokPlan ?? cursorPlan) && (
                  <span className="text-[10px] font-semibold px-1.5 py-0.5 rounded-full bg-blue-500/15 text-blue-300 border border-blue-400/30 shrink-0">
                    {grokPlan ?? cursorPlan}
                  </span>
                )}
              </div>
            </div>
          </div>
        ) : tab === "codex" ? (
          <div className="space-y-2">
            <div className="bg-zinc-800 border border-zinc-700 rounded-lg px-3 py-2">
              <div className="flex items-center gap-2">
  <span className="flex-1 min-w-0 text-sm text-zinc-200 truncate">
                  {codexAccount ?? "account unavailable"}
                </span>
                  {codexStale && (
                    <span className={`text-[9px] font-semibold px-1.5 py-0.5 rounded-full border shrink-0 ${CACHED_PILL}`}>
                      cached
                    </span>
                  )}
                  <button
                    onClick={() => setSubDetail("codex")}
                    title="Codex details"
                    className="p-1 rounded text-zinc-400 hover:text-zinc-100 hover:bg-zinc-700 shrink-0"
                  >
                    <svg
                      xmlns="http://www.w3.org/2000/svg"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      className="h-3.5 w-3.5"
                    >
                      <circle cx="12" cy="12" r="3" />
                      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
                    </svg>
                  </button>
              </div>
              <div className="mt-1 flex items-center gap-2">
                <span className="flex-1 min-w-0 text-xs text-zinc-500 truncate">
                  {displayNames["codex"] ?? "no nickname"}
                </span>
                {codexPlan && (
                  <span
                    className={`text-[10px] font-semibold px-1.5 py-0.5 rounded-full border shrink-0 ${
                      codexPlan.toLowerCase() === "free"
                        ? "bg-emerald-500/15 text-emerald-300 border-emerald-400/30"
                        : "bg-blue-500/15 text-blue-300 border-blue-400/30"
                    }`}
                  >
                    {codexPlan.charAt(0).toUpperCase() + codexPlan.slice(1)}
                  </span>
                )}
              </div>
            </div>
          </div>
        ) : (
          <div className="space-y-2">
            <div className="bg-zinc-800 border border-zinc-700 rounded-lg px-3 py-2">
              <div className="flex items-center gap-2">
  <span className="flex-1 min-w-0 text-sm text-zinc-200 truncate">
                  {claudeAccount ?? "account unavailable"}
                </span>
                  <button
                    onClick={() => setSubDetail("claude")}
                    title="Claude details"
                    className="p-1 rounded text-zinc-400 hover:text-zinc-100 hover:bg-zinc-700 shrink-0"
                  >
                    <svg
                      xmlns="http://www.w3.org/2000/svg"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      className="h-3.5 w-3.5"
                    >
                      <circle cx="12" cy="12" r="3" />
                      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
                    </svg>
                  </button>
              </div>
              <div className="mt-1 flex items-center gap-2">
                <span className="flex-1 min-w-0 text-xs text-zinc-500 truncate">
                  {displayNames["claude"] ?? "no nickname"}
                </span>
                {claudeStatus && (
                  <span
                    className={`text-[10px] font-semibold px-1.5 py-0.5 rounded-full border shrink-0 ${
                      claudeStatus === "not subscribed"
                        ? "bg-emerald-500/15 text-emerald-300 border-emerald-400/30"
                        : claudeStatus === "unavailable" || claudeStatus === "not signed in"
                          ? "bg-zinc-500/15 text-zinc-400 border-zinc-500/30"
                          : "bg-blue-500/15 text-blue-300 border-blue-400/30"
                    }`}
                  >
                    {claudeStatus === "not subscribed" ? "Free" : claudeStatus}
                  </span>
                )}
              </div>
            </div>
          </div>
        )}
            </div>
          </>
        )}
      </div>
      {detailEmail && (
        <div
          className="fixed inset-0 bg-transparent flex items-center justify-center p-4 z-50"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) setDetailEmail(null);
          }}
        >
          <div
            className="bg-zinc-900 rounded-xl shadow-2xl w-[420px] border border-zinc-700 p-5"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between mb-4">
              <div className="min-w-0">
                <div className="flex items-center gap-1.5">
                  <h3 className="text-base font-bold text-zinc-100">OpenCode</h3>
                  <span className="text-[9px] font-semibold px-1.5 py-0.5 rounded-full bg-blue-500/15 text-blue-300 border border-blue-400/30">
                    Go
                  </span>
                  <button
                    onClick={() =>
                      toggleAccount(
                        detailEmail,
                        disabledAccounts.includes(detailEmail.toLowerCase()),
                      )
                    }
                    title={
                      disabledAccounts.includes(detailEmail.toLowerCase())
                        ? "Enable account"
                        : "Disable account"
                    }
                    className={`h-4 w-4 rounded-[4px] border flex items-center justify-center shrink-0 transition-colors ${
                      disabledAccounts.includes(detailEmail.toLowerCase())
                        ? "bg-zinc-800 border-zinc-600"
                        : "bg-emerald-500 border-emerald-400"
                    }`}
                  >
                    {!disabledAccounts.includes(detailEmail.toLowerCase()) && (
                      <svg
                        xmlns="http://www.w3.org/2000/svg"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="white"
                        strokeWidth="3"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        className="h-2.5 w-2.5"
                      >
                        <path d="M20 6 9 17l-5-5" />
                      </svg>
                    )}
                  </button>
                </div>
                <div className="text-xs text-zinc-500 truncate">{detailEmail}</div>
              </div>
              <button
                onClick={() => setDetailEmail(null)}
                title="Close"
                className="p-1.5 rounded-md text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800"
              >
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  className="h-4 w-4"
                >
                  <path d="M18 6 6 18" />
                  <path d="m6 6 12 12" />
                </svg>
              </button>
            </div>
            <div className="space-y-3">
<div className="flex items-center gap-2">
                  <div className="text-xs text-zinc-400 shrink-0">Nickname</div>
                  <input
                    defaultValue={displayNames[detailEmail] ?? ""}
                    placeholder="nickname"
                    onBlur={(e) => saveDisplayName(detailEmail, e.target.value)}
                    className="flex-1 min-w-0 rounded bg-zinc-800 border border-zinc-600 px-2 py-1 text-sm text-zinc-100 placeholder:text-zinc-500"
                  />
                  {savedFlash === detailEmail && (
                    <span className="text-[10px] text-emerald-400 shrink-0">✓ saved</span>
                  )}
                </div>
              <div className="flex items-center gap-2">
                <div className="text-xs text-zinc-400 shrink-0">API key</div>
                <div className="flex-1 min-w-0 text-xs font-mono text-zinc-300 bg-zinc-800 border border-zinc-700 rounded-lg px-2 py-1 break-all">
                  {detailKeyHint ?? "…"}
                </div>
                <button
                  onClick={() => {
                    setNewKeyValue("");
                    setEditKey(true);
                  }}
                  title="Update API key"
                  className="p-1 rounded text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 shrink-0"
                >
                  <svg
                    xmlns="http://www.w3.org/2000/svg"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    className="h-3.5 w-3.5"
                  >
                    <path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z" />
                  </svg>
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
      {editKey && detailEmail && (
        <div
          className="fixed inset-0 bg-transparent flex items-center justify-center p-4 z-[60]"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) setEditKey(false);
          }}
        >
          <div
            className="bg-zinc-900 rounded-xl shadow-2xl w-[420px] border border-zinc-700 p-5"
            onClick={(e) => e.stopPropagation()}
          >
            <h3 className="text-base font-bold text-zinc-100 mb-4">
              Update API key · {detailEmail}
            </h3>
            <div className="flex flex-col gap-3">
              <input
                type="password"
                autoComplete="off"
                value={newKeyValue}
                onChange={(e) => setNewKeyValue(e.target.value)}
                placeholder="new sk-... key"
                className="w-full rounded bg-zinc-800 border border-zinc-600 px-2 py-1 text-sm font-mono text-zinc-100 placeholder:text-zinc-500"
              />
              <div className="flex gap-2 justify-end">
                <button
                  onClick={() => setEditKey(false)}
                  className="px-3 py-1.5 rounded-lg text-sm text-zinc-300 hover:bg-zinc-800"
                >
                  Cancel
                </button>
                <button
                  onClick={saveNewKey}
                  disabled={keyBusy || !newKeyValue.trim()}
                  className="px-3 py-1.5 rounded-lg text-sm bg-zinc-200 hover:bg-zinc-300 text-zinc-800 disabled:opacity-50"
                >
                  {keyBusy ? "Saving…" : "Save"}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
{subDetail && (
        <div
          className="fixed inset-0 bg-transparent flex items-center justify-center p-4 z-[70]"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) setSubDetail(null);
          }}
        >
          <div
            className="bg-zinc-900 rounded-xl shadow-2xl w-[420px] border border-zinc-700 p-5"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between mb-4">
              <div className="min-w-0">
                <div className="flex items-center gap-1.5">
                  <h3 className="text-base font-bold text-zinc-100">
                    {subDetail === "cursor"
                      ? "Cursor"
                      : subDetail === "grok"
                        ? "Grok Bot"
                        : subDetail === "codex"
                          ? "Codex"
                          : "Claude"}
                  </h3>
                  {subDetail === "grok" && (grokPlan ?? cursorPlan) && (
                    <span className="text-[9px] font-semibold px-1.5 py-0.5 rounded-full bg-blue-500/15 text-blue-300 border border-blue-400/30">
                      {grokPlan ?? cursorPlan}
                    </span>
                  )}
                  {subDetail === "cursor" && cursorPlan && (
                    <span className="text-[9px] font-semibold px-1.5 py-0.5 rounded-full bg-blue-500/15 text-blue-300 border border-blue-400/30">
                      {cursorPlan}
                    </span>
                  )}
                  {subDetail === "codex" && codexPlan && (
                    <span
                      className={`text-[9px] font-semibold px-1.5 py-0.5 rounded-full border ${
                        codexPlan.toLowerCase() === "free"
                          ? "bg-emerald-500/15 text-emerald-300 border-emerald-400/30"
                          : "bg-blue-500/15 text-blue-300 border-blue-400/30"
                      }`}
                    >
                      {codexPlan.charAt(0).toUpperCase() + codexPlan.slice(1)}
                    </span>
                  )}
                  {subDetail === "claude" && claudeStatus && (
                    <span
                      className={`text-[9px] font-semibold px-1.5 py-0.5 rounded-full border ${
                        claudeStatus === "not subscribed"
                          ? "bg-emerald-500/15 text-emerald-300 border-emerald-400/30"
                          : claudeStatus === "unavailable" || claudeStatus === "not signed in"
                            ? "bg-zinc-500/15 text-zinc-400 border-zinc-500/30"
                            : "bg-blue-500/15 text-blue-300 border-blue-400/30"
                      }`}
                    >
                      {claudeStatus === "not subscribed" ? "Free" : claudeStatus}
                    </span>
                  )}
                  <button
                    onClick={() => {
                      if (subDetail === "cursor") toggleCursor(!cursorEnabled);
                      else if (subDetail === "grok") toggleGrok(!grokEnabled);
                      else if (subDetail === "codex") toggleCodex(!codexEnabled);
                      else toggleClaude(!claudeEnabled);
                    }}
                    title="Monitoring"
                    className={`h-4 w-4 rounded-[4px] border flex items-center justify-center shrink-0 transition-colors ${
                      (subDetail === "cursor"
                        ? cursorEnabled
                        : subDetail === "grok"
                          ? grokEnabled
                          : subDetail === "codex"
                            ? codexEnabled
                            : claudeEnabled)
                        ? "bg-emerald-500 border-emerald-400"
                        : "bg-zinc-800 border-zinc-600"
                    }`}
                  >
                    {(subDetail === "cursor"
                      ? cursorEnabled
                      : subDetail === "grok"
                        ? grokEnabled
                        : subDetail === "codex"
                          ? codexEnabled
                          : claudeEnabled) && (
                      <svg
                        xmlns="http://www.w3.org/2000/svg"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="white"
                        strokeWidth="3"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        className="h-2.5 w-2.5"
                      >
                        <path d="M20 6 9 17l-5-5" />
                      </svg>
                    )}
                  </button>
                </div>
                <div className="text-xs text-zinc-500 truncate">
                  {(subDetail === "codex"
                    ? codexAccount
                    : subDetail === "claude"
                      ? claudeAccount
                      : subDetail === "grok"
                        ? grokAccount ?? cursorAccount
                        : cursorAccount) ?? "account unavailable"}
                </div>
              </div>
              <button
                onClick={() => setSubDetail(null)}
                title="Close"
                className="p-1.5 rounded-md text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800"
              >
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  className="h-4 w-4"
                >
                  <path d="M18 6 6 18" />
                  <path d="m6 6 12 12" />
                </svg>
              </button>
            </div>
            <div className="space-y-3">
<div className="flex items-center gap-2">
                  <div className="text-xs text-zinc-400 shrink-0">Nickname</div>
                  <input
                    defaultValue={displayNames[subDetail] ?? ""}
                    placeholder="nickname"
                    onBlur={(e) => saveDisplayName(subDetail, e.target.value)}
                    className="flex-1 min-w-0 rounded bg-zinc-800 border border-zinc-600 px-2 py-1 text-sm text-zinc-100 placeholder:text-zinc-500"
                  />
                  {savedFlash === subDetail && (
                    <span className="text-[10px] text-emerald-400 shrink-0">✓ saved</span>
                  )}
                </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
