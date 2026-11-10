// Card wording for a failed fetch. Pure, so it is unit-tested (tests/provider-errors.test.ts).
import type { ProviderErrorKind } from "./types";

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
