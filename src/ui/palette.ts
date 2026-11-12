// Design tokens for the widget and the settings panel. Every text/background
// pair is checked for WCAG AA contrast in tests/unit/palette.test.ts.
import type { CSSProperties } from "react";

export type Tone = "blue" | "green" | "violet" | "gray" | "red" | "amber";

export interface BadgeColors {
  bg: string;
  fg: string;
  border: string;
}

export interface WidgetPalette {
  card: string;
  cardBorder: string;
  text: string;
  label: string;
  muted: string;
  track: string;
  barOk: string;
  barWarn: string;
  barDanger: string;
  barDim: string;
  /** Fable's part of a Claude meter (purple, 3:1 against the card) */
  barFable: string;
  /** Claude's even-pace tick on a meter (3:1 against the track and the card) */
  paceTick: string;
  /** text and meter fills of a signed-out or lapsed account (instead of fading with opacity) */
  dimText: string;
  dimMuted: string;
  dimBarOk: string;
  dimBarWarn: string;
  dimBarDanger: string;
  ringOk: string;
  ringDanger: string;
  peakActive: string;
  peakNext: string;
  warn: string;
  /** spend on target / above target (the Claude header vs the hourly target; `warn` is below) */
  success: string;
  danger: string;
  hover: string;
  /** keyboard focus ring (the settings panel's --p-focus values, 3:1 against the card) */
  focus: string;
  badge: Record<Tone, BadgeColors>;
}

export type WidgetTheme = "dark" | "light";

const darkBadges: Record<Tone, BadgeColors> = {
  blue: { bg: "rgba(59,130,246,0.15)", fg: "#8ec5ff", border: "rgba(96,165,250,0.3)" },
  green: { bg: "rgba(16,185,129,0.15)", fg: "#5ee9b5", border: "rgba(52,211,153,0.3)" },
  violet: { bg: "rgba(139,92,246,0.15)", fg: "#c4b4ff", border: "rgba(167,139,250,0.3)" },
  gray: { bg: "rgba(113,113,123,0.15)", fg: "#9f9fa9", border: "rgba(113,113,123,0.3)" },
  red: { bg: "rgba(239,68,68,0.15)", fg: "#ffa2a2", border: "rgba(248,113,113,0.3)" },
  amber: { bg: "rgba(245,158,11,0.15)", fg: "#ffd230", border: "rgba(251,191,36,0.3)" },
};

const lightBadges: Record<Tone, BadgeColors> = {
  blue: { bg: "#dbeafe", fg: "#193cb8", border: "#8ec5ff" },
  green: { bg: "#d0fae5", fg: "#006045", border: "#5ee9b5" },
  violet: { bg: "#ede9fe", fg: "#5d0ec0", border: "#c4b4ff" },
  gray: { bg: "#d4d4d8", fg: "#3f3f46", border: "#9f9fa9" },
  red: { bg: "#ffe2e2", fg: "#9f0712", border: "#ffa2a2" },
  amber: { bg: "#fef3c6", fg: "#973c00", border: "#ffd230" },
};

const lightBase: Omit<WidgetPalette, "card" | "track"> = {
  cardBorder: "rgba(113,113,123,0.45)",
  text: "#18181b",
  label: "#3f3f46",
  muted: "#52525c",
  barOk: "#007a55",
  barWarn: "#bb4d00",
  barDanger: "#c10007",
  barDim: "#71717b",
  barFable: "#7f22fe",
  paceTick: "#3f3f46",
  dimText: "#5f5f66",
  dimMuted: "#62626b",
  dimBarOk: "#2b8e71",
  dimBarWarn: "#c26727",
  dimBarDanger: "#cf595e",
  ringOk: "#009966",
  ringDanger: "#e7000b",
  peakActive: "#9f0712",
  peakNext: "#006045",
  warn: "#973c00",
  success: "#006045",
  danger: "#9f0712",
  hover: "rgba(0,0,0,0.08)",
  focus: "#1447e6",
  badge: lightBadges,
};

export const WIDGET_PALETTES: Record<WidgetTheme, WidgetPalette> = {
  dark: {
    card: "#18181b",
    cardBorder: "rgba(255,255,255,0.15)",
    text: "#f4f4f5",
    label: "#e4e4e7",
    muted: "#9f9fa9",
    track: "rgba(255,255,255,0.15)",
    barOk: "#00d492",
    barWarn: "#ffb900",
    barDanger: "#ff6467",
    barDim: "#71717b",
    barFable: "#a684ff",
    paceTick: "#e4e4e7",
    dimText: "#a1a1aa",
    dimMuted: "#83838b",
    dimBarOk: "#0a8962",
    dimBarWarn: "#a3790b",
    dimBarDanger: "#ac494c",
    ringOk: "#00d492",
    ringDanger: "#ff6467",
    peakActive: "#ffa2a2",
    peakNext: "#5ee9b5",
    warn: "#ffd230",
    success: "#5ee9b5",
    danger: "#ffa2a2",
    hover: "rgba(255,255,255,0.1)",
    focus: "#8ec5ff",
    badge: darkBadges,
  },
  light: { ...lightBase, card: "#e4e4e7", track: "rgba(113,113,123,0.25)" },
};

/**
 * The widget's cards as drawn inside the settings window. Dark is the widget's own; light sits on
 * the settings rows' surface (`--s-card` #fbfbfb, `--s-cardBorder` #e5e5e5 in settings/App.tsx)
 * instead of the widget's grey #e4e4e7, so cards and rows read as one family. Muted text is the
 * panel's own muted colour.
 */
export const SETTINGS_CARD_PALETTES: Record<WidgetTheme, WidgetPalette> = {
  dark: WIDGET_PALETTES.dark,
  light: {
    ...WIDGET_PALETTES.light,
    card: "#fbfbfb",
    cardBorder: "#e5e5e5",
    muted: "#52525c",
    track: "rgba(113,113,123,0.2)",
  },
};

export interface PanelPalette {
  bg: string;
  surface: string;
  surfaceHover: string;
  border: string;
  accountActiveBg: string;
  accountActiveBorder: string;
  accountInactiveBg: string;
  divider: string;
  text: string;
  textSecondary: string;
  muted: string;
  inputBg: string;
  inputBorder: string;
  accent: string;
  switchOn: string;
  focus: string;
  primaryBg: string;
  primaryFg: string;
  primaryHover: string;
  dangerBg: string;
  dangerFg: string;
  danger: string;
  success: string;
  /** amber text (spend below the hourly target) */
  warning: string;
  badge: Record<Tone, BadgeColors>;
}

export const PANEL_PALETTES: Record<"dark" | "light", PanelPalette> = {
  dark: {
    bg: "#18181b",
    surface: "#27272a",
    surfaceHover: "#3f3f46",
    border: "#3f3f46",
    accountActiveBg: "rgba(16,185,129,0.24)",
    accountActiveBorder: "rgba(52,211,153,0.55)",
    accountInactiveBg: "#3f3f46",
    divider: "#27272a",
    text: "#f4f4f5",
    textSecondary: "#d4d4d8",
    muted: "#9f9fa9",
    inputBg: "#18181b",
    inputBorder: "#71717b",
    accent: "#00d492",
    switchOn: "#009966",
    focus: "#8ec5ff",
    primaryBg: "#e4e4e7",
    primaryFg: "#18181b",
    primaryHover: "#d4d4d8",
    dangerBg: "#c10007",
    dangerFg: "#ffffff",
    danger: "#ffa2a2",
    success: "#5ee9b5",
    warning: "#ffd230",
    badge: darkBadges,
  },
  light: {
    bg: "#ffffff",
    surface: "#f4f4f5",
    surfaceHover: "#e4e4e7",
    border: "#d4d4d8",
    accountActiveBg: "#86b28e",
    accountActiveBorder: "#2f6f3b",
    accountInactiveBg: "#d4d4d8",
    divider: "#e4e4e7",
    text: "#18181b",
    textSecondary: "#3f3f46",
    muted: "#52525c",
    inputBg: "#ffffff",
    inputBorder: "#71717b",
    accent: "#009966",
    switchOn: "#009966",
    focus: "#1447e6",
    primaryBg: "#18181b",
    primaryFg: "#ffffff",
    primaryHover: "#3f3f46",
    dangerBg: "#c10007",
    dangerFg: "#ffffff",
    danger: "#9f0712",
    success: "#006045",
    warning: "#973c00",
    badge: lightBadges,
  },
};

/** The settings rows' card colour (`--s-card` in settings/App.tsx). */
export const SETTINGS_ROW_CARD = { dark: "#2b2b2b", light: "#fbfbfb" } as const;

/** Text colour per spend tone (see `SPEND_BAND_TONE` in lib/pace.ts); "muted" keeps each place's own muted style. */
export const WIDGET_SPEND_TONE = {
  warning: "warn",
  success: "success",
  danger: "danger",
} as const satisfies Record<string, keyof WidgetPalette>;
export const PANEL_SPEND_TONE = {
  warning: "warning",
  success: "success",
  danger: "danger",
} as const satisfies Record<string, keyof PanelPalette>;

/** Flatten a palette into CSS custom properties: { '--w-card': … }. */
export function cssVars(prefix: string, palette: WidgetPalette | PanelPalette): CSSProperties {
  const vars: Record<string, string> = {};
  for (const [key, value] of Object.entries(palette)) {
    if (typeof value === "string") {
      vars[`--${prefix}-${key}`] = value;
    } else {
      for (const [tone, colors] of Object.entries(value as Record<Tone, BadgeColors>)) {
        vars[`--${prefix}-badge-${tone}-bg`] = colors.bg;
        vars[`--${prefix}-badge-${tone}-fg`] = colors.fg;
        vars[`--${prefix}-badge-${tone}-border`] = colors.border;
      }
    }
  }
  return vars as CSSProperties;
}

// ---- contrast (WCAG 2.x) ---------------------------------------------------

type RGBA = [number, number, number, number];

export function parseColor(color: string): RGBA {
  const hex = color.match(/^#([0-9a-f]{6})$/i);
  if (hex) {
    const n = parseInt(hex[1], 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255, 1];
  }
  const rgba = color.match(/^rgba?\(([^)]+)\)$/);
  if (rgba) {
    const [r, g, b, a = "1"] = rgba[1].split(",").map((s) => s.trim());
    return [Number(r), Number(g), Number(b), Number(a)];
  }
  throw new Error(`unsupported color ${color}`);
}

/** Composite `top` over an opaque `bottom`. */
export function over(top: string, bottom: string): RGBA {
  const [r, g, b, a] = parseColor(top);
  const [R, G, B] = parseColor(bottom);
  return [r * a + R * (1 - a), g * a + G * (1 - a), b * a + B * (1 - a), 1];
}

function luminance([r, g, b]: RGBA): number {
  const f = (v: number) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

export function contrast(fg: string, bg: string, base = bg): number {
  const back = over(bg, base);
  const fore = over(fg, `rgb(${back[0]},${back[1]},${back[2]})`);
  const l1 = luminance(fore);
  const l2 = luminance(back);
  return (Math.max(l1, l2) + 0.05) / (Math.min(l1, l2) + 0.05);
}
