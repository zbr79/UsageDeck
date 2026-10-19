import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { closeSettings, inTauri } from "@/lib/bridge";
import { cssVars, PANEL_PALETTES, SETTINGS_CARD_PALETTES, SETTINGS_ROW_CARD } from "@/ui/palette";
import { useSettings, useSummaries } from "./_lib/hooks";
import { PanelThemeContext, resolvePanelTheme, useSystemDark } from "./_lib/theme";
import {
  isProviderPage,
  pageFromSearch,
  PAGE_TITLES,
  searchWithPage,
  type PageId,
} from "./_lib/pages";
import { Nav } from "./_components/Nav";
import OpenCodeAccounts from "./_components/OpenCodeAccounts";
import ProviderPanel from "./_components/ProviderPanel";
import { AboutPage, AppearancePage, GeneralPage, WidgetPage } from "./_components/SettingsPages";

/** Settings-only surface colours (WinUI: cards sit one step off the page). */
const SURFACES = {
  dark: {
    "--s-page": "#202020",
    "--s-card": SETTINGS_ROW_CARD.dark,
    "--s-cardBorder": "#1d1d1d",
    "--s-navHover": "rgba(255,255,255,0.06)",
    "--s-navActive": "rgba(255,255,255,0.09)",
  },
  light: {
    "--s-page": "#f3f3f3",
    "--s-card": SETTINGS_ROW_CARD.light,
    "--s-cardBorder": "#e5e5e5",
    "--s-navHover": "rgba(0,0,0,0.04)",
    "--s-navActive": "rgba(0,0,0,0.06)",
  },
} as const;

/** A menu or dialog is open: Esc belongs to it, not to the window. */
function overlayOpen(): boolean {
  return document.querySelector("dialog[open], [role='menu']") !== null;
}

export default function SettingsPage() {
  const { settings, update } = useSettings();
  const summaries = useSummaries();
  const [page, setPage] = useState<PageId>(() => pageFromSearch(window.location.search));
  const main = useRef<HTMLElement>(null);
  const systemDark = useSystemDark();
  const followSystem = settings?.panelTheme === "system";
  const theme = resolvePanelTheme(settings?.panelTheme, systemDark);
  // The account cards are the widget's own, so they also need its colours, in the panel's theme.
  const vars = useMemo(
    () =>
      ({
        ...cssVars("p", PANEL_PALETTES[theme]),
        ...cssVars("w", SETTINGS_CARD_PALETTES[theme]),
        ...SURFACES[theme],
        colorScheme: theme,
        fontFamily: '"Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif',
      }) as CSSProperties,
    [theme],
  );

  // The native caption follows the theme; "Use system setting" un-forces it, so the caption and
  // `prefers-color-scheme` both track Windows live.
  const loaded = settings !== null;
  useEffect(() => {
    if (!inTauri() || !loaded) return;
    getCurrentWindow()
      .setTheme(followSystem ? null : theme)
      .catch((err) => console.error("setTheme failed", err));
  }, [theme, followSystem, loaded]);

  // Esc (when no menu or dialog is open) and Ctrl+W close the window, like Alt+F4 and the X.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.defaultPrevented && !overlayOpen()) {
        closeSettings("escape key");
      } else if (e.ctrlKey && !e.altKey && e.key.toLowerCase() === "w") {
        e.preventDefault();
        closeSettings("ctrl+w");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const select = (next: PageId) => {
    setPage(next);
    history.replaceState(null, "", searchWithPage(window.location.search, next));
    main.current?.scrollTo({ top: 0 });
  };

  useEffect(() => {
    document.title = `${PAGE_TITLES[page]} - UsageDeck Settings`;
  }, [page]);

  let content;
  if (!settings) {
    content = <p className="p-8 text-sm text-(--p-muted)">Loading…</p>;
  } else if (page === "general") {
    content = <GeneralPage settings={settings} update={update} />;
  } else if (page === "appearance") {
    content = <AppearancePage settings={settings} update={update} />;
  } else if (page === "widget") {
    content = <WidgetPage settings={settings} update={update} />;
  } else if (page === "about") {
    content = <AboutPage />;
  } else if (page === "opencode") {
    content = (
      <OpenCodeAccounts
        accounts={summaries.accounts}
        usage={summaries.usage}
        settings={settings}
        update={update}
        reload={summaries.reloadAccounts}
        reloadAll={summaries.reloadAll}
      />
    );
  } else if (isProviderPage(page)) {
    content = <ProviderPanel id={page} summaries={summaries} settings={settings} update={update} />;
  }

  return (
    <PanelThemeContext.Provider value={theme}>
      <div
        style={vars}
        data-menu-root
        className="settings-root fixed inset-0 flex bg-(--s-page) text-(--p-text) antialiased"
      >
        <Nav
          current={page}
          onSelect={select}
          providerOrder={
            settings?.providerOrder ?? ["claude", "codex", "cursor", "grok", "opencode"]
          }
          light={theme === "light"}
        />
        <main
          ref={main}
          aria-labelledby="page-title"
          className="min-w-0 flex-1 overflow-y-auto bg-(--s-page)"
        >
          {content}
        </main>
      </div>
    </PanelThemeContext.Provider>
  );
}
