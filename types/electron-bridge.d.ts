// APIs exposed by desktop/preload.js and desktop/settings-preload.js.
// Absent when the pages are opened in a normal browser.

type Unsubscribe = () => void;

interface WidgetBridge {
  secret: string;
  resize(width: number, height: number): void;
  onBg(cb: (brightness: number) => void): Unsubscribe;
  onToggleExpand(cb: () => void): Unsubscribe;
  onRefresh(cb: () => void): Unsubscribe;
  onSettingsChanged(cb: () => void): Unsubscribe;
  showMenu(): void;
  startDrag(screenX: number, screenY: number): void;
  moveDrag(screenX: number, screenY: number): void;
  endDrag(): void;
}

interface SettingsBridge {
  secret: string;
  close(): void;
}

interface Window {
  widget?: WidgetBridge;
  settingsWindow?: SettingsBridge;
}
