import { invoke } from "@tauri-apps/api/core";
import { toastError, toastSuccess } from "@/components/toast/toast";
import { inTauri } from "./bridge";
import { mockCopyKey } from "./mock";

/**
 * Copy an OpenCode account's API key to the clipboard (widget card icon and settings card). The
 * Rust `copy_key` command writes the clipboard itself, so the key never reaches the web view.
 */
export async function copyKey(email: string) {
  try {
    if (inTauri()) await invoke("copy_key", { email });
    else await mockCopyKey(email);
    toastSuccess("Key copied");
  } catch {
    toastError("Copy failed");
  }
}
