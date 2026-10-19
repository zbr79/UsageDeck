import { createContext, useContext, useSyncExternalStore } from "react";
import type { PanelTheme } from "@/lib/types";

/** The theme the settings window actually paints in. */
export type ResolvedTheme = "light" | "dark";

/** "Use system setting" takes the Windows app mode; a missing setting is dark, the default. */
export function resolvePanelTheme(
  theme: PanelTheme | undefined,
  systemDark: boolean,
): ResolvedTheme {
  if (theme === "system") return systemDark ? "dark" : "light";
  return theme ?? "dark";
}

const DARK_QUERY = "(prefers-color-scheme: dark)";

function subscribe(onChange: () => void): () => void {
  if (typeof matchMedia !== "function") return () => {};
  const query = matchMedia(DARK_QUERY);
  query.addEventListener("change", onChange);
  return () => query.removeEventListener("change", onChange);
}

function systemDark(): boolean {
  return typeof matchMedia === "function" ? matchMedia(DARK_QUERY).matches : true;
}

/** Windows light / dark mode, live: WebView2 reports it through `prefers-color-scheme`. */
export function useSystemDark(): boolean {
  return useSyncExternalStore(subscribe, systemDark);
}

/** The resolved theme, provided by the settings root. */
export const PanelThemeContext = createContext<ResolvedTheme>("dark");

export function usePanelTheme(): ResolvedTheme {
  return useContext(PanelThemeContext);
}
