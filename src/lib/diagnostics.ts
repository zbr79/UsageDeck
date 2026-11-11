// The in-app diagnostics (src-tauri/src/diagnostics.rs): what the last Cursor attempt and the last
// update check did. Outside Tauri (plain browser dev) a mock answers instead; see mockDiagnostics.
import { invoke } from "@tauri-apps/api/core";
import { inTauri } from "./bridge";
import { mockDiagnostics } from "./mock";
import type { UpdateStatus } from "./updates";

/** `CursorAttempt` in src-tauri/src/providers/cursor_diag.rs. It never holds the token. */
export interface CursorAttempt {
  atMs: number;
  db: {
    how: "missing" | "opened" | "copied" | "locked" | "failed";
    detail: string | null;
    readAtMs: number;
    reusedEarlier: boolean;
  };
  token: {
    present: boolean;
    /** "jwt", "opaque", "safe-storage-v1" (encrypted) or an unknown scheme's name */
    form: string | null;
    /** for an encrypted token: "ok", or why it could not be decrypted */
    decrypt: string | null;
    /** first 8 hex characters of the token's SHA-256 */
    id: string | null;
    expiresAtMs: number | null;
  };
  requests: { method: string; endpoint: string; status: number | null; detail: string | null }[];
  authPath: "api2" | "cursor.com" | null;
  parse: string | null;
  /** "ok", "not-signed-in" or the error kind on the card */
  outcome: string;
  failures: number;
  lastOkMs: number | null;
}

export interface Diagnostics {
  appVersion: string;
  system: string;
  /** null: nothing tried yet since UsageDeck started */
  cursor: CursorAttempt | null;
  cursorReason: string | null;
  updater: UpdateStatus;
  updaterReport: string;
  /** everything as text; the same text "Copy diagnostics" puts on the clipboard */
  report: string;
}

export function getDiagnostics(): Promise<Diagnostics> {
  return inTauri() ? invoke<Diagnostics>("get_diagnostics") : mockDiagnostics.get();
}

/** Copies the full report. Rust writes the clipboard, so the page needs no clipboard access. */
export function copyDiagnostics(): Promise<void> {
  return inTauri() ? invoke("copy_diagnostics") : mockDiagnostics.copy();
}

const DB: Record<CursorAttempt["db"]["how"], string> = {
  missing: "Not found",
  opened: "Found · opened read-only",
  copied: "Found · busy, read from a private copy",
  locked: "Found · locked",
  failed: "Found · could not be read",
};
const FORM: Record<string, string> = {
  jwt: "plain token",
  opaque: "plain token (not a JWT)",
  "safe-storage-v1": "encrypted (cursor-safe-storage:v1)",
};
const PATH = { api2: "api2.cursor.sh (bearer token)", "cursor.com": "cursor.com (session cookie)" };

export interface DiagnosticRow {
  label: string;
  value: string;
  /** the step failed: shown in the warning colour */
  bad?: boolean;
}

/** The Cursor record as label / value rows. `at` formats a time (ms) for the reader. */
export function cursorRows(
  attempt: CursorAttempt,
  reason: string | null,
  at: (ms: number) => string,
): DiagnosticRow[] {
  const ok = attempt.outcome === "ok";
  const rows: DiagnosticRow[] = [
    {
      label: "Last attempt",
      value: `${at(attempt.atMs)} · ${ok ? "worked" : attempt.outcome}`,
      bad: !ok,
    },
  ];
  if (reason) rows.push({ label: "Reason", value: reason, bad: true });
  rows.push({
    label: "Last success",
    value:
      attempt.lastOkMs == null
        ? `None since UsageDeck started · ${attempt.failures} failed in a row`
        : at(attempt.lastOkMs),
    bad: attempt.lastOkMs == null && attempt.failures > 0,
  });
  const db = attempt.db;
  rows.push({
    label: "state.vscdb",
    value:
      DB[db.how] +
      (db.detail ? ` (${db.detail})` : "") +
      (db.reusedEarlier ? ` · using the sign-in read at ${at(db.readAtMs)}` : ""),
    bad: db.how === "locked" || db.how === "failed",
  });
  const token = attempt.token;
  if (!token.present) {
    rows.push({ label: "Sign-in", value: "No token stored", bad: db.how === "opened" });
  } else {
    const form = FORM[token.form ?? ""] ?? token.form ?? "unknown format";
    const decrypt = token.decrypt ? ` · decrypt: ${token.decrypt}` : "";
    rows.push({
      label: "Sign-in",
      value: `Token ${form}${decrypt}`,
      bad: token.decrypt != null && token.decrypt !== "ok",
    });
    if (token.id) {
      const expired = token.expiresAtMs != null && token.expiresAtMs <= attempt.atMs;
      const expiry =
        token.expiresAtMs == null
          ? "no expiry"
          : `${expired ? "expired" : "valid until"} ${at(token.expiresAtMs)}`;
      rows.push({ label: "Token", value: `#${token.id} · ${expiry}`, bad: expired });
    }
  }
  for (const r of attempt.requests) {
    const good = r.status != null && r.status >= 200 && r.status < 300;
    rows.push({
      label: `${r.method} ${r.endpoint}`,
      value: (r.status ?? "no reply") + (r.detail ? ` · ${r.detail}` : ""),
      bad: !good,
    });
  }
  rows.push({
    label: "Usage from",
    value: attempt.authPath
      ? `${PATH[attempt.authPath]} · parse: ${attempt.parse ?? "?"}`
      : `Nothing${attempt.parse ? ` · parse: ${attempt.parse}` : ""}`,
    bad: !attempt.authPath && attempt.requests.length > 0,
  });
  return rows;
}
