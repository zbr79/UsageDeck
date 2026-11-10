import {
  Fragment,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
} from "react";
import { CollapseIcon, ExpandIcon, EyeOffIcon, GearIcon, PowerIcon, RefreshIcon } from "@/ui/Icons";
import { cssVars, WIDGET_PALETTES } from "@/ui/palette";
import { accountKey, isAccountHidden } from "@/lib/account-key";
import { copyKey } from "@/lib/copy-key";
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
import { openCodeUsedUp, usedUp, type UsageOf } from "@/lib/used-up";
import { ClaudeCard, CodexCard, CursorCard, GrokCard, OpenCodeCard } from "./_components/cards";
import { nextBrightness, resolveWidgetTheme } from "./_lib/theme";
import { menuNavIndex } from "./_lib/menu-nav";
import { snoozedCards, snoozeSummary } from "./_lib/snooze";
import { slotFromAccount, useWidgetData, type AccountList } from "./_lib/useWidgetData";

const WIDTH = 238;
const MENU_WIDTH = 144;
/** Gap between the right-click menu and the window edge. */
const MENU_MARGIN = 4;
/** +6px below the content leaves room for the bottom card's glow. */
const GLOW = 6;
/** Longest the menu waits for the window to grow before showing anyway. */
const MENU_REVEAL_MAX_MS = 150;
/** Reset text is minute-resolution, so the whole panel does not re-render every second. */
const CLOCK_MS = 60_000;
type MenuAction = "expand" | "collapse" | "refresh" | "settings" | "quit";

function startDrag(e: MouseEvent) {
  if (e.button !== 0) return;
  if ((e.target as HTMLElement).closest("button, a, input, textarea, select")) return;
  startWindowDrag();
}

export default function WidgetPage() {
  const { data, reload, lastUsed, increased, updateSettings } = useWidgetData();
  const [expanded, setExpanded] = useState(false);
  const [brightness, setBrightness] = useState<number | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const [menuOpen, setMenuOpen] = useState(false);
  /** Where the menu was asked for (the right-click point); its top is settled once it is measured. */
  const [menuAnchor, setMenuAnchor] = useState({ x: 8, y: 8 });
  const rootRef = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  /** Window height the open menu needs (0 when closed): a one-card widget is shorter than the menu. */
  const menuBottom = useRef(0);
  const sendSize = useRef<() => void>(() => {});
  /** Opened with the Menu key or Shift+F10 (focus the first item) rather than a right-click. */
  const menuByKeyboard = useRef(false);

  const settings = data.settings ?? DEFAULT_SETTINGS;
  const theme = resolveWidgetTheme(settings.themeMode, brightness);
  const vars = useMemo(() => cssVars("w", WIDGET_PALETTES[theme]), [theme]);
  const ranked = useMemo(
    () => (data.accounts ? rankAccounts(data.accounts, lastUsed, now) : []),
    [data.accounts, lastUsed, now],
  );
  const enabled: Record<ProviderId, boolean> = {
    cursor: settings.cursorEnabled,
    grok: settings.grokEnabled,
    codex: settings.codexEnabled,
    claude: settings.claudeEnabled,
  };
  // Only the account that app is signed into. OpenCode is separate: it has no
  // sign-in, so its API-key accounts are ranked instead.
  const signedIn = <T,>(provider: ProviderId, list: AccountList<T>) =>
    enabled[provider]
      ? list.accounts.filter(
          (account) =>
            account.active && !isAccountHidden(settings.disabledAccounts, provider, account.id),
        )
      : [];

  const cardsUsedUp = <P extends ProviderId>(provider: P, list: AccountList<UsageOf[P]>) =>
    signedIn(provider, list).map((account) => ({
      provider,
      id: account.id,
      used: usedUp(provider, account, now),
    }));
  const snoozed = snoozedCards(
    [
      ...(settings.opencodeEnabled
        ? ranked.map((row) => ({
            provider: "opencode" as const,
            id: row.email,
            used: openCodeUsedUp(row, now),
          }))
        : []),
      ...cardsUsedUp("cursor", data.cursor),
      ...cardsUsedUp("grok", data.grok),
      ...cardsUsedUp("codex", data.codex),
      ...cardsUsedUp("claude", data.claude),
    ],
    settings.snoozedUntil,
    now,
  );
  const snoozedKeys = new Set(snoozed.map((card) => card.key));
  const summary = snoozeSummary(snoozed, now);
  const nextBack = snoozed.length ? Math.min(...snoozed.map((card) => card.until)) : null;
  const hide = (provider: WidgetProvider, id: string) => (until: number) =>
    void updateSettings({ snoozedUntil: { [accountKey(provider, id)]: until } }).catch(() => {});
  const showSnoozed = () =>
    void updateSettings({
      snoozedUntil: Object.fromEntries(snoozed.map((card) => [card.key, null])),
    }).catch(() => {});

  const shownRows = ranked.filter((row) => !snoozedKeys.has(accountKey("opencode", row.email)));
  const canToggleExpand = settings.opencodeEnabled && shownRows.length > 1;

  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), CLOCK_MS);
    return () => clearInterval(id);
  }, []);

  // A hidden card comes back at its reset, to the second rather than on the next minute tick.
  useEffect(() => {
    if (nextBack == null) return;
    const wait = nextBack - Date.now();
    if (wait > CLOCK_MS) return;
    const id = setTimeout(() => setNow(Date.now()), Math.max(0, wait));
    return () => clearTimeout(id);
  }, [nextBack, now]);

  // Native bridge: brightness samples, expand toggle, and context menu.
  useEffect(() => {
    const offBg = onBg((v) => setBrightness((prev) => nextBrightness(prev, v)));
    const offExpand = onToggleExpand(() => setExpanded((e) => !e));
    const onContextMenu = (e: globalThis.MouseEvent) => {
      e.preventDefault();
      const x = Math.max(MENU_MARGIN, Math.min(e.clientX, WIDTH - MENU_WIDTH - MENU_MARGIN));
      setMenuAnchor({ x, y: e.clientY });
      menuByKeyboard.current = false;
      setMenuOpen(true);
    };
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
        e.preventDefault();
        menuByKeyboard.current = true;
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
  }, []);

  // Place the open menu below the anchor when it fits, else above it; when neither fits (one
  // short card), keep it below and grow the window to hold it instead of showing scroll bars.
  useLayoutEffect(() => {
    const menu = menuRef.current;
    if (!menuOpen || !menu) {
      if (menuBottom.current === 0) return;
      menuBottom.current = 0;
      sendSize.current();
      return;
    }
    const h = menu.offsetHeight;
    const contentHeight = (rootRef.current?.offsetHeight ?? 0) + GLOW;
    const { y } = menuAnchor;
    const top =
      y + h + MENU_MARGIN <= contentHeight
        ? y
        : y - h >= MENU_MARGIN
          ? y - h
          : Math.max(MENU_MARGIN, Math.min(y, contentHeight - h - MENU_MARGIN));
    const bottom = top + h + MENU_MARGIN;
    menu.style.top = `${top}px`;
    menuBottom.current = bottom;
    sendSize.current();
    // The window grows a moment later, over IPC. Hold the menu back until it fits, so it never
    // shows cut off; opacity rather than visibility, because the menu takes focus while hidden.
    if (bottom <= window.innerHeight) {
      menu.style.opacity = "";
      return;
    }
    menu.style.opacity = "0";
    const show = () => {
      menu.style.opacity = "";
      window.removeEventListener("resize", onResize);
      clearTimeout(timer);
    };
    const onResize = () => {
      if (window.innerHeight >= bottom) show();
    };
    window.addEventListener("resize", onResize);
    // The window stops growing at the screen's height: show the menu anyway.
    const timer = setTimeout(show, MENU_REVEAL_MAX_MS);
    return show;
  }, [menuOpen, menuAnchor, canToggleExpand, expanded]);

  useEffect(() => {
    if (!menuOpen) return;
    const close = (e: globalThis.PointerEvent) => {
      if (!(e.target as HTMLElement).closest("[data-widget-menu]")) setMenuOpen(false);
    };
    document.addEventListener("pointerdown", close);
    return () => document.removeEventListener("pointerdown", close);
  }, [menuOpen]);

  // Menu focus, back to where it was on close. From the keyboard: the first item, with its ring.
  // From a right-click: the menu itself, so no item shows a focus ring (Down / Up still enter it).
  useEffect(() => {
    if (!menuOpen) return;
    const previous = document.activeElement;
    const menu = menuRef.current;
    if (menuByKeyboard.current) menu?.querySelector<HTMLElement>('[role="menuitem"]')?.focus();
    else menu?.focus();
    return () => {
      if (previous instanceof HTMLElement && previous !== document.body && previous.isConnected) {
        previous.focus();
      }
    };
  }, [menuOpen]);

  // Up / Down / Home / End move between items; Enter and Space activate the focused
  // button natively; Esc closes (window keydown above); Tab closes like a native menu.
  const onMenuKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Tab") {
      e.preventDefault();
      setMenuOpen(false);
      return;
    }
    const items = Array.from(e.currentTarget.querySelectorAll<HTMLElement>('[role="menuitem"]'));
    const next = menuNavIndex(
      e.key,
      items.indexOf(document.activeElement as HTMLElement),
      items.length,
    );
    if (next == null) return;
    e.preventDefault();
    items[next]?.focus();
  };

  // Size the frameless window to the content.
  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const send = () => {
      const h = Math.ceil(el.getBoundingClientRect().height);
      if (h > 0) resizeWidget(Math.max(h + GLOW, menuBottom.current));
    };
    sendSize.current = send;
    const ro = new ResizeObserver(send);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const visible = expanded ? shownRows : shownRows.slice(0, 1);
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
    if (!enabled[provider]) return null;
    const shown = signedIn(provider, list).filter(
      (account) => !snoozedKeys.has(accountKey(provider, account.id)),
    );
    if (shown.length === 0) return list.accounts.length === 0 ? empty : null;
    const multiple = shown.length > 1;
    return <Fragment key={provider}>{shown.map((account) => card(account, multiple))}</Fragment>;
  };

  const slotOf = slotFromAccount;
  // Spend and pace cover Claude Code on this PC, not one account: one card shows them,
  // the account Claude is signed into right now.
  const paceAccountId = data.claude.accounts.find((a) => a.active)?.id ?? null;

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
                onHide={hide("opencode", row.email)}
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
              onHide={hide("cursor", account.id)}
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
              onHide={hide("grok", account.id)}
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
              onHide={hide("codex", account.id)}
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
              onHide={hide("claude", account.id)}
              pace={account.id === paceAccountId ? data.pace : null}
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
              className="flex items-center justify-between gap-2 rounded-lg border border-(--w-ringDanger) bg-(--w-card) px-2 py-1 text-[11px] text-(--w-text)"
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
          {summary && (
            <button
              type="button"
              onClick={showSnoozed}
              aria-label={summary.label}
              title={summary.label}
              className="flex w-full items-center gap-1.5 rounded-lg border border-(--w-cardBorder) bg-(--w-card) px-2 py-1 text-left text-[11px] leading-4 text-(--w-muted) hover:bg-(--w-hover) hover:text-(--w-text) transition-colors"
            >
              <EyeOffIcon size={12} className="shrink-0" />
              <span className="truncate min-w-0">{summary.text}</span>
            </button>
          )}
        </div>
      )}
      {menuOpen && (
        <div
          ref={menuRef}
          data-widget-menu
          role="menu"
          aria-label="Widget menu"
          tabIndex={-1}
          // inline: the global :focus-visible ring is unlayered and beats a utility class
          style={{ left: menuAnchor.x, top: menuAnchor.y, width: MENU_WIDTH, outline: "none" }}
          className="absolute z-50 rounded-lg border border-(--w-cardBorder) bg-(--w-card) p-1 shadow-xl text-(--w-text)"
          onContextMenu={(e) => e.preventDefault()}
          onKeyDown={onMenuKeyDown}
        >
          {canToggleExpand && (
            <button
              type="button"
              role="menuitem"
              tabIndex={-1}
              onClick={() => menuAction(expanded ? "collapse" : "expand")}
              className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[12px] hover:bg-(--w-hover) focus-visible:bg-(--w-hover)"
            >
              {expanded ? <CollapseIcon size={14} /> : <ExpandIcon size={14} />}
              {expanded ? "Collapse" : "Expand"}
            </button>
          )}
          <button
            type="button"
            role="menuitem"
            tabIndex={-1}
            onClick={() => menuAction("refresh")}
            className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[12px] hover:bg-(--w-hover) focus-visible:bg-(--w-hover)"
          >
            <RefreshIcon size={14} />
            Refresh
          </button>
          <button
            type="button"
            role="menuitem"
            tabIndex={-1}
            onClick={() => menuAction("settings")}
            className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[12px] hover:bg-(--w-hover) focus-visible:bg-(--w-hover)"
          >
            <GearIcon size={14} />
            Settings
          </button>
          <div role="separator" className="my-1 border-t border-(--w-cardBorder)" />
          <button
            type="button"
            role="menuitem"
            tabIndex={-1}
            onClick={() => menuAction("quit")}
            className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[12px] text-(--w-barDanger) hover:bg-(--w-hover) focus-visible:bg-(--w-hover)"
          >
            <PowerIcon size={14} />
            Quit
          </button>
        </div>
      )}
    </main>
  );
}
