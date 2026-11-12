import { useState, type ReactNode } from "react";
import type { WidgetProvider } from "@/lib/types";
import {
  checkForUpdates,
  installUpdate,
  isBusy,
  useUpdateStatus,
  type UpdateStatus,
} from "@/lib/updates";
import { DownloadIcon, RefreshIcon } from "@/ui/Icons";
import { PAGE_TITLES, PROVIDER_ICONS, type PageId } from "../_lib/pages";

function NavItem({
  page,
  current,
  onSelect,
  icon,
  indent,
  badge,
}: {
  page: PageId;
  current: PageId;
  onSelect: (page: PageId) => void;
  icon?: ReactNode;
  indent?: boolean;
  /** a dot after the title; the text is what assistive tech reads for it */
  badge?: string;
}) {
  const selected = page === current;
  return (
    <li>
      <button
        type="button"
        aria-current={selected ? "page" : undefined}
        onClick={() => onSelect(page)}
        className={`ud-nav-item relative flex min-h-9 w-full items-center gap-3 rounded px-3 text-left text-sm hover:bg-(--s-navHover) ${
          indent ? "pl-9" : ""
        } ${selected ? "bg-(--s-navActive) font-semibold" : ""}`}
      >
        {selected && (
          <span
            aria-hidden="true"
            className="ud-nav-pill absolute top-1/2 left-0 h-4 w-[3px] -translate-y-1/2 rounded-full bg-(--p-accent)"
          />
        )}
        {icon}
        <span className="truncate">{PAGE_TITLES[page]}</span>
        {badge && (
          <span
            role="img"
            aria-label={badge}
            className="ml-auto size-2 shrink-0 rounded-full bg-(--p-accent)"
          />
        )}
      </button>
    </li>
  );
}

/** Bottom of the nav: starts a check, or restarts into a downloaded update. The result shows on About. */
function UpdateButton({ status }: { status: UpdateStatus | null }) {
  const [restarting, setRestarting] = useState(false);
  const ready = status?.state === "ready";
  const restart = () => {
    setRestarting(true);
    installUpdate().catch(() => setRestarting(false));
  };
  let label = "Check for updates";
  if (ready) label = restarting ? "Restarting…" : "Restart to update";
  else if (isBusy(status)) label = "Checking…";
  return (
    <li>
      <button
        type="button"
        disabled={status === null || isBusy(status) || restarting}
        onClick={ready ? restart : checkForUpdates}
        className="ud-nav-item flex min-h-9 w-full items-center gap-3 rounded px-3 text-left text-sm hover:bg-(--s-navHover) disabled:cursor-default disabled:text-(--p-muted) disabled:hover:bg-transparent"
      >
        {ready ? (
          <DownloadIcon size={16} className="shrink-0" />
        ) : (
          <RefreshIcon size={16} className="shrink-0" />
        )}
        <span className="truncate">{label}</span>
      </button>
    </li>
  );
}

/** Left navigation: General, Widget, Accounts (one item per provider),
 * Check for updates, About. */
export function Nav({
  current,
  onSelect,
  providerOrder,
  light,
}: {
  current: PageId;
  onSelect: (page: PageId) => void;
  providerOrder: readonly WidgetProvider[];
  light: boolean;
}) {
  const updateStatus = useUpdateStatus();
  const updateReady = updateStatus?.state === "ready";
  const item = (page: PageId, icon?: ReactNode, indent?: boolean) => (
    <NavItem
      key={page}
      page={page}
      current={current}
      onSelect={onSelect}
      icon={icon}
      indent={indent}
      badge={page === "about" && updateReady ? "Update ready" : undefined}
    />
  );
  return (
    <nav
      aria-label="Settings"
      className="flex w-[232px] shrink-0 flex-col overflow-y-auto bg-(--s-page) px-2 pt-3 pb-2"
    >
      <ul className="flex flex-col gap-0.5 p-0.5">
        {item("general")}
        {item("widget")}
      </ul>
      <h2 id="nav-accounts" className="mt-4 mb-1 px-3 text-xs font-semibold text-(--p-muted)">
        Accounts
      </h2>
      <ul aria-labelledby="nav-accounts" className="flex flex-col gap-0.5 p-0.5">
        {providerOrder.map((id) =>
          item(
            id,
            <img
              src={PROVIDER_ICONS[id]}
              alt=""
              draggable={false}
              className={`size-4 shrink-0 rounded-[3px] ${id === "codex" && light ? "invert" : ""}`}
            />,
          ),
        )}
      </ul>
      <ul className="mt-auto flex flex-col gap-0.5 p-0.5 pt-4">
        <UpdateButton status={updateStatus} />
        {item("about")}
      </ul>
    </nav>
  );
}
