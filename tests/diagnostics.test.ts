import { describe, expect, it } from "vitest";
import { cursorRows, type CursorAttempt } from "@/lib/diagnostics";
import { CURSOR_PERSISTENT_FAILURES, cursorErrorHint } from "@/lib/errors";
import type { ProviderErrorKind } from "@/lib/types";
import { isBusy, lastCheckLine, statusChanged, type UpdateStatus } from "@/lib/updates";
import { snoozedEntries } from "@/settings/_components/SettingsPages";

const at = (ms: number) => `t${ms}`;
const attempt = (patch: Partial<CursorAttempt> = {}): CursorAttempt => ({
  atMs: 100,
  db: { how: "opened", detail: null, readAtMs: 100, reusedEarlier: false },
  token: { present: true, form: "jwt", decrypt: null, id: "1a2b3c4d", expiresAtMs: 900 },
  requests: [],
  authPath: "api2",
  parse: "ok",
  outcome: "ok",
  failures: 0,
  lastOkMs: 100,
  ...patch,
});
const row = (rows: ReturnType<typeof cursorRows>, label: string) =>
  rows.find((r) => r.label === label);

describe("Cursor card error hint", () => {
  const note = (failures: number) => ({ reason: "api2.cursor.sh replied 503 at 14:02", failures });

  it("names a lasting failure and explains it with the attempt's own reason", () => {
    for (const kind of ["sign-in-unreadable", "schema-changed", "http"] as ProviderErrorKind[]) {
      const hint = cursorErrorHint(kind, note(1));
      expect(hint?.short).toBeTruthy();
      expect(hint?.long).toBe("api2.cursor.sh replied 503 at 14:02");
    }
    expect(cursorErrorHint("sign-in-unreadable", null)?.short).toBe("can't read sign-in");
    expect(cursorErrorHint("http", null)?.long).toBe("Cursor's server answered with an error");
  });

  it("waits out a passing failure before naming it", () => {
    for (const kind of ["network", "rate-limited", "locked"] as ProviderErrorKind[]) {
      expect(cursorErrorHint(kind, note(CURSOR_PERSISTENT_FAILURES - 1))).toBeNull();
      expect(cursorErrorHint(kind, null)).toBeNull();
      expect(cursorErrorHint(kind, note(CURSOR_PERSISTENT_FAILURES))?.short).toBe("not updating");
    }
  });

  it("leaves a rejected sign-in to the Open Cursor button and shows nothing when all is well", () => {
    expect(cursorErrorHint("auth-rejected", note(9))).toBeNull();
    expect(cursorErrorHint(null, note(0))).toBeNull();
    expect(cursorErrorHint(undefined, null)).toBeNull();
  });
});

describe("Cursor diagnostics rows", () => {
  it("shows a working attempt without warnings", () => {
    const rows = cursorRows(attempt(), null, at);
    expect(rows.every((r) => !r.bad)).toBe(true);
    expect(row(rows, "Last attempt")?.value).toBe("t100 · worked");
    expect(row(rows, "Token")?.value).toBe("#1a2b3c4d · valid until t900");
    expect(row(rows, "Usage from")?.value).toBe("api2.cursor.sh (bearer token) · parse: ok");
  });

  it("marks the step that failed: a sealed token that could not be decrypted", () => {
    const rows = cursorRows(
      attempt({
        outcome: "sign-in-unreadable",
        failures: 4,
        lastOkMs: null,
        authPath: null,
        parse: null,
        token: {
          present: true,
          form: "safe-storage-v1",
          decrypt: "Local State not found",
          id: null,
          expiresAtMs: null,
        },
      }),
      "Cursor's sign-in could not be read",
      at,
    );
    expect(row(rows, "Reason")?.value).toBe("Cursor's sign-in could not be read");
    const signIn = row(rows, "Sign-in");
    expect(signIn?.value).toBe(
      "Token encrypted (cursor-safe-storage:v1) · decrypt: Local State not found",
    );
    expect(signIn?.bad).toBe(true);
    expect(row(rows, "Token")).toBeUndefined();
    expect(row(rows, "Last success")?.value).toBe(
      "None since UsageDeck started · 4 failed in a row",
    );
  });

  it("lists every request, a busy file read from a copy, and the fallback that answered", () => {
    const rows = cursorRows(
      attempt({
        db: { how: "copied", detail: "database is locked", readAtMs: 100, reusedEarlier: false },
        requests: [
          {
            method: "POST",
            endpoint: "api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage",
            status: 401,
            detail: "unauthenticated: Not logged in",
          },
          { method: "GET", endpoint: "cursor.com/api/usage-summary", status: 200, detail: null },
          { method: "GET", endpoint: "cursor.com/other", status: null, detail: "timed out" },
        ],
        authPath: "cursor.com",
        token: { present: true, form: "jwt", decrypt: null, id: "1a2b3c4d", expiresAtMs: 50 },
      }),
      null,
      at,
    );
    expect(row(rows, "state.vscdb")?.value).toBe(
      "Found · busy, read from a private copy (database is locked)",
    );
    const rejected = row(
      rows,
      "POST api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage",
    );
    expect(rejected).toEqual({
      label: "POST api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage",
      value: "401 · unauthenticated: Not logged in",
      bad: true,
    });
    expect(row(rows, "GET cursor.com/api/usage-summary")?.bad).toBe(false);
    expect(row(rows, "GET cursor.com/other")?.value).toBe("no reply · timed out");
    expect(row(rows, "Token")).toEqual({
      label: "Token",
      value: "#1a2b3c4d · expired t50",
      bad: true,
    });
    expect(row(rows, "Usage from")?.value).toBe("cursor.com (session cookie) · parse: ok");
  });
});

describe("update status store rules", () => {
  const status = (patch: Partial<UpdateStatus> = {}): UpdateStatus =>
    ({ state: "idle", lastChecked: null, currentVersion: "1.4.4", ...patch }) as UpdateStatus;

  it("keeps reading while a check runs", () => {
    expect(isBusy(status({ state: "checking" }))).toBe(true);
    expect(isBusy(status({ state: "downloading", version: "1.5.0" } as never))).toBe(true);
    expect(isBusy(status({ state: "upToDate" }))).toBe(false);
    expect(isBusy(null)).toBe(false);
  });

  it("publishes a status only when it says something new", () => {
    expect(statusChanged(null, status())).toBe(true);
    expect(statusChanged(status(), status())).toBe(false);
    expect(statusChanged(status(), status({ state: "checking" }))).toBe(true);
    expect(statusChanged(status(), status({ lastChecked: 5 }))).toBe(true);
  });

  it("puts the last check in words, also a silent background one", () => {
    expect(lastCheckLine(status(), at)).toBeNull();
    expect(
      lastCheckLine(
        status({
          lastChecked: 7,
          lastCheck: { trigger: "background", result: "up to date", latestVersion: "1.4.4" },
        }),
        at,
      ),
    ).toBe("Last check t7 (background): up to date · newest release 1.4.4");
    expect(
      lastCheckLine(
        status({
          lastChecked: 9,
          lastCheck: { trigger: "manual", result: "failed: HTTP 403", latestVersion: null },
        }),
        at,
      ),
    ).toBe("Last check t9 (manual): failed: HTTP 403");
  });
});

describe("hidden until reset list", () => {
  it("lists the cards still hidden, soonest back first, by nickname", () => {
    const entries = snoozedEntries(
      {
        snoozedUntil: {
          "opencode:main@example.com": 900,
          "cursor:dev@example.com": 500,
          "codex:old@example.com": 50,
          nonsense: 999,
        },
        displayNames: { "main@example.com": "Main" },
      },
      100,
    );
    expect(entries).toEqual([
      { key: "cursor:dev@example.com", provider: "cursor", name: "dev@example.com", until: 500 },
      { key: "opencode:main@example.com", provider: "opencode", name: "Main", until: 900 },
    ]);
  });
});
