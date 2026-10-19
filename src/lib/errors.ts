// Card wording for a failed fetch. Pure, so it is unit-tested (tests/widget-logic.test.ts).
import type { ProviderErrorKind } from "./types";

export interface ErrorHint {
  /** compact text for the card header (the widget is 238 px wide) */
  short: string;
  /** tooltip with the longer explanation */
  long: string;
}

/**
 * The header hint for an error kind. `how` says how to sign in again, e.g. "open Cursor".
 * Returns null for no error and for `auth-rejected`, which keeps its own re-auth / key-rejected
 * hint; a missing field from an older backend counts as no error.
 */
export function errorHint(
  kind: ProviderErrorKind | null | undefined,
  how = "sign in again",
): ErrorHint | null {
  switch (kind) {
    case "rate-limited":
      return {
        short: "rate limited · retrying",
        long: "The service is rate limiting requests · showing the last known usage and retrying automatically",
      };
    case "schema-changed":
      return {
        short: "reply format changed",
        long: "The service changed its reply format, so usage could not be read · an app update is needed",
      };
    case "network":
      return {
        short: "offline",
        long: "Could not reach the service · check the network connection; retrying automatically",
      };
    case "http":
      return {
        short: "service error",
        long: "The service answered with an error · retrying on the next refresh",
      };
    case "locked":
      return {
        short: "busy",
        long: "The sign-in data is in use by another program · retrying shortly",
      };
    case "missing-credentials":
      return { short: "not signed in", long: `No sign-in found · ${how} to sign in` };
    default:
      return null;
  }
}

/** True when the account's sign-in or key was rejected (old `authExpired` flag or the new kind). */
export function isAuthRejected(
  authExpired: boolean | undefined,
  kind: ProviderErrorKind | null | undefined,
) {
  return !!authExpired || kind === "auth-rejected";
}
