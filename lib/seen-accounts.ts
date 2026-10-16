import { dataFile } from "./paths";
import { readJson, withLock, writeJsonAtomic } from "./fs-atomic";

/**
 * Account identities this monitor has already seen. Tokens are never stored:
 * a sign-in that the other app has switched away from can still be listed,
 * and its last usage comes from the usage cache.
 */

export interface SeenAccount {
  id: string;
  label: string | null;
}

type Roster = Record<string, SeenAccount[]>;

const FILE_NAME = "seen-accounts.json";
const MAX_ACCOUNTS = 50;
const MAX_ID = 320;

function normalizeId(id: string): string {
  return id.trim().toLowerCase();
}

function clean(value: unknown): Roster {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const out: Roster = {};
  for (const [provider, rows] of Object.entries(value as Record<string, unknown>)) {
    if (!provider || provider.length > 32 || !Array.isArray(rows)) continue;
    const list: SeenAccount[] = [];
    const seen = new Set<string>();
    for (const row of rows) {
      if (!row || typeof row !== "object") continue;
      const rawId = (row as { id?: unknown }).id;
      const id = typeof rawId === "string" ? normalizeId(rawId) : "";
      if (!id || id.length > MAX_ID || seen.has(id)) continue;
      seen.add(id);
      const rawLabel = (row as { label?: unknown }).label;
      const label =
        typeof rawLabel === "string" && rawLabel.trim() ? rawLabel.trim().slice(0, MAX_ID) : null;
      list.push({ id, label });
      if (list.length >= MAX_ACCOUNTS) break;
    }
    out[provider] = list;
  }
  return out;
}

export async function rememberedAccounts(provider: string): Promise<SeenAccount[]> {
  return clean(await readJson(dataFile(FILE_NAME)))[provider] ?? [];
}

/** Add identities. Labels fill in when we learn them; an existing label is kept if the new one is empty. */
export async function rememberAccounts(provider: string, accounts: SeenAccount[]): Promise<void> {
  const incoming = accounts
    .map((a) => ({ id: normalizeId(a.id), label: a.label?.trim() ? a.label.trim() : null }))
    .filter((a) => a.id && a.id.length <= MAX_ID);
  if (incoming.length === 0) return;
  await withLock(FILE_NAME, async () => {
    const roster = clean(await readJson(dataFile(FILE_NAME)));
    const current = new Map((roster[provider] ?? []).map((a) => [a.id, a]));
    let changed = false;
    for (const account of incoming) {
      const prev = current.get(account.id);
      const label = account.label ?? prev?.label ?? null;
      if (prev && prev.label === label) continue;
      current.set(account.id, { id: account.id, label });
      changed = true;
    }
    if (!changed) return;
    roster[provider] = [...current.values()].slice(0, MAX_ACCOUNTS);
    await writeJsonAtomic(dataFile(FILE_NAME), roster);
  });
}
