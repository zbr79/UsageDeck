"use client";

import { useEffect, useState, type CSSProperties } from "react";
import { toastError, toastSuccess } from "@/app/components/toast/toast";
import { apiFetch } from "@/lib/api-fetch";
import { usageWidget } from "@/lib/desktop-bridge";
import {
  resolveWidgetThemeName,
  themeStyleVars,
  WIDGET_THEMES,
  WIDGET_TYPE_SCALE,
} from "@/lib/theme";
import ClaudeCard from "./ClaudeCard";
import CodexCard from "./CodexCard";
import CursorCard from "./CursorCard";
import GrokCard from "./GrokCard";
import OpenCodeCard from "./OpenCodeCard";
import type { AccountRow } from "./types";
import { useWidgetData } from "./useWidgetData";

export default function Widget() {
  const {
    rows,
    sortedRows,
    activeRow,
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
    reload,
  } = useWidgetData();

  const [expanded, setExpanded] = useState(false);
  const [bgBright, setBgBright] = useState<number | null>(null);

  useEffect(() => {
    const off = usageWidget()?.onBg?.((v: number) => {
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
    const off = usageWidget()?.onToggleExpand?.(() => setExpanded((e) => !e));
    return () => off?.();
  }, []);

  useEffect(() => {
    const onCtx = (e: MouseEvent) => {
      e.preventDefault();
      usageWidget()?.showMenu?.();
    };
    document.addEventListener("contextmenu", onCtx);
    return () => document.removeEventListener("contextmenu", onCtx);
  }, []);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (target && ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName)) return;

      if ((e.ctrlKey || e.metaKey) && e.key === ",") {
        e.preventDefault();
        usageWidget()?.openSettings?.();
        return;
      }
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "q") {
        e.preventDefault();
        usageWidget()?.close?.();
        return;
      }
      if ((e.shiftKey && e.key === "F10") || e.key === "ContextMenu") {
        e.preventDefault();
        usageWidget()?.showMenu?.();
        return;
      }
      if (!e.ctrlKey && !e.metaKey && !e.altKey) {
        if (e.key.toLowerCase() === "e") {
          e.preventDefault();
          setExpanded((v) => !v);
        } else if (e.key.toLowerCase() === "r") {
          e.preventDefault();
          void reload();
        }
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [reload]);

  const onDragStart = (e: React.MouseEvent) => {
    if (e.button !== 0) return;
    const widget = usageWidget();
    widget?.startDrag?.(e.screenX, e.screenY);
    const move = (ev: MouseEvent) => widget?.moveDrag?.(ev.screenX, ev.screenY);
    const up = () => {
      widget?.endDrag?.();
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  const widgetThemeName = resolveWidgetThemeName(themeMode, bgBright);
  const light = widgetThemeName !== "dark";
  const themeVars = themeStyleVars(WIDGET_THEMES[widgetThemeName]) as CSSProperties;

  useEffect(() => {
    const t = setTimeout(() => {
      const h = document.body?.scrollHeight ?? 0;
      if (h > 0) usageWidget()?.resize?.(238, h + 6);
    }, 80);
    return () => clearTimeout(t);
  }, [expanded, rows, cursor]);

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

  const showOffline = failedPolls >= 2 || serverState !== "online";
  const offlineLabel =
    serverState === "stopped"
      ? "Server stopped · right-click to refresh"
      : "Offline · retrying";

  const renderOpenCode = (row: AccountRow, withControls: boolean) => (
    <OpenCodeCard
      key={row.email}
      row={row}
      isInUse={row.enabled !== false && sortedRows?.[0]?.email === row.email}
      withControls={withControls}
      light={light}
      showProviderNames={showProviderNames}
      displayNames={displayNames}
      deepseekPeakHourWarning={deepseekPeakHourWarning}
      increaseAt={increaseAt}
      onDragStart={onDragStart}
      onCopyKey={copyKey}
    />
  );

  const renderProviderCards = () => (
    <>
      <CursorCard
        cursor={cursor}
        cursorError={cursorError}
        cursorEnabled={cursorEnabled}
        light={light}
        showProviderNames={showProviderNames}
        displayNames={displayNames}
        onDragStart={onDragStart}
      />
      <GrokCard
        cursor={cursor}
        grokEnabled={grokEnabled}
        light={light}
        showProviderNames={showProviderNames}
        displayNames={displayNames}
        onDragStart={onDragStart}
      />
      <CodexCard
        codex={codex}
        codexEnabled={codexEnabled}
        light={light}
        showProviderNames={showProviderNames}
        displayNames={displayNames}
        onDragStart={onDragStart}
      />
      <ClaudeCard
        claude={claude}
        claudeEnabled={claudeEnabled}
        light={light}
        showProviderNames={showProviderNames}
        displayNames={displayNames}
        onDragStart={onDragStart}
      />
    </>
  );

  return (
    <div
      role="region"
      aria-label="Provider usage monitor"
      className="w-screen p-1.5 select-none"
      style={themeVars}
    >
      <div className={expanded ? "space-y-1.5" : ""}>
        {showOffline && (
          <div
            role="status"
            aria-live="polite"
            className={`rounded-lg px-2 py-1 ${WIDGET_TYPE_SCALE.body} font-medium mb-1.5 bg-role-surface text-role-text border border-role-border`}
          >
            {offlineLabel}
          </div>
        )}
        {rows === null ? (
          <div className={`${WIDGET_TYPE_SCALE.title} px-1 py-2 text-role-muted`}>
            Loading…
          </div>
        ) : expanded ? (
          <>
            {sortedRows?.map((row, i) => renderOpenCode(row, i === 0))}
            {renderProviderCards()}
          </>
        ) : (
          <>
            {activeRow ? renderOpenCode(activeRow, true) : null}
            {renderProviderCards()}
          </>
        )}
      </div>
    </div>
  );
}
