export interface ServerStatusPayload {
  state: 'online' | 'restarting' | 'stopped'
  delayMs?: number
  attempt?: number
  crashes?: number
}

export interface UsageWidgetBridge {
  secret?: string
  close?: () => void
  minimize?: () => void
  openSettings?: () => void
  onBg?: (cb: (value: number) => void) => () => void
  onToggleExpand?: (cb: () => void) => () => void
  onSettingsChanged?: (cb: () => void) => () => void
  onServerStatus?: (cb: (status: ServerStatusPayload) => void) => () => void
  showMenu?: () => void
  startDrag?: (screenX: number, screenY: number) => void
  moveDrag?: (screenX: number, screenY: number) => void
  endDrag?: () => void
  resize?: (width: number, height: number) => void
}

export interface SettingsWindowBridge {
  secret?: string
  close?: () => void
}

type DesktopWindow = Window & {
  widget?: UsageWidgetBridge
  settingsWindow?: SettingsWindowBridge
}

function desktopWindow(): DesktopWindow | null {
  if (typeof window === 'undefined') return null
  return window as DesktopWindow
}

export function usageWidget(): UsageWidgetBridge | undefined {
  return desktopWindow()?.widget
}

export function settingsWindow(): SettingsWindowBridge | undefined {
  return desktopWindow()?.settingsWindow
}
