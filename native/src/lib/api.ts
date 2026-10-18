// Access to the local API (was lib/api-fetch.ts). Requests go to the Rust
// `api` command, which returns the old route's status and JSON body. Outside
// Tauri (plain browser dev) a mock answers instead.
import { invoke } from "@tauri-apps/api/core";
import { inTauri, onWidgetSettingsChanged } from "./bridge";
import { mockApi } from "./mock";

type Method = "GET" | "POST" | "DELETE";

export interface ApiReply {
  status: number;
  body: unknown;
}

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}

function request(method: Method, path: string, body: unknown): Promise<ApiReply> {
  if (!inTauri()) return mockApi(method, path, body);
  return invoke<ApiReply>("api", { method, path, body });
}

function parse<T>(res: ApiReply): T {
  const ok = res.status >= 200 && res.status < 300;
  const body = (res.body ?? null) as { error?: unknown } | null;
  if (!ok) {
    const message =
      typeof body?.error === "string" ? body.error : `request failed (${res.status})`;
    throw new ApiError(res.status, message);
  }
  return body as T;
}

export async function getJson<T>(path: string): Promise<T> {
  return parse<T>(await request("GET", path, null));
}

export async function sendJson<T>(
  path: string,
  method: "POST" | "DELETE",
  body: unknown,
): Promise<T> {
  return parse<T>(await request(method, path, body ?? null));
}

// Settings changes are broadcast so the widget updates live, not only when
// the settings window closes. Rust also emits `widget-settings-changed`.
const CHANNEL = "usage-monitor:settings";

export function announceSettingsChanged(): void {
  if (typeof BroadcastChannel === "undefined") return;
  const ch = new BroadcastChannel(CHANNEL);
  ch.postMessage("changed");
  ch.close();
}

export function onSettingsChanged(cb: () => void): () => void {
  const offEvent = onWidgetSettingsChanged(cb);
  if (typeof BroadcastChannel === "undefined") return offEvent;
  const ch = new BroadcastChannel(CHANNEL);
  ch.onmessage = () => cb();
  return () => {
    ch.close();
    offEvent();
  };
}
