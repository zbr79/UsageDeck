import { useEffect, useRef, useState, type PointerEvent } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { inTauri } from "@/lib/bridge";
import type { PanelTheme, Settings, ThemeMode, WidgetProvider } from "@/lib/types";
import { Toggle } from "@/ui/Toggle";
import appIcon from "../../../src-tauri/icons/128x128.png";
import { moveItem, PAGE_TITLES, PROVIDER_ICONS } from "../_lib/pages";
import { usePanelTheme } from "../_lib/theme";
import { Page, Section, secondaryButton, selectClass, SettingRow } from "./fields";

type Update = (patch: Partial<Settings>, success?: string) => Promise<boolean>;
type Props = { settings: Settings; update: Update };

const ENABLED_KEY: Record<WidgetProvider, keyof Settings & `${string}Enabled`> = {
  claude: "claudeEnabled",
  codex: "codexEnabled",
  cursor: "cursorEnabled",
  grok: "grokEnabled",
  opencode: "opencodeEnabled",
};

export function GeneralPage({ settings, update }: Props) {
  return (
    <Page title="General">
      <Section>
        <SettingRow
          title="Start with Windows"
          description="Open the widget when you sign in to Windows"
          control={(labelId) => (
            <Toggle
              labelledBy={labelId}
              checked={settings.startAtLogin}
              onChange={(startAtLogin) =>
                void update(
                  { startAtLogin },
                  startAtLogin ? "Starts with Windows" : "No longer starts with Windows",
                )
              }
            />
          )}
        />
        <SettingRow
          title="Hide during full-screen apps"
          description="Games, videos and presentations stay clear of the widget"
          control={(labelId) => (
            <Toggle
              labelledBy={labelId}
              checked={settings.hideInFullscreen}
              onChange={(hideInFullscreen) =>
                void update(
                  { hideInFullscreen },
                  hideInFullscreen
                    ? "Hides during full-screen apps"
                    : "Stays visible during full-screen apps",
                )
              }
            />
          )}
        />
        <SettingRow
          title="DeepSeek peak-hour warning"
          description="Show peak-hour start and end countdowns on the top OpenCode card"
          control={(labelId) => (
            <Toggle
              labelledBy={labelId}
              checked={settings.deepseekPeakHourWarning}
              onChange={(deepseekPeakHourWarning) =>
                void update(
                  { deepseekPeakHourWarning },
                  deepseekPeakHourWarning ? "Peak-hour warning on" : "Peak-hour warning off",
                )
              }
            />
          )}
        />
      </Section>
    </Page>
  );
}

const PANEL_THEMES: { value: PanelTheme; label: string }[] = [
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
  { value: "system", label: "Use system setting" },
];
const WIDGET_THEMES: { value: ThemeMode; label: string }[] = [
  { value: "auto", label: "Match the screen behind it" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

export function AppearancePage({ settings, update }: Props) {
  return (
    <Page title="Appearance">
      <Section title="Theme">
        <SettingRow
          title="Settings theme"
          description="Colours of this settings window"
          control={(labelId, descId) => (
            <select
              aria-labelledby={labelId}
              aria-describedby={descId}
              value={settings.panelTheme}
              onChange={(e) => void update({ panelTheme: e.target.value as PanelTheme })}
              className={selectClass}
            >
              {PANEL_THEMES.map((t) => (
                <option key={t.value} value={t.value}>
                  {t.label}
                </option>
              ))}
            </select>
          )}
        />
        <SettingRow
          title="Widget theme"
          description="Match follows the brightness of the screen behind the widget"
          control={(labelId, descId) => (
            <select
              aria-labelledby={labelId}
              aria-describedby={descId}
              value={settings.themeMode}
              onChange={(e) => void update({ themeMode: e.target.value as ThemeMode })}
              className={selectClass}
            >
              {WIDGET_THEMES.map((t) => (
                <option key={t.value} value={t.value}>
                  {t.label}
                </option>
              ))}
            </select>
          )}
        />
      </Section>
      <Section title="Cards">
        <SettingRow
          title="Show provider names"
          description="Title widget cards with the provider name instead of the account nickname"
          control={(labelId) => (
            <Toggle
              labelledBy={labelId}
              checked={settings.showProviderNames}
              onChange={(showProviderNames) =>
                void update(
                  { showProviderNames },
                  showProviderNames ? "Cards show provider names" : "Cards show nicknames",
                )
              }
            />
          )}
        />
      </Section>
    </Page>
  );
}

/** Provider order (drag a row; arrow keys on its grip) and each provider's widget switch. */
export function WidgetPage({ settings, update }: Props) {
  const light = usePanelTheme() === "light";
  const order = settings.providerOrder;
  const rows = useRef<(HTMLLIElement | null)[]>([]);
  const drag = useRef<{ id: WidgetProvider; pointerId: number } | null>(null);
  const [dragging, setDragging] = useState<{ id: WidgetProvider; to: number } | null>(null);
  const reorder = (from: number, to: number) => {
    if (from === to) return;
    void update({ providerOrder: moveItem(order, from, to) });
  };
  const dropIndex = (y: number) => {
    for (let i = 0; i < order.length; i++) {
      const r = rows.current[i]?.getBoundingClientRect();
      if (r && y < r.top + r.height / 2) return i;
    }
    return order.length - 1;
  };
  const onPointerDown = (e: PointerEvent<HTMLLIElement>, id: WidgetProvider) => {
    // The whole row drags, except its switch.
    if (e.button !== 0 || (e.target as HTMLElement).closest("button")) return;
    drag.current = { id, pointerId: e.pointerId };
    e.currentTarget.setPointerCapture(e.pointerId);
    setDragging({ id, to: order.indexOf(id) });
  };
  const onPointerMove = (e: PointerEvent<HTMLLIElement>) => {
    const d = drag.current;
    if (!d || d.pointerId !== e.pointerId) return;
    const to = dropIndex(e.clientY);
    setDragging((prev) => (prev && prev.to !== to ? { ...prev, to } : prev));
  };
  const onPointerUp = (e: PointerEvent<HTMLLIElement>) => {
    const d = drag.current;
    if (!d || d.pointerId !== e.pointerId) return;
    drag.current = null;
    setDragging(null);
    reorder(order.indexOf(d.id), dropIndex(e.clientY));
  };

  return (
    <Page title="Widget">
      <Section title="Providers">
        <p className="mb-2 text-xs text-(--p-muted)">
          Drag to reorder. The switch shows or hides the card.
        </p>
        <ol className="flex flex-col gap-1">
          {order.map((id, i) => {
            const name = PAGE_TITLES[id];
            const key = ENABLED_KEY[id];
            const target = dragging && dragging.to === i && dragging.id !== id;
            return (
              <li
                key={id}
                ref={(el) => {
                  rows.current[i] = el;
                }}
                onPointerDown={(e) => onPointerDown(e, id)}
                onPointerMove={onPointerMove}
                onPointerUp={onPointerUp}
                onPointerCancel={() => {
                  drag.current = null;
                  setDragging(null);
                }}
                className={`ud-row flex min-h-[52px] cursor-grab touch-none select-none items-center gap-3 rounded border bg-(--s-card) px-3 py-2 active:cursor-grabbing ${
                  target ? "border-(--p-focus)" : "border-(--s-cardBorder)"
                } ${dragging?.id === id ? "opacity-60" : ""}`}
              >
                <span
                  role="button"
                  tabIndex={0}
                  aria-label={`Reorder ${name}, position ${i + 1} of ${order.length}`}
                  aria-keyshortcuts="ArrowUp ArrowDown"
                  title="Drag to reorder"
                  onKeyDown={(e) => {
                    const to = e.key === "ArrowUp" ? i - 1 : e.key === "ArrowDown" ? i + 1 : -1;
                    if (to < 0 || to >= order.length) return;
                    e.preventDefault();
                    reorder(i, to);
                  }}
                  className="flex h-8 w-6 items-center justify-center rounded text-(--p-muted)"
                >
                  <svg width="10" height="16" viewBox="0 0 10 16" fill="currentColor">
                    {[2, 8, 14].map((y) => (
                      <g key={y}>
                        <circle cx="2.5" cy={y} r="1.3" />
                        <circle cx="7.5" cy={y} r="1.3" />
                      </g>
                    ))}
                  </svg>
                </span>
                <img
                  src={PROVIDER_ICONS[id]}
                  alt=""
                  draggable={false}
                  className={`size-5 rounded ${id === "codex" && light ? "invert" : ""}`}
                />
                <span className="min-w-0 flex-1 text-sm">{name}</span>
                <Toggle
                  label={`Show ${name} on the widget`}
                  checked={settings[key]}
                  onChange={(next) =>
                    void update(
                      { [key]: next },
                      `${name} ${next ? "shown on" : "hidden from"} widget`,
                    )
                  }
                />
              </li>
            );
          })}
        </ol>
      </Section>
    </Page>
  );
}

function openAboutLink(link: "logs" | "repo") {
  if (!inTauri()) return;
  invoke("open_about_link", { link }).catch((err) => console.error("open link failed", err));
}

export function AboutPage() {
  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => {
    if (!inTauri()) {
      setVersion("preview");
      return;
    }
    getVersion().then(setVersion, () => setVersion(null));
  }, []);
  return (
    <Page title="About">
      <Section>
        <div className="ud-row flex items-center gap-4 rounded border border-(--s-cardBorder) bg-(--s-card) p-4">
          <img src={appIcon} alt="" className="size-12" />
          <div>
            <div className="text-xl leading-7 font-semibold">UsageDeck</div>
            <div className="text-xs text-(--p-muted)">
              Version {version ?? "unknown"} · usage monitor for Claude, Codex, Cursor, Grok Bot and
              OpenCode
            </div>
          </div>
        </div>
        <SettingRow
          title="Log folder"
          description="Where UsageDeck writes its logs"
          control={(labelId) => (
            <button
              type="button"
              aria-describedby={labelId}
              onClick={() => openAboutLink("logs")}
              className={secondaryButton}
            >
              Open log folder
            </button>
          )}
        />
        <SettingRow
          title="Source code"
          description="github.com/zbr79/UsageDeck"
          control={(labelId) => (
            <button
              type="button"
              aria-describedby={labelId}
              onClick={() => openAboutLink("repo")}
              className={secondaryButton}
            >
              Open on GitHub
            </button>
          )}
        />
      </Section>
    </Page>
  );
}
