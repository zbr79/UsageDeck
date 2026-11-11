// The self-updater's status (src-tauri/src/updater/status.rs) and its commands. Outside Tauri
// (plain browser dev) a mock answers instead; see mockUpdates in ./mock.
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { inTauri, onUpdateStatus as onTauriUpdateStatus } from "./bridge";
import { mockUpdates } from "./mock";

/**
 * Only a check the user asked for is ever `checking`, `downloading`, `upToDate` or `failed`; a
 * background check stays `idle` until an update is `ready`.
 */
export type UpdatePhase =
  | { state: "idle" }
  | { state: "checking" }
  | { state: "downloading"; version: string }
  | { state: "upToDate" }
  | { state: "failed"; message: string }
  /** `dismissed`: the user chose "Later"; the widget hides its banner, settings still offers it */
  | { state: "ready"; version: string; dismissed: boolean };

export type UpdateStatus = UpdatePhase & {
  /** when the last check finished, ms since the epoch */
  lastChecked: number | null;
  currentVersion: string;
};

export function getUpdateStatus(): Promise<UpdateStatus> {
  return inTauri() ? invoke<UpdateStatus>("get_update_status") : mockUpdates.status();
}

/** Results arrive as status changes. */
export function checkForUpdates(): void {
  const done = inTauri() ? invoke("check_for_updates") : mockUpdates.check();
  done.catch((err) => console.error("check_for_updates failed", err));
}

/** Rejects when no update is ready or the installer cannot start; on success the app exits. */
export function installUpdate(): Promise<void> {
  return inTauri() ? invoke("install_update") : mockUpdates.install();
}

/** "Later": hide the widget's banner for the ready version. */
export function dismissUpdate(): void {
  const done = inTauri() ? invoke("dismiss_update") : mockUpdates.dismiss();
  done.catch((err) => console.error("dismiss_update failed", err));
}

function onUpdateStatus(cb: (status: UpdateStatus) => void): () => void {
  return inTauri() ? onTauriUpdateStatus(cb) : mockUpdates.subscribe(cb);
}

/** The live update status; null until the first answer. Each change is a new object. */
export function useUpdateStatus(): UpdateStatus | null {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  useEffect(() => {
    let live = false;
    const off = onUpdateStatus((next) => {
      live = true;
      setStatus(next);
    });
    getUpdateStatus().then(
      (first) => {
        if (!live) setStatus(first);
      },
      (err) => console.error("get_update_status failed", err),
    );
    return off;
  }, []);
  return status;
}
