import { useEffect, useState, type ReactNode } from "react";
import { CheckIcon, DownloadIcon, RefreshIcon } from "@/ui/Icons";
import {
  checkForUpdates,
  dismissUpdate,
  installUpdate,
  type UpdatePhase,
  type UpdateStatus,
} from "@/lib/updates";

/** How long a check's result stays up before the banner hides itself. */
export const AUTO_HIDE_MS: Partial<Record<UpdatePhase["state"], number>> = {
  upToDate: 4_000,
  failed: 8_000,
};

/** What the banner says for `status`, or null for none: idle, or an update the user put off. */
export function bannerText(status: UpdateStatus | null, restarting = false): string | null {
  switch (status?.state) {
    case undefined:
    case "idle":
      return null;
    case "checking":
      return "Checking for updates…";
    case "downloading":
      return `Downloading ${status.version}…`;
    case "upToDate":
      return `You're up to date · v${status.currentVersion}`;
    case "failed":
      return "Couldn't check for updates";
    case "ready":
      if (status.dismissed) return null;
      return restarting ? "Restarting…" : `Update ${status.version} is ready`;
  }
}

const textButton =
  "shrink-0 rounded px-1 py-1 text-(--w-muted) hover:text-(--w-text) disabled:opacity-60";
/** the offline banner's Retry (App.tsx) */
const linkButton =
  "shrink-0 rounded px-1 py-1 font-semibold underline underline-offset-2 hover:opacity-80";
const primaryButton =
  "shrink-0 rounded-md bg-(--w-text) px-1.5 py-1 font-semibold text-(--w-card) hover:opacity-85 disabled:opacity-60";

/** One line at the top of the widget: an update to restart into, or the result of a check. */
export function UpdateBanner({ status }: { status: UpdateStatus | null }) {
  /** The status whose result already timed out (each change is a new object). */
  const [expired, setExpired] = useState<UpdateStatus | null>(null);
  const [restarting, setRestarting] = useState(false);

  useEffect(() => {
    const ms = status && AUTO_HIDE_MS[status.state];
    if (!ms) return;
    const id = setTimeout(() => setExpired(status), ms);
    return () => clearTimeout(id);
  }, [status]);

  const text = status === expired ? null : bannerText(status, restarting);
  // A live region that is already there announces the text put into it; one mounted together
  // with its text often stays silent.
  const live = (
    <span role="status" className="sr-only">
      {text ?? ""}
    </span>
  );
  if (!status || text == null) return live;

  const restart = () => {
    setRestarting(true);
    installUpdate().catch(() => setRestarting(false));
  };

  let icon: ReactNode = (
    <RefreshIcon size={12} className="animate-spin motion-reduce:animate-none" />
  );
  let actions: ReactNode = null;
  if (status.state === "downloading") icon = <DownloadIcon size={12} />;
  if (status.state === "upToDate") icon = <CheckIcon size={12} className="text-(--w-ringOk)" />;
  if (status.state === "failed") {
    icon = <RefreshIcon size={12} className="text-(--w-barDanger)" />;
    actions = (
      <button type="button" onClick={checkForUpdates} className={linkButton}>
        Retry
      </button>
    );
  }
  if (status.state === "ready") {
    // no icon: at 238 px the text and both buttons need the whole line
    icon = null;
    actions = (
      <>
        <button type="button" onClick={restart} disabled={restarting} className={primaryButton}>
          Restart
        </button>
        <button type="button" onClick={dismissUpdate} disabled={restarting} className={textButton}>
          Later
        </button>
      </>
    );
  }

  return (
    <>
      {live}
      <div className="flex min-h-7 items-center gap-1 rounded-lg border border-(--w-cardBorder) bg-(--w-card) py-0.5 pr-0.5 pl-2 text-[11px] leading-4 text-(--w-text)">
        {icon && <span className="mr-0.5 flex shrink-0 text-(--w-muted)">{icon}</span>}
        {/* the live region above is what assistive tech reads */}
        <span
          aria-hidden="true"
          title={status.state === "failed" ? status.message : text}
          className="min-w-0 flex-1 truncate"
        >
          {text}
        </span>
        {actions}
      </div>
    </>
  );
}
