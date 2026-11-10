import { describe, expect, it } from 'vitest'
import {
  contrastRatio,
  SETTINGS_THEMES,
  TOAST_PALETTE,
  WIDGET_THEMES,
  type ThemeRoles,
} from '@/lib/theme'

function assertThemeContrast(name: string, roles: ThemeRoles) {
  describe(name, () => {
    it('keeps primary and muted text at or above 4.5:1 on both bg and surface', () => {
      expect(contrastRatio(roles.text, roles.bg)).toBeGreaterThanOrEqual(4.5)
      expect(contrastRatio(roles.text, roles.surface)).toBeGreaterThanOrEqual(4.5)
      expect(contrastRatio(roles.muted, roles.bg)).toBeGreaterThanOrEqual(4.5)
      expect(contrastRatio(roles.muted, roles.surface)).toBeGreaterThanOrEqual(4.5)
    })

    it('keeps semantic text roles (accent, danger, success) at or above 4.5:1 on card bg', () => {
      expect(contrastRatio(roles.accent, roles.bg)).toBeGreaterThanOrEqual(4.5)
      expect(contrastRatio(roles.danger, roles.bg)).toBeGreaterThanOrEqual(4.5)
      expect(contrastRatio(roles.success, roles.bg)).toBeGreaterThanOrEqual(4.5)
    })

    it('keeps bar tracks, bar fills, field borders, and focus rings at or above 3:1', () => {
      expect(contrastRatio(roles.track, roles.bg)).toBeGreaterThanOrEqual(3.0)
      expect(contrastRatio(roles.barOk, roles.bg)).toBeGreaterThanOrEqual(3.0)
      expect(contrastRatio(roles.barWarn, roles.bg)).toBeGreaterThanOrEqual(3.0)
      expect(contrastRatio(roles.barDanger, roles.bg)).toBeGreaterThanOrEqual(3.0)
      expect(contrastRatio(roles.fieldLine, roles.bg)).toBeGreaterThanOrEqual(3.0)
      expect(contrastRatio(roles.fieldLine, roles.surface)).toBeGreaterThanOrEqual(3.0)
      expect(contrastRatio(roles.focus, roles.bg)).toBeGreaterThanOrEqual(3.0)
    })
  })
}

describe('theme contrast guarantees', () => {
  for (const [key, roles] of Object.entries(WIDGET_THEMES)) {
    assertThemeContrast(`widget-${key}`, roles)
  }

  for (const [key, roles] of Object.entries(SETTINGS_THEMES)) {
    assertThemeContrast(`settings-${key}`, roles)
  }

  it('keeps toast text above 4.5:1 against every toast background', () => {
    expect(contrastRatio(TOAST_PALETTE.baseText, TOAST_PALETTE.baseBg)).toBeGreaterThanOrEqual(4.5)
    expect(contrastRatio(TOAST_PALETTE.baseText, TOAST_PALETTE.successBg)).toBeGreaterThanOrEqual(4.5)
    expect(contrastRatio(TOAST_PALETTE.baseText, TOAST_PALETTE.errorBg)).toBeGreaterThanOrEqual(4.5)
    expect(contrastRatio(TOAST_PALETTE.baseText, TOAST_PALETTE.loadingBg)).toBeGreaterThanOrEqual(4.5)
  })
})
