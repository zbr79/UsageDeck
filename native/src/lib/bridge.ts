// Native window bridge (replaces the Electron preload). Every call is a no-op
// when the page runs in a normal browser (npm run dev without Tauri).
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type Unsubscribe = () => void;

export function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Subscribe to a Rust event; the returned function unsubscribes (even before `listen` resolves). */
function on<T>(event: string, cb: (payload: T) => void): Unsubscribe {
  if (!inTauri()) return () => {};
  let off: Unsubscribe | null = null;
  let closed = false;
  listen<T>(event, (e) => cb(e.payload)).then(
    (unlisten) => {
      if (closed) unlisten();
      else off = unlisten;
    },
    (err) => console.error(`listen ${event} failed`, err),
  );
  return () => {
    closed = true;
    off?.();
  };
}

function call(cmd: string, args?: Record<string, unknown>): void {
  if (!inTauri()) return;
  invoke(cmd, args).catch((err) => console.error(`${cmd} failed`, err));
}

/** Size the frameless widget to its content (width is fixed on the Rust side). */
export function resizeWidget(height: number): void {
  call("widget_resize", { height });
}

/** Native window drag; call from a left-button mousedown. */
export function startWindowDrag(): void {
  if (!inTauri()) return;
  getCurrentWindow()
    .startDragging()
    .catch((err) => console.error("startDragging failed", err));
}

export function menuAction(action: "settings" | "quit"): void {
  call(action === "settings" ? "open_settings" : "quit_app");
}

export function closeSettings(): void {
  call("close_settings");
}

/** Screen brightness behind the widget, 0..1 (themeMode "auto"). */
export const onBg = (cb: (brightness: number) => void) => on<number>("widget-bg", cb);
export const onToggleExpand = (cb: () => void) => on<null>("widget-toggle-expand", () => cb());
export const onRefresh = (cb: () => void) => on<null>("widget-refresh", () => cb());
export const onWidgetSettingsChanged = (cb: () => void) =>
  on<null>("widget-settings-changed", () => cb());
