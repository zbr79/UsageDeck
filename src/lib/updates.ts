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
  | { state: "ready"; version: string };

/** The last finished check, kept whether or not the widget showed anything for it. */
export interface LastCheck {
  trigger: "manual" | "background";
  /** "up to date", "update 1.5.0 downloaded" or "failed: <why>" */
  result: string;
  /** the newest release GitHub named, when it answered */
  latestVersion: string | null;
}

export type UpdateStatus = UpdatePhase & {
  /** when the last check finished, ms since the epoch */
  lastChecked: number | null;
  currentVersion: string;
  lastCheck?: LastCheck;
};

export function getUpdateStatus(): Promise<UpdateStatus> {
  return inTauri() ? invoke<UpdateStatus>("get_update_status") : mockUpdates.status();
}

// ---- the status every component shares ----------------------------------------------------
//
// The status reaches the page three ways, so no single one is a point of failure: the
// `update-status` event, the value `check_for_updates` returns, and a poll while a check runs.

/** How often the status is read while a check runs, and after a failed first read. */
export const WATCH_MS = 1_000;
const FIRST_READ_TRIES = 5;

let current: UpdateStatus | null = null;
const listeners = new Set<(status: UpdateStatus) => void>();
let started = false;
let watch: ReturnType<typeof setInterval> | null = null;

/** A check is running: keep reading the status until it ends. */
export function isBusy(status: UpdateStatus | null): boolean {
  return status?.state === "checking" || status?.state === "downloading";
}

/** True when `next` says something `prev` does not. */
export function statusChanged(prev: UpdateStatus | null, next: UpdateStatus): boolean {
  return JSON.stringify(prev) !== JSON.stringify(next);
}

function publish(next: UpdateStatus) {
  if (!statusChanged(current, next)) return;
  current = next;
  for (const cb of listeners) cb(next);
  if (isBusy(next)) startWatch();
}

function read(): Promise<void> {
  return getUpdateStatus().then(publish);
}

function startWatch() {
  if (watch) return;
  watch = setInterval(() => {
    if (!isBusy(current)) {
      if (watch) clearInterval(watch);
      watch = null;
      return;
    }
    read().catch((err) => console.error("get_update_status failed", err));
  }, WATCH_MS);
}

function start() {
  if (started) return;
  started = true;
  onUpdateStatus(publish);
  let tries = 0;
  const first = () => {
    read().catch((err) => {
      console.error("get_update_status failed", err);
      if (++tries < FIRST_READ_TRIES) setTimeout(first, WATCH_MS);
    });
  };
  first();
}

/** Starts a check. The status it starts with comes straight back and is shown at once. */
export function checkForUpdates(): void {
  start();
  const done: Promise<UpdateStatus | void> = inTauri()
    ? invoke<UpdateStatus>("check_for_updates")
    : mockUpdates.check();
  done.then(
    (status) => {
      if (status) publish(status);
      startWatch();
    },
    (err) => console.error("check_for_updates failed", err),
  );
}

/** Rejects when no update is ready or the installer cannot start; on success the app exits. */
export function installUpdate(): Promise<void> {
  return inTauri() ? invoke("install_update") : mockUpdates.install();
}

function onUpdateStatus(cb: (status: UpdateStatus) => void): () => void {
  return inTauri() ? onTauriUpdateStatus(cb) : mockUpdates.subscribe(cb);
}

/** The live update status; null until the first answer. Each change is a new object. */
export function useUpdateStatus(): UpdateStatus | null {
  const [status, setStatus] = useState<UpdateStatus | null>(current);
  useEffect(() => {
    listeners.add(setStatus);
    start();
    if (current) setStatus(current);
    return () => void listeners.delete(setStatus);
  }, []);
  return status;
}

/** The last check in words, for Settings → General → Updates; null before the first check. */
export function lastCheckLine(status: UpdateStatus, at: (ms: number) => string): string | null {
  const check = status.lastCheck;
  if (!check || status.lastChecked == null) return null;
  const latest = check.latestVersion ? ` · newest release ${check.latestVersion}` : "";
  return `Last check ${at(status.lastChecked)} (${check.trigger}): ${check.result}${latest}`;
}
