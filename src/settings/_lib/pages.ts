import type { WidgetProvider } from "@/lib/types";

/** The settings window's pages: General, Widget, one per provider, then About. */
export type PageId = "general" | "widget" | WidgetProvider | "about";

export const PROVIDER_PAGES: readonly WidgetProvider[] = [
  "claude",
  "codex",
  "cursor",
  "grok",
  "opencode",
];

const FIXED_PAGES = ["general", "widget", "about"] as const;

export const PAGE_TITLES: Record<PageId, string> = {
  general: "General",
  widget: "Widget",
  about: "About",
  claude: "Claude",
  codex: "Codex",
  cursor: "Cursor",
  grok: "Grok Bot",
  opencode: "OpenCode",
};

export const PROVIDER_ICONS: Record<WidgetProvider, string> = {
  claude: "/claude.ico",
  codex: "/openai.png",
  cursor: "/cursor.ico",
  grok: "/grokbot.ico",
  opencode: "/opencode.ico",
};

export function isPageId(value: string | null): value is PageId {
  return (
    value !== null &&
    ((FIXED_PAGES as readonly string[]).includes(value) ||
      (PROVIDER_PAGES as readonly string[]).includes(value))
  );
}

export function isProviderPage(page: PageId): page is WidgetProvider {
  return (PROVIDER_PAGES as readonly string[]).includes(page);
}

/**
 * The page a URL opens: `settings.html?page=<id>` (for screenshots and deep links). The older
 * `?tab=<id>` still works; anything unknown opens General.
 */
export function pageFromSearch(search: string): PageId {
  const params = new URLSearchParams(search);
  const page = params.get("page") ?? params.get("tab");
  return isPageId(page) ? page : "general";
}

/** The same search string with `page` set (other parameters such as `panel` are kept). */
export function searchWithPage(search: string, page: PageId): string {
  const params = new URLSearchParams(search);
  params.delete("tab");
  params.set("page", page);
  return `?${params.toString()}`;
}

/** Move one item of a list to another index (provider order). */
export function moveItem<T>(list: readonly T[], from: number, to: number): T[] {
  const next = [...list];
  if (from < 0 || from >= next.length || to < 0 || to >= next.length) return next;
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item);
  return next;
}
