export type WidgetThemeName = 'dark' | 'light' | 'white'
export type SettingsThemeName = 'dark' | 'light'

export interface ThemeRoles {
  bg: string
  surface: string
  border: string
  text: string
  muted: string
  accent: string
  danger: string
  success: string
  track: string
  fieldLine: string
  focus: string
  barOk: string
  barWarn: string
  barDanger: string
  barDim: string
  badgeBlueBg: string
  badgeBlueText: string
  badgeBlueBorder: string
  badgeEmeraldBg: string
  badgeEmeraldText: string
  badgeEmeraldBorder: string
  badgeMutedBg: string
  badgeMutedText: string
  badgeMutedBorder: string
}

export const WIDGET_TYPE_SCALE = {
  title: 'text-[11px]',
  body: 'text-[10px] leading-3',
  caption: 'text-[9px] leading-3',
} as const

export const SETTINGS_TYPE_SCALE = {
  heading: 'text-base',
  body: 'text-sm',
  caption: 'text-xs',
} as const

export const WIDGET_THEMES: Record<WidgetThemeName, ThemeRoles> = {
  dark: {
    bg: '#18181b',
    surface: '#27272a',
    border: '#52525b',
    text: '#f4f4f5',
    muted: '#a1a1aa',
    accent: '#34d399',
    danger: '#fca5a5',
    success: '#6ee7b7',
    track: '#6e6e78',
    fieldLine: '#71717a',
    focus: '#34d399',
    barOk: '#34d399',
    barWarn: '#fbbf24',
    barDanger: '#f87171',
    barDim: '#71717a',
    badgeBlueBg: 'rgba(59, 130, 246, 0.15)',
    badgeBlueText: '#93c5fd',
    badgeBlueBorder: 'rgba(96, 165, 250, 0.35)',
    badgeEmeraldBg: 'rgba(16, 185, 129, 0.15)',
    badgeEmeraldText: '#6ee7b7',
    badgeEmeraldBorder: 'rgba(52, 211, 153, 0.35)',
    badgeMutedBg: 'rgba(113, 113, 122, 0.15)',
    badgeMutedText: '#a1a1aa',
    badgeMutedBorder: 'rgba(113, 113, 122, 0.35)',
  },
  light: {
    bg: '#e4e4e7',
    surface: '#d4d4d8',
    border: '#71717a',
    text: '#18181b',
    muted: '#3f3f46',
    accent: '#065f46',
    danger: '#991b1b',
    success: '#065f46',
    track: '#71717a',
    fieldLine: '#63636b',
    focus: '#1d4ed8',
    barOk: '#047857',
    barWarn: '#b45309',
    barDanger: '#b91c1c',
    barDim: '#52525b',
    badgeBlueBg: '#dbeafe',
    badgeBlueText: '#1e40af',
    badgeBlueBorder: '#93c5fd',
    badgeEmeraldBg: '#d1fae5',
    badgeEmeraldText: '#065f46',
    badgeEmeraldBorder: '#6ee7b7',
    badgeMutedBg: '#d4d4d8',
    badgeMutedText: '#3f3f46',
    badgeMutedBorder: '#71717a',
  },
  white: {
    bg: '#ffffff',
    surface: '#f4f4f5',
    border: '#71717a',
    text: '#18181b',
    muted: '#52525b',
    accent: '#047857',
    danger: '#b91c1c',
    success: '#047857',
    track: '#76767f',
    fieldLine: '#71717a',
    focus: '#1d4ed8',
    barOk: '#047857',
    barWarn: '#b45309',
    barDanger: '#b91c1c',
    barDim: '#52525b',
    badgeBlueBg: '#dbeafe',
    badgeBlueText: '#1e40af',
    badgeBlueBorder: '#93c5fd',
    badgeEmeraldBg: '#d1fae5',
    badgeEmeraldText: '#065f46',
    badgeEmeraldBorder: '#6ee7b7',
    badgeMutedBg: '#e4e4e7',
    badgeMutedText: '#3f3f46',
    badgeMutedBorder: '#71717a',
  },
}

export const SETTINGS_THEMES: Record<SettingsThemeName, ThemeRoles> = {
  dark: {
    bg: '#18181b',
    surface: '#27272a',
    border: '#3f3f46',
    text: '#f4f4f5',
    muted: '#a1a1aa',
    accent: '#34d399',
    danger: '#fca5a5',
    success: '#6ee7b7',
    track: '#6e6e78',
    fieldLine: '#71717a',
    focus: '#34d399',
    barOk: '#34d399',
    barWarn: '#fbbf24',
    barDanger: '#f87171',
    barDim: '#71717a',
    badgeBlueBg: 'rgba(59, 130, 246, 0.15)',
    badgeBlueText: '#93c5fd',
    badgeBlueBorder: 'rgba(96, 165, 250, 0.35)',
    badgeEmeraldBg: 'rgba(16, 185, 129, 0.15)',
    badgeEmeraldText: '#6ee7b7',
    badgeEmeraldBorder: 'rgba(52, 211, 153, 0.35)',
    badgeMutedBg: 'rgba(113, 113, 122, 0.15)',
    badgeMutedText: '#a1a1aa',
    badgeMutedBorder: 'rgba(113, 113, 122, 0.35)',
  },
  light: {
    bg: '#ffffff',
    surface: '#f4f4f5',
    border: '#d4d4d8',
    text: '#18181b',
    muted: '#52525b',
    accent: '#047857',
    danger: '#b91c1c',
    success: '#047857',
    track: '#76767f',
    fieldLine: '#71717a',
    focus: '#1d4ed8',
    barOk: '#047857',
    barWarn: '#b45309',
    barDanger: '#b91c1c',
    barDim: '#52525b',
    badgeBlueBg: 'rgba(59, 130, 246, 0.1)',
    badgeBlueText: '#1d4ed8',
    badgeBlueBorder: '#93c5fd',
    badgeEmeraldBg: 'rgba(16, 185, 129, 0.12)',
    badgeEmeraldText: '#047857',
    badgeEmeraldBorder: '#6ee7b7',
    badgeMutedBg: 'rgba(0, 0, 0, 0.06)',
    badgeMutedText: '#52525b',
    badgeMutedBorder: '#a1a1aa',
  },
}

export const TOAST_PALETTE = {
  baseBg: '#f9fafb',
  baseText: '#18181b',
  successBg: '#ecfdf5',
  successBorder: '#059669',
  errorBg: '#fef2f2',
  errorBorder: '#dc2626',
  loadingBg: '#eff6ff',
  loadingBorder: '#2563eb',
} as const

export function resolveWidgetThemeName(
  themeMode: 'auto' | 'light' | 'dark' | 'white',
  bgBright: number | null,
): WidgetThemeName {
  if (themeMode === 'white') return 'white'
  if (themeMode === 'light') return 'light'
  if (themeMode === 'dark') return 'dark'
  return bgBright !== null && bgBright < 0.5 ? 'light' : 'dark'
}

export function themeStyleVars(roles: ThemeRoles): Record<string, string> {
  return {
    '--role-bg': roles.bg,
    '--role-surface': roles.surface,
    '--role-border': roles.border,
    '--role-text': roles.text,
    '--role-muted': roles.muted,
    '--role-accent': roles.accent,
    '--role-danger': roles.danger,
    '--role-success': roles.success,
    '--role-track': roles.track,
    '--role-field-line': roles.fieldLine,
    '--role-focus': roles.focus,
    '--role-bar-ok': roles.barOk,
    '--role-bar-warn': roles.barWarn,
    '--role-bar-danger': roles.barDanger,
    '--role-bar-dim': roles.barDim,
    '--role-badge-blue-bg': roles.badgeBlueBg,
    '--role-badge-blue-text': roles.badgeBlueText,
    '--role-badge-blue-border': roles.badgeBlueBorder,
    '--role-badge-emerald-bg': roles.badgeEmeraldBg,
    '--role-badge-emerald-text': roles.badgeEmeraldText,
    '--role-badge-emerald-border': roles.badgeEmeraldBorder,
    '--role-badge-muted-bg': roles.badgeMutedBg,
    '--role-badge-muted-text': roles.badgeMutedText,
    '--role-badge-muted-border': roles.badgeMutedBorder,
  }
}

function srgbToLinear(channel: number): number {
  const c = channel / 255
  return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4)
}

export function relativeLuminance(hex: string): number {
  const clean = hex.replace('#', '').trim()
  if (clean.length !== 6) {
    throw new Error(`Expected 6-digit hex color, got ${hex}`)
  }
  const r = parseInt(clean.slice(0, 2), 16)
  const g = parseInt(clean.slice(2, 4), 16)
  const b = parseInt(clean.slice(4, 6), 16)
  return 0.2126 * srgbToLinear(r) + 0.7152 * srgbToLinear(g) + 0.0722 * srgbToLinear(b)
}

export function contrastRatio(foregroundHex: string, backgroundHex: string): number {
  const l1 = relativeLuminance(foregroundHex)
  const l2 = relativeLuminance(backgroundHex)
  const lighter = Math.max(l1, l2)
  const darker = Math.min(l1, l2)
  return (lighter + 0.05) / (darker + 0.05)
}
