"use client";

import { useEffect, useMemo, useRef, useState, type MouseEvent } from "react";
import { cssVars, WIDGET_PALETTES } from "@/app/_ui/palette";
import { toastError, toastSuccess } from "@/app/components/toast/toast";
import { getJson } from "@/lib/api-fetch";
import { formatAgo } from "@/lib/format";
import { getPeakInfo } from "@/lib/peak";
import { rankAccounts } from "@/lib/ranking";
import { DEFAULT_SETTINGS } from "@/lib/settings-schema";
import { ClaudeCard, CodexCard, CursorCard, GrokCard, OpenCodeCard } from "./_components/cards";
import { nextBrightness, resolveWidgetTheme } from "./_lib/theme";
import { useWidgetData } from "./_lib/useWidgetData";

const WIDTH = 238;
/** Reset text is minute-resolution, so the whole panel does not re-render every second. */
const CLOCK_MS = 60_000;

async function copyKey(email: string) {
  try {
    const { key } = await getJson<{ key: string }>(`/api/key?email=${encodeURIComponent(email)}`);
    await navigator.clipboard.writeText(key);
    toastSuccess("Key copied");
  } catch {
    toastError("Copy failed");
  }
}

function startDrag(e: MouseEvent) {
  if (e.button !== 0) return;
  if ((e.target as HTMLElement).closest("button, a, input, textarea, select")) return;
  const bridge = window.widget;
  if (!bridge) return;
  bridge.startDrag(e.screenX, e.screenY);
  const move = (ev: globalThis.MouseEvent) => bridge.moveDrag(ev.screenX, ev.screenY);
  const up = () => {
    bridge.endDrag();
    window.removeEventListener("mousemove", move);
    window.removeEventListener("mouseup", up);
  };
  window.addEventListener("mousemove", move);
  window.addEventListener("mouseup", up);
}

export default function WidgetPage() {
  const { data, reload, lastUsed, increased } = useWidgetData();
  const [expanded, setExpanded] = useState(false);
  const [brightness, setBrightness] = useState<number | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const rootRef = useRef<HTMLDivElement>(null);

  const settings = data.settings ?? DEFAULT_SETTINGS;
  const theme = resolveWidgetTheme(settings.themeMode, brightness);
  const vars = useMemo(() => cssVars("w", WIDGET_PALETTES[theme]), [theme]);

  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), CLOCK_MS);
    return () => clearInterval(id);
  }, []);

  // Electron bridge: brightness samples, expand toggle, context menu.
  useEffect(() => {
    const w = window.widget;
    const offBg = w?.onBg((v) => setBrightness((prev) => nextBrightness(prev, v)));
    const offExpand = w?.onToggleExpand(() => setExpanded((e) => !e));
    const onContextMenu = (e: globalThis.MouseEvent) => {
      e.preventDefault();
      w?.showMenu();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
        e.preventDefault();
        w?.showMenu();
      }
    };
    document.addEventListener("contextmenu", onContextMenu);
    window.addEventListener("keydown", onKey);
    return () => {
      offBg?.();
      offExpand?.();
      document.removeEventListener("contextmenu", onContextMenu);
      window.removeEventListener("keydown", onKey);
    };
  }, []);

  // Size the frameless window to the content.
  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const send = () => {
      const h = Math.ceil(el.getBoundingClientRect().height);
      // +6px leaves room for the bottom card's glow.
      if (h > 0) window.widget?.resize(WIDTH, h + 6);
    };
    const ro = new ResizeObserver(send);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const ranked = useMemo(
    () => (data.accounts ? rankAccounts(data.accounts, lastUsed, now) : []),
    [data.accounts, lastUsed, now],
  );
  const visible = expanded ? ranked : ranked.slice(0, 1);
  const peak = useMemo(() => getPeakInfo(new Date(now)), [now]);
  const light = theme !== "dark";

  return (
    <main
      ref={rootRef}
      aria-label="Provider usage"
      data-theme={theme}
      style={vars}
      className="w-screen p-1.5 select-none cursor-move"
      onMouseDown={startDrag}
    >
      {!data.ready ? (
        <p className="text-[11px] px-1 py-2 text-(--w-muted)">Loading…</p>
      ) : (
        <div className="space-y-1.5">
          {data.offline && (
            <div
              role="status"
              className="flex items-center justify-between gap-2 rounded-lg border border-(--w-ringDanger) bg-(--w-card) px-2 py-1 text-[10px] text-(--w-text)"
            >
              <span>
                Server unreachable
                {data.updatedAt ? ` · updated ${formatAgo(data.updatedAt, now)}` : ""}
              </span>
              <button
                type="button"
                onClick={() => void reload()}
                className="font-semibold underline underline-offset-2 hover:opacity-80"
              >
                Retry
              </button>
            </div>
          )}
          {visible.map((row, i) => (
            <OpenCodeCard
              key={row.email}
              row={row}
              active={i === 0}
              primary={i === 0}
              settings={settings}
              peak={peak}
              now={now}
              increased={(win) => increased(row.email, win, now)}
              onCopyKey={() => void copyKey(row.email)}
            />
          ))}
          {settings.cursorEnabled && (
            <CursorCard slot={data.cursor} settings={settings} now={now} />
          )}
          {settings.grokEnabled && <GrokCard slot={data.grok} settings={settings} now={now} />}
          {settings.codexEnabled && (
            <CodexCard slot={data.codex} settings={settings} now={now} light={light} />
          )}
          {settings.claudeEnabled && (
            <ClaudeCard slot={data.claude} settings={settings} now={now} />
          )}
        </div>
      )}
    </main>
  );
}
