import { claudeResetRows } from "@/lib/claude-resets";
import type { ClaudeResets } from "@/lib/types";
import { Section, SettingRow } from "./fields";

/** Claude's resets, one row per kind; grouped under each account's name when several have some. */
export default function ResetsSection({
  accounts,
}: {
  accounts: readonly { id: string; name: string; resets: ClaudeResets }[];
}) {
  if (accounts.length === 0) return null;
  const named = accounts.length > 1;
  return (
    <Section title="Resets">
      {accounts.map((account) => (
        <div key={account.id} className="flex flex-col gap-1">
          {named && <h3 className="mt-2 px-1 text-xs text-(--p-muted)">{account.name}</h3>}
          {claudeResetRows(account.resets).map((row) => (
            <SettingRow
              key={row.key}
              title={row.title}
              control={() => (
                <span className="text-sm whitespace-nowrap text-(--p-muted)">{row.detail}</span>
              )}
            />
          ))}
        </div>
      ))}
    </Section>
  );
}
