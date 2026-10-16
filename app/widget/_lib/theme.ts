import type { WidgetTheme } from "@/app/_ui/palette";
import type { ThemeMode } from "@/lib/types";

/**
 * Hysteresis for the sampled screen brightness (0..1) so the auto theme
 * doesn't flicker around the threshold: only switch to light cards when the
 * background is clearly dark (< 0.3), and back when clearly bright (≥ 0.45).
 */
export function nextBrightness(prev: number | null, sample: number): number | null {
  if (prev === null) return sample;
  const showingLight = prev < 0.5;
  if (!showingLight && sample < 0.3) return sample;
  if (showingLight && sample >= 0.45) return sample;
  return prev;
}

/** Auto: light cards over dark backgrounds, dark cards over bright ones. */
export function resolveWidgetTheme(mode: ThemeMode, brightness: number | null): WidgetTheme {
  if (mode === "light") return "light";
  if (mode === "dark") return "dark";
  return brightness !== null && brightness < 0.5 ? "light" : "dark";
}
