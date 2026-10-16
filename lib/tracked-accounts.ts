import type { ProviderResult } from "./provider-cache";
import { rememberedAccounts, rememberAccounts, type SeenAccount } from "./seen-accounts";
import type { TrackedAccount } from "./types";
import { readCached } from "./usage-cache";

export interface LiveAccount<T> {
  id: string;
  label: string | null;
  active: boolean;
  load: () => Promise<ProviderResult<T>>;
  /** Email or name learned from the usage payload, once a fetch succeeds. */
  labelFrom?: (usage: T | null) => string | null;
}

function normalizeId(id: string): string {
  return id.trim().toLowerCase();
}

/**
 * Live sign-ins plus every account seen before. Accounts the app is no longer
 * signed into keep their last cached reading and are not fetched again.
 */
export async function collectAccounts<T>(
  provider: string,
  section: string,
  live: LiveAccount<T>[],
): Promise<TrackedAccount<T>[]> {
  const usable = live.map((a) => ({ ...a, id: normalizeId(a.id) })).filter((a) => a.id);
  await rememberAccounts(
    provider,
    usable.map((a) => ({ id: a.id, label: a.label })),
  );
  const seen = await rememberedAccounts(provider);
  const byId = new Map(usable.map((a) => [a.id, a]));
  const ids: SeenAccount[] = [...seen];
  for (const account of usable) {
    if (!ids.some((s) => s.id === account.id)) ids.push({ id: account.id, label: account.label });
  }

  const rows = await Promise.all(
    ids.map(async (entry) => {
      const current = byId.get(entry.id);
      if (!current) {
        const cached = await readCached<T>(section, entry.id);
        return {
          id: entry.id,
          label: entry.label,
          active: false,
          usage: cached?.data ?? null,
          stale: cached != null,
          cachedAt: cached?.savedAt ?? null,
          authExpired: false,
        } satisfies TrackedAccount<T>;
      }
      const result = await current.load();
      let usage = result.usage;
      let stale = result.stale;
      let cachedAt = result.cachedAt;
      // Older builds cached the single active account under "default".
      if (!usage && ids.length === 1) {
        const legacy = await readCached<T>(section, "default");
        if (legacy) {
          usage = legacy.data;
          stale = true;
          cachedAt = legacy.savedAt;
        }
      }
      const learned = current.labelFrom?.(usage)?.trim() || null;
      const label = learned ?? current.label ?? entry.label;
      return {
        id: entry.id,
        label,
        active: current.active,
        usage,
        stale,
        cachedAt,
        authExpired: result.authExpired,
      } satisfies TrackedAccount<T>;
    }),
  );

  const learned = rows.filter((r) => r.label && byId.has(r.id));
  if (learned.length) {
    await rememberAccounts(
      provider,
      learned.map((r) => ({ id: r.id, label: r.label })),
    );
  }

  rows.sort(
    (a, b) =>
      Number(b.active) - Number(a.active) || (a.label ?? a.id).localeCompare(b.label ?? b.id),
  );
  return rows;
}
