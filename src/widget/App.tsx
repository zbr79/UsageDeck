import {
  Fragment,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent,
  type ReactNode,
} from "react";
import { CollapseIcon, ExpandIcon, GearIcon, PowerIcon, RefreshIcon } from "@/ui/Icons";
import { cssVars, WIDGET_PALETTES } from "@/ui/palette";
import { toastError, toastSuccess } from "@/components/toast/toast";
import { isAccountHidden } from "@/lib/account-key";
import { getJson } from "@/lib/api";
import {
  menuAction as bridgeMenuAction,
  onBg,
  onToggleExpand,
  resizeWidget,
  startWindowDrag,
} from "@/lib/bridge";
import { formatAgo } from "@/lib/format";
import { getPeakInfo } from "@/lib/peak";
import { rankAccounts } from "@/lib/ranking";
import { DEFAULT_SETTINGS } from "@/lib/settings-schema";
import type { ProviderId, TrackedAccount, WidgetProvider } from "@/lib/types";
import { ClaudeCard, CodexCard, CursorCard, GrokCard, OpenCodeCard } from "./_components/cards";
import { nextBrightness, resolveWidgetTheme } from "./_lib/theme";
import { useWidgetData, type AccountList } from "./_lib/useWidgetData";

const WIDTH = 238;
const MENU_WIDTH = 120;
const MENU_HEIGHT = 132;
const COMPACT_MENU_HEIGHT = 100;
/** Reset text is minute-resolution, so the whole panel does not re-render every second. */
const CLOCK_MS = 60_000;
type MenuAction = "expand" | "collapse" | "refresh" | "settings" | "quit";

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
  startWindowDrag();
}

export default function WidgetPage() {
  const { data, reload, lastUsed, increased } = useWidgetData();
  const [expanded, setExpanded] = useState(false);
  const [brightness, setBrightness] = useState<number | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const [menuOpen, setMenuOpen] = useState(false);
  const [menuPosition, setMenuPosition] = useState({ x: 8, y: 8 });
  const rootRef = useRef<HTMLDivElement>(null);

  const settings = data.settings ?? DEFAULT_SETTINGS;
  const theme = resolveWidgetTheme(settings.themeMode, brightness);
  const vars = useMemo(() => cssVars("w", WIDGET_PALETTES[theme]), [theme]);
  const ranked = useMemo(
    () => (data.accounts ? rankAccounts(data.accounts, lastUsed, now) : []),
    [data.accounts, lastUsed, now],
  );
  const canToggleExpand = settings.opencodeEnabled && ranked.length > 1;

  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), CLOCK_MS);
    return () => clearInterval(id);
  }, []);

  // Native bridge: brightness samples, expand toggle, and context menu.
  useEffect(() => {
    const offBg = onBg((v) => setBrightness((prev) => nextBrightness(prev, v)));
    const offExpand = onToggleExpand(() => setExpanded((e) => !e));
    const onContextMenu = (e: globalThis.MouseEvent) => {
      e.preventDefault();
      const x = Math.max(4, Math.min(e.clientX, WIDTH - MENU_WIDTH - 4));
      const menuHeight = canToggleExpand ? MENU_HEIGHT : COMPACT_MENU_HEIGHT;
      const y =
        e.clientY + menuHeight <= window.innerHeight - 4
          ? e.clientY
          : Math.max(4, e.clientY - menuHeight);
      setMenuPosition({ x, y });
      setMenuOpen(true);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
        e.preventDefault();
        setMenuOpen(true);
      } else if (e.key === "Escape") {
        setMenuOpen(false);
      }
    };
    document.addEventListener("contextmenu", onContextMenu);
    window.addEventListener("keydown", onKey);
    return () => {
      offBg();
      offExpand();
      document.removeEventListener("contextmenu", onContextMenu);
      window.removeEventListener("keydown", onKey);
    };
  }, [canToggleExpand]);

  useEffect(() => {
    if (!menuOpen) return;
    const close = (e: globalThis.PointerEvent) => {
      if (!(e.target as HTMLElement).closest("[data-widget-menu]")) setMenuOpen(false);
    };
    document.addEventListener("pointerdown", close);
    return () => document.removeEventListener("pointerdown", close);
  }, [menuOpen]);

  // Size the frameless window to the content.
  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const send = () => {
      const h = Math.ceil(el.getBoundingClientRect().height);
      // +6px leaves room for the bottom card's glow.
      if (h > 0) resizeWidget(h + 6);
    };
    const ro = new ResizeObserver(send);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const visible = expanded ? ranked : ranked.slice(0, 1);
  const peak = useMemo(() => getPeakInfo(new Date(now)), [now]);
  const light = theme !== "dark";
  const menuAction = (action: MenuAction) => {
    setMenuOpen(false);
    if (action === "expand") setExpanded(true);
    else if (action === "collapse") setExpanded(false);
    else if (action === "refresh") void reload();
    else bridgeMenuAction(action);
  };

  const renderAccounts = <T,>(
    provider: ProviderId,
    list: AccountList<T>,
    card: (account: TrackedAccount<T>, multiple: boolean) => ReactNode,
    empty: ReactNode,
  ) => {
    const enabled =
      provider === "cursor"
        ? settings.cursorEnabled
        : provider === "grok"
          ? settings.grokEnabled
          : provider === "codex"
            ? settings.codexEnabled
            : settings.claudeEnabled;
    if (!enabled) return null;
    // Only the account that app is signed into. OpenCode is separate: it has no
    // sign-in, so its API-key accounts are rendered above.
    const shown = list.accounts.filter(
      (account) =>
        account.active && !isAccountHidden(settings.disabledAccounts, provider, account.id),
    );
    if (shown.length === 0) return list.accounts.length === 0 ? empty : null;
    const multiple = shown.length > 1;
    return <Fragment key={provider}>{shown.map((account) => card(account, multiple))}</Fragment>;
  };

  const slotOf = <T,>(account: TrackedAccount<T> | null, loaded: boolean) => ({
    data: account?.usage ?? null,
    stale: account?.stale ?? false,
    cachedAt: account?.cachedAt ?? null,
    authExpired: account?.authExpired ?? false,
    loaded,
  });

  const renderProvider = (id: WidgetProvider) => {
    switch (id) {
      case "opencode":
        if (!settings.opencodeEnabled) return null;
        return (
          <Fragment key="opencode">
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
          </Fragment>
        );
      case "cursor":
        return renderAccounts(
          "cursor",
          data.cursor,
          (account, multiple) => (
            <CursorCard
              key={account.id}
              slot={slotOf(account, true)}
              settings={settings}
              now={now}
              accountId={account.id}
              accountLabel={account.label ?? account.usage?.accountName}
              multiple={multiple}
            />
          ),
          <CursorCard
            key="cursor-empty"
            slot={slotOf(null, data.cursor.loaded)}
            settings={settings}
            now={now}
          />,
        );
      case "grok":
        return renderAccounts(
          "grok",
          data.grok,
          (account, multiple) => (
            <GrokCard
              key={account.id}
              slot={slotOf(account, true)}
              settings={settings}
              now={now}
              accountId={account.id}
              accountLabel={account.label ?? account.usage?.email}
              multiple={multiple}
            />
          ),
          <GrokCard
            key="grok-empty"
            slot={slotOf(null, data.grok.loaded)}
            settings={settings}
            now={now}
          />,
        );
      case "codex":
        return renderAccounts(
          "codex",
          data.codex,
          (account, multiple) => (
            <CodexCard
              key={account.id}
              slot={slotOf(account, true)}
              settings={settings}
              now={now}
              light={light}
              accountId={account.id}
              accountLabel={account.label ?? account.usage?.email}
              multiple={multiple}
            />
          ),
          <CodexCard
            key="codex-empty"
            slot={slotOf(null, data.codex.loaded)}
            settings={settings}
            now={now}
            light={light}
          />,
        );
      case "claude":
        return renderAccounts(
          "claude",
          data.claude,
          (account, multiple) => (
            <ClaudeCard
              key={account.id}
              slot={slotOf(account, true)}
              settings={settings}
              now={now}
              accountId={account.id}
              accountLabel={account.label ?? account.usage?.email}
              multiple={multiple}
            />
          ),
          <ClaudeCard
            key="claude-empty"
            slot={slotOf(null, data.claude.loaded)}
            settings={settings}
            now={now}
          />,
        );
    }
  };

  return (
    <main
      ref={rootRef}
      aria-label="Provider usage"
      data-theme={theme}
      style={vars}
      className="relative w-screen p-1.5 select-none cursor-move"
      onMouseDown={startDrag}
    >
      {!data.ready ? (
        <p className="text-[11px] px-1 py-2 text-(--w-muted)">Loading…</p>
      ) : (
        <div className="space-y-1">
          {data.offline && (
            <div
              role="status"
              className="flex items-center justify-between gap-2 rounded-lg border border-(--w-ringDanger) bg-(--w-card) px-2 py-1 text-[10px] text-(--w-text)"
            >
              <span>
                Usage unavailable
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
          {settings.providerOrder.map((id) => renderProvider(id))}
        </div>
      )}
      {menuOpen && (
        <div
          data-widget-menu
          role="menu"
          aria-label="Widget menu"
          style={{
            left: menuPosition.x,
            top: menuPosition.y,
            width: MENU_WIDTH,
          }}
          className="absolute z-50 rounded-lg border border-(--w-border) bg-(--w-card) p-1 shadow-xl text-(--w-text)"
          onContextMenu={(e) => e.preventDefault()}
        >
          {canToggleExpand && (
            <button
              type="button"
              role="menuitem"
              onClick={() => menuAction(expanded ? "collapse" : "expand")}
              className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[11px] hover:bg-(--w-hover)"
            >
              {expanded ? <CollapseIcon size={14} /> : <ExpandIcon size={14} />}
              {expanded ? "Collapse" : "Expand"}
            </button>
          )}
          <button
            type="button"
            role="menuitem"
            onClick={() => menuAction("refresh")}
            className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[11px] hover:bg-(--w-hover)"
          >
            <RefreshIcon size={14} />
            Refresh
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => menuAction("settings")}
            className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[11px] hover:bg-(--w-hover)"
          >
            <GearIcon size={14} />
            Settings
          </button>
          <div className="my-1 border-t border-(--w-border)" />
          <button
            type="button"
            role="menuitem"
            onClick={() => menuAction("quit")}
            className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[11px] text-(--w-barDanger) hover:bg-(--w-hover)"
          >
            <PowerIcon size={14} />
            Quit
          </button>
        </div>
      )}
    </main>
  );
}
