import { invoke } from "@tauri-apps/api/core";
import { toastError } from "@/components/toast/toast";
import { inTauri } from "./bridge";
import { mockOpenCursor } from "./mock";

/**
 * Start Cursor so it renews its own sign-in (the Cursor card's "Open Cursor" button). Rust finds
 * and starts Cursor.exe; UsageDeck never refreshes Cursor's token itself. True when it started.
 */
export async function openCursor(): Promise<boolean> {
  try {
    if (inTauri()) await invoke("open_cursor");
    else mockOpenCursor();
    return true;
  } catch (err) {
    toastError(typeof err === "string" ? err : "Could not open Cursor");
    return false;
  }
}
