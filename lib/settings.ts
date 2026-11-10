import { promises as fs } from 'fs'
import path from 'path'
import { dataDir } from '@/lib/data-dir'

export type ThemeMode = 'auto' | 'light' | 'dark' | 'white'
export type PanelTheme = 'light' | 'dark'

export interface Settings {
  cursorEnabled: boolean
  grokEnabled: boolean
  codexEnabled: boolean
  claudeEnabled: boolean
  deepseekPeakHourWarning: boolean
  showProviderNames: boolean
  themeMode: ThemeMode
  panelTheme: PanelTheme
  displayNames: Record<string, string>
  disabledAccounts: string[]
}

function settingsFile(): string {
  return path.join(dataDir(), 'settings.json')
}

export async function getSettings(): Promise<Settings> {
  try {
    const parsed = JSON.parse(await fs.readFile(settingsFile(), 'utf8'))
    const mode = parsed?.themeMode
    return {
      cursorEnabled: parsed?.cursorEnabled !== false,
      grokEnabled: parsed?.grokEnabled !== false,
      codexEnabled: parsed?.codexEnabled !== false,
      claudeEnabled: parsed?.claudeEnabled !== false,
      deepseekPeakHourWarning: parsed?.deepseekPeakHourWarning !== false,
      showProviderNames: parsed?.showProviderNames === true,
      themeMode: mode === 'light' || mode === 'dark' || mode === 'white' ? mode : 'auto',
      panelTheme: parsed?.panelTheme === 'light' ? 'light' : 'dark',
      displayNames:
        parsed?.displayNames && typeof parsed.displayNames === 'object'
          ? parsed.displayNames
          : {},
      disabledAccounts: Array.isArray(parsed?.disabledAccounts)
        ? parsed.disabledAccounts.map((e: unknown) => String(e).toLowerCase())
        : [],
    }
  } catch {
    return {
      cursorEnabled: true,
      grokEnabled: true,
      codexEnabled: true,
      claudeEnabled: true,
      deepseekPeakHourWarning: true,
      showProviderNames: false,
      themeMode: 'auto',
      panelTheme: 'dark',
      displayNames: {},
      disabledAccounts: [],
    }
  }
}

export async function saveSettings(s: Settings): Promise<void> {
  const file = settingsFile()
  await fs.mkdir(path.dirname(file), { recursive: true })
  await fs.writeFile(file, JSON.stringify(s, null, 2), 'utf8')
}
