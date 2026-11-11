// Card wording for a failed fetch. Pure, so it is unit-tested (tests/provider-errors.test.ts).
import type { CursorAttemptNote, ProviderErrorKind } from "./types";

export interface ErrorHint {
  /** compact text for the card header (the widget is 238 px wide) */
  short: string;
  /** tooltip with the longer explanation */
  long: string;
}

/**
 * The header hint for an error kind. `how` says how to sign in again, e.g. "open Cursor".
 * A card only names an error the user must act on: the transient kinds (rate-limited, network,
 * http, locked, schema-changed) show nothing, and the card keeps its last known usage exactly
 * like fresh data. Returns null for those, for no error, and for `auth-rejected`, which keeps
 * its own re-auth / key-rejected hint; a missing field from an older backend counts as no error.
 */
export function errorHint(
  kind: ProviderErrorKind | null | undefined,
  how = "sign in again",
): ErrorHint | null {
  switch (kind) {
    case "missing-credentials":
      return { short: "not signed in", long: `No sign-in found · ${how} to sign in` };
    default:
      return null;
  }
}

/**
 * The error kind a card should name for an account. A missing sign-in is only actionable for the
 * account the app reads (the signed-in one); a remembered account that is not signed in is
 * normal and shows its cached usage without a hint.
 */
export function cardErrorKind(
  kind: ProviderErrorKind | null | undefined,
  active: boolean,
): ProviderErrorKind | null {
  return kind === "missing-credentials" && !active ? null : (kind ?? null);
}

/** True when the account's sign-in or key was rejected (old `authExpired` flag or the new kind). */
export function isAuthRejected(
  authExpired: boolean | undefined,
  kind: ProviderErrorKind | null | undefined,
) {
  return !!authExpired || kind === "auth-rejected";
}

/** A passing failure is named on the Cursor card only once it has lasted this many attempts. */
export const CURSOR_PERSISTENT_FAILURES = 3;

const CURSOR_HINTS: Partial<Record<ProviderErrorKind, ErrorHint>> = {
  "missing-credentials": {
    short: "not signed in",
    long: "No sign-in found · open Cursor to sign in",
  },
  "sign-in-unreadable": {
    short: "can't read sign-in",
    long: "Cursor's sign-in could not be read",
  },
  "schema-changed": { short: "not updating", long: "Cursor's reply held no usage" },
  http: { short: "not updating", long: "Cursor's server answered with an error" },
  network: { short: "not updating", long: "Cursor's server could not be reached" },
  "rate-limited": { short: "not updating", long: "Cursor asked UsageDeck to slow down" },
  locked: { short: "not updating", long: "Cursor's sign-in file is locked" },
};
const PASSING: readonly ProviderErrorKind[] = ["network", "rate-limited", "locked"];

/**
 * The Cursor card's header hint for a failed fetch, with the attempt's own reason as the tooltip.
 * Unlike the other cards, Cursor's names every failure, so stale numbers are never unexplained;
 * a failure that usually passes by itself (network, rate limit, locked file) is named once it has
 * lasted `CURSOR_PERSISTENT_FAILURES` attempts. A rejected sign-in keeps its "Open Cursor" button.
 */
export function cursorErrorHint(
  kind: ProviderErrorKind | null | undefined,
  note: CursorAttemptNote | null | undefined,
): ErrorHint | null {
  if (!kind || kind === "auth-rejected") return null;
  const hint = CURSOR_HINTS[kind];
  if (!hint) return null;
  if (PASSING.includes(kind) && (note?.failures ?? 0) < CURSOR_PERSISTENT_FAILURES) return null;
  return { short: hint.short, long: note?.reason ?? hint.long };
}
