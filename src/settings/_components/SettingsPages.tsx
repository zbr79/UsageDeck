import { useEffect, useRef, useState, type PointerEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { inTauri } from "@/lib/bridge";
import { formatAgo, formatResetDuration } from "@/lib/format";
import { checkForUpdates, installUpdate, useUpdateStatus, type UpdateStatus } from "@/lib/updates";
import type { SettingsPatch } from "@/lib/settings-schema";
import type { PanelTheme, Settings, ThemeMode, WidgetProvider } from "@/lib/types";
import { Toggle } from "@/ui/Toggle";
import appIcon from "../../../src-tauri/icons/128x128.png";
import { moveItem, PAGE_TITLES, PROVIDER_ICONS } from "../_lib/pages";
import { usePanelTheme } from "../_lib/theme";
import { CopyDiagnosticsButton } from "./Diagnostics";
import { Page, primaryButton, Section, secondaryButton, selectClass, SettingRow } from "./fields";

type Update = (patch: SettingsPatch, success?: string) => Promise<boolean>;
type Props = { settings: Settings; update: Update };

const ENABLED_KEY: Record<WidgetProvider, keyof Settings & `${string}Enabled`> = {
  claude: "claudeEnabled",
  codex: "codexEnabled",
  cursor: "cursorEnabled",
  grok: "grokEnabled",
  opencode: "opencodeEnabled",
};

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

export function GeneralPage({ settings, update }: Props) {
  return (
    <Page title="General">
      <Section>
        <SettingRow
          title="Start with Windows"
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
      <Section title="Theme">
        <SettingRow
          title="Settings theme"
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
      <UpdatesSection settings={settings} update={update} />
    </Page>
  );
}

/** The update row's description: the last check's result, or when it ran. */
export function updateDescription(status: UpdateStatus, now: number): string {
  const checked = status.lastChecked == null ? null : formatAgo(status.lastChecked, now);
  switch (status.state) {
    case "checking":
      return "Checking for updates…";
    case "downloading":
      return `Downloading ${status.version}…`;
    case "upToDate":
      return `You're up to date · checked ${checked ?? "just now"}`;
    case "failed":
      return `Couldn't check for updates: ${status.message}`;
    case "ready":
      return `Version ${status.version} is downloaded and ready to install`;
    case "idle":
      return checked ? `Last checked ${checked}` : "Not checked yet";
  }
}

function UpdatesSection({ settings, update }: Props) {
  const status = useUpdateStatus();
  const [now, setNow] = useState(() => Date.now());
  const [restarting, setRestarting] = useState(false);
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(id);
  }, []);
  // "just now" right after a check, not the minute-old clock
  useEffect(() => setNow(Date.now()), [status]);
  if (!status) return null;
  const busy = status.state === "checking" || status.state === "downloading";
  const restart = () => {
    setRestarting(true);
    installUpdate().catch(() => setRestarting(false));
  };
  return (
    <Section title="UsageDeck" id="updates">
      <SettingRow
        title={`Version ${status.currentVersion}`}
        description={
          updateDescription(status, now) +
          (settings.autoInstallUpdates && status.state === "ready"
            ? " · installs when you're away"
            : "")
        }
        live
        icon={<img src={appIcon} alt="" className="size-5" />}
        control={(labelId, descId) =>
          status.state === "ready" ? (
            <button
              type="button"
              aria-describedby={descId}
              disabled={restarting}
              onClick={restart}
              className={primaryButton}
            >
              {restarting ? "Restarting…" : "Restart to update"}
            </button>
          ) : (
            <button
              type="button"
              aria-describedby={descId ?? labelId}
              disabled={busy}
              onClick={checkForUpdates}
              className={secondaryButton}
            >
              {busy ? "Checking…" : "Check for updates"}
            </button>
          )
        }
      />
      <SettingRow
        title="Install updates automatically"
        control={(labelId) => (
          <Toggle
            labelledBy={labelId}
            checked={settings.autoInstallUpdates}
            onChange={(autoInstallUpdates) =>
              void update(
                { autoInstallUpdates },
                autoInstallUpdates
                  ? "Installs updates automatically"
                  : "Updates wait for you to install them",
              )
            }
          />
        )}
      />
      <SettingRow
        title="Support"
        control={() => (
          <div className="flex flex-wrap justify-end gap-2">
            <button type="button" onClick={() => openAboutLink("logs")} className={secondaryButton}>
              Log folder
            </button>
            <button type="button" onClick={() => openAboutLink("repo")} className={secondaryButton}>
              GitHub
            </button>
            <CopyDiagnosticsButton />
          </div>
        )}
      />
    </Section>
  );
}

/** Provider order (drag a row; arrow keys on its grip) and each provider's widget switch. */
export function WidgetPage({ settings, update }: Props) {
  const light = usePanelTheme() === "light";
  const order = settings.providerOrder;
  const rows = useRef<(HTMLLIElement | null)[]>([]);
  const drag = useRef<{ id: WidgetProvider; pointerId: number } | null>(null);
  // `gap` is where the row would land: before row `gap`, or after the last row at order.length.
  const [dragging, setDragging] = useState<{ id: WidgetProvider; gap: number } | null>(null);
  const reorder = (from: number, to: number) => {
    if (from === to) return;
    void update({ providerOrder: moveItem(order, from, to) });
  };
  const dropGap = (y: number) => {
    for (let i = 0; i < order.length; i++) {
      const r = rows.current[i]?.getBoundingClientRect();
      if (r && y < r.top + r.height / 2) return i;
    }
    return order.length;
  };
  const onPointerDown = (e: PointerEvent<HTMLLIElement>, id: WidgetProvider) => {
    // The whole row drags, except its switch.
    if (e.button !== 0 || (e.target as HTMLElement).closest("button")) return;
    drag.current = { id, pointerId: e.pointerId };
    e.currentTarget.setPointerCapture(e.pointerId);
    setDragging({ id, gap: order.indexOf(id) });
  };
  const onPointerMove = (e: PointerEvent<HTMLLIElement>) => {
    const d = drag.current;
    if (!d || d.pointerId !== e.pointerId) return;
    const gap = dropGap(e.clientY);
    setDragging((prev) => (prev && prev.gap !== gap ? { ...prev, gap } : prev));
  };
  const onPointerUp = (e: PointerEvent<HTMLLIElement>) => {
    const d = drag.current;
    if (!d || d.pointerId !== e.pointerId) return;
    drag.current = null;
    setDragging(null);
    const from = order.indexOf(d.id);
    const gap = dropGap(e.clientY);
    reorder(from, gap > from ? gap - 1 : gap);
  };
  const from = dragging ? order.indexOf(dragging.id) : -1;
  const lineAt = dragging && dragging.gap !== from && dragging.gap !== from + 1 ? dragging.gap : -1;
  const insertionLine = (edge: "top" | "bottom") => (
    <span
      aria-hidden="true"
      className={`pointer-events-none absolute -inset-x-px h-0.5 rounded-full bg-(--p-accent) ${
        edge === "top" ? "-top-[3px]" : "-bottom-[3px]"
      }`}
    />
  );

  return (
    <Page title="Widget">
      <Section title="Providers">
        <ol className="flex flex-col gap-1">
          {order.map((id, i) => {
            const name = PAGE_TITLES[id];
            const key = ENABLED_KEY[id];
            const lifted = dragging?.id === id;
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
                className={`ud-row relative flex min-h-[52px] touch-none select-none items-center gap-3 rounded border border-(--s-cardBorder) bg-(--s-card) px-3 py-2 ${
                  lifted
                    ? "z-10 cursor-grabbing opacity-60 shadow-lg"
                    : dragging
                      ? "cursor-grabbing"
                      : "cursor-grab hover:bg-[color-mix(in_srgb,var(--s-card),var(--p-text)_5%)]"
                }`}
              >
                {lineAt === i && insertionLine("top")}
                {lineAt === order.length && i === order.length - 1 && insertionLine("bottom")}
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
                        <circle cx="2.5" cy={y} r="1.6" />
                        <circle cx="7.5" cy={y} r="1.6" />
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
      <HiddenUntilReset settings={settings} update={update} />
    </Page>
  );
}

/** A snoozed card's provider and account from its settings key ("cursor:dev@example.com"). */
export function snoozedEntries(
  settings: Pick<Settings, "snoozedUntil" | "displayNames">,
  now: number,
): { key: string; provider: WidgetProvider; name: string; until: number }[] {
  return Object.entries(settings.snoozedUntil)
    .filter(([, until]) => until > now)
    .flatMap(([key, until]) => {
      const split = key.indexOf(":");
      const provider = key.slice(0, split) as WidgetProvider;
      const id = key.slice(split + 1);
      if (split < 0 || !(provider in PROVIDER_ICONS)) return [];
      // OpenCode nicknames are stored under the bare email, the others under the full key.
      const name = settings.displayNames[key] ?? settings.displayNames[id] ?? id;
      return [{ key, provider, name, until }];
    })
    .sort((a, b) => a.until - b.until);
}

/** Used-up cards the user hid from the widget until they reset, each with a way back. */
function HiddenUntilReset({ settings, update }: Props) {
  const light = usePanelTheme() === "light";
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(id);
  }, []);
  const hidden = snoozedEntries(settings, now);
  return (
    <Section title="Hidden until reset">
      {hidden.length === 0 ? (
        <p className="text-xs text-(--p-muted)">
          No cards are hidden. A used-up card has an eye button on the widget that hides it until it
          resets.
        </p>
      ) : (
        hidden.map((card) => (
          <SettingRow
            key={card.key}
            title={card.name}
            description={`${PAGE_TITLES[card.provider]} · back in ${formatResetDuration(card.until - now)}`}
            icon={
              <img
                src={PROVIDER_ICONS[card.provider]}
                alt=""
                className={`size-5 rounded ${card.provider === "codex" && light ? "invert" : ""}`}
              />
            }
            control={(labelId) => (
              <button
                type="button"
                aria-describedby={labelId}
                onClick={() =>
                  void update(
                    { snoozedUntil: { [card.key]: null } },
                    `${card.name} is back on the widget`,
                  )
                }
                className={secondaryButton}
              >
                Show
              </button>
            )}
          />
        ))
      )}
    </Section>
  );
}

function openAboutLink(link: "logs" | "repo") {
  if (!inTauri()) return;
  invoke("open_about_link", { link }).catch((err) => console.error("open link failed", err));
}
