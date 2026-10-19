import { toastError, toastSuccess } from "@/components/toast/toast";
import { getJson } from "./api";

/** Copy an OpenCode account's API key to the clipboard (widget card icon and settings card). */
export async function copyKey(email: string) {
  try {
    const { key } = await getJson<{ key: string }>(`/api/key?email=${encodeURIComponent(email)}`);
    await navigator.clipboard.writeText(key);
    toastSuccess("Key copied");
  } catch {
    toastError("Copy failed");
  }
}
