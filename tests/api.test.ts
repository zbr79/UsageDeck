// api.ts outside Tauri answers from the browser mock; the error rules match
// the old lib/api-fetch.ts (non-2xx -> ApiError with the body's `error`).
import { describe, expect, it, vi } from "vitest";

describe("api (browser mock)", () => {
  it("returns the JSON body on success", async () => {
    const { getJson } = await import("@/lib/api");
    const r = await getJson<{ settings: { providerOrder: string[] } }>("/api/settings");
    expect(r.settings.providerOrder[0]).toBe("opencode");
  });

  it("throws ApiError with the server's message on failure", async () => {
    const { getJson, ApiError } = await import("@/lib/api");
    const err = await getJson<never>("/api/nope").catch((e) => e as InstanceType<typeof ApiError>);
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(404);
    expect(err.message).toBe("not found");
  });

  it("uses the Tauri api command when running inside Tauri", async () => {
    vi.resetModules();
    const invoke = vi.fn(async () => ({ status: 500, body: null }));
    vi.doMock("@tauri-apps/api/core", () => ({ invoke }));
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    const { sendJson, ApiError } = await import("@/lib/api");
    const err = await sendJson<never>("/api/settings", "POST", { cursorEnabled: false }).catch(
      (e) => e as InstanceType<typeof ApiError>,
    );
    expect(invoke).toHaveBeenCalledWith("api", {
      method: "POST",
      path: "/api/settings",
      body: { cursorEnabled: false },
    });
    expect(err).toBeInstanceOf(ApiError);
    expect(err.message).toBe("request failed (500)");
    vi.doUnmock("@tauri-apps/api/core");
  });
});
