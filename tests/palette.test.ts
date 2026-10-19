import { describe, expect, it } from "vitest";
import { contrast, PANEL_PALETTES, WIDGET_PALETTES, type Tone } from "@/ui/palette";

const AA_TEXT = 4.5;
const AA_NON_TEXT = 3; // WCAG 1.4.11: meaningful graphics such as meter fills
const tones: Tone[] = ["blue", "green", "violet", "gray", "red", "amber"];

describe("contrast()", () => {
  it("matches known WCAG ratios", () => {
    expect(contrast("#000000", "#ffffff")).toBeCloseTo(21, 0);
    // The old widget's "cached" hint (zinc-500 on zinc-900) measured 3.67:1.
    expect(contrast("#71717b", "#18181b")).toBeCloseTo(3.67, 1);
  });
});

describe.each(Object.entries(WIDGET_PALETTES))("widget %s theme", (_name, p) => {
  it.each(["text", "label", "muted", "peakActive", "peakNext", "warn"] as const)(
    "%s text meets AA on the card",
    (key) => expect(contrast(p[key], p.card)).toBeGreaterThanOrEqual(AA_TEXT),
  );

  it.each(tones)("%s badge text meets AA", (tone) => {
    const b = p.badge[tone];
    expect(contrast(b.fg, b.bg, p.card)).toBeGreaterThanOrEqual(AA_TEXT);
  });

  it.each(["barOk", "barWarn", "barDanger"] as const)("%s meter fill meets 3:1", (key) => {
    expect(contrast(p[key], p.card)).toBeGreaterThanOrEqual(AA_NON_TEXT);
  });

  // Signed-out / lapsed accounts swap in these instead of fading the card with opacity.
  it.each(["dimText", "dimMuted"] as const)("dimmed %s meets AA on the card", (key) => {
    expect(contrast(p[key], p.card)).toBeGreaterThanOrEqual(AA_TEXT);
  });

  it.each(["dimBarOk", "dimBarWarn", "dimBarDanger", "barDim"] as const)(
    "dimmed %s meter fill meets 3:1",
    (key) => expect(contrast(p[key], p.card)).toBeGreaterThanOrEqual(AA_NON_TEXT),
  );

  it("focus ring meets 3:1 against the card", () => {
    expect(contrast(p.focus, p.card)).toBeGreaterThanOrEqual(3);
  });

  it("dimmed colours are quieter than the normal ones", () => {
    expect(contrast(p.dimText, p.card)).toBeLessThan(contrast(p.text, p.card));
    expect(contrast(p.dimMuted, p.card)).toBeLessThanOrEqual(contrast(p.muted, p.card));
  });
});

describe.each(Object.entries(PANEL_PALETTES))("settings %s panel", (_name, p) => {
  it.each(["text", "textSecondary", "muted", "danger", "success"] as const)(
    "%s text meets AA on bg and surface",
    (key) => {
      expect(contrast(p[key], p.bg)).toBeGreaterThanOrEqual(AA_TEXT);
      expect(contrast(p[key], p.surface)).toBeGreaterThanOrEqual(AA_TEXT);
    },
  );

  it("buttons meet AA", () => {
    expect(contrast(p.primaryFg, p.primaryBg)).toBeGreaterThanOrEqual(AA_TEXT);
    expect(contrast(p.dangerFg, p.dangerBg)).toBeGreaterThanOrEqual(AA_TEXT);
  });

  it("input borders, focus ring and switch meet 3:1", () => {
    expect(contrast(p.inputBorder, p.bg)).toBeGreaterThanOrEqual(AA_NON_TEXT);
    expect(contrast(p.focus, p.bg)).toBeGreaterThanOrEqual(AA_NON_TEXT);
    expect(contrast(p.switchOn, p.surface)).toBeGreaterThanOrEqual(AA_NON_TEXT);
  });

  it.each(tones)("%s badge text meets AA", (tone) => {
    const b = p.badge[tone];
    expect(contrast(b.fg, b.bg, p.surface)).toBeGreaterThanOrEqual(AA_TEXT);
  });
});
