"use client";

import { useId, useState } from "react";
import { Badge, IconButton, Switch } from "@/app/_ui/controls";
import Dialog from "@/app/_ui/Dialog";
import { CloseIcon, GearIcon } from "@/app/_ui/Icons";
import type { Tone } from "@/app/_ui/palette";
import { capitalize } from "@/lib/format";
import type { ProviderId, ProviderResponse, Settings } from "@/lib/types";
import type { Summaries } from "../_lib/hooks";
import { NicknameField } from "./fields";

type Update = (patch: Partial<Settings>, success?: string) => Promise<boolean>;

export const PROVIDERS: Record<
  ProviderId,
  { name: string; icon: string; enabledKey: keyof Settings & `${string}Enabled` }
> = {
  cursor: { name: "Cursor", icon: "/cursor.ico", enabledKey: "cursorEnabled" },
  grok: { name: "Grok Bot", icon: "/grokbot.ico", enabledKey: "grokEnabled" },
  codex: { name: "Codex", icon: "/openai.png", enabledKey: "codexEnabled" },
  claude: { name: "Claude", icon: "/claude.ico", enabledKey: "claudeEnabled" },
};

interface Status {
  account: string | null;
  plan: string | null;
  tone: Tone;
  stale: boolean;
  authExpired: boolean;
}

function statusOf(id: ProviderId, s: Summaries): Status {
  const base = (r: ProviderResponse<unknown> | null) => ({
    stale: r?.stale === true,
    authExpired: r?.authExpired === true,
  });
  switch (id) {
    case "cursor":
      return {
        ...base(s.cursor),
        account: s.cursor?.usage?.accountName ?? null,
        plan: s.cursor?.usage?.planName ?? null,
        tone: "blue",
      };
    case "grok": {
      const g = s.grok?.usage;
      const account = g?.email ? `${g.email}${g.source === "cursor" ? " (via Cursor)" : ""}` : null;
      return {
        ...base(s.grok),
        account,
        plan: g?.planLabel ?? g?.cursorPlanName ?? null,
        tone: "blue",
      };
    }
    case "codex": {
      const plan = s.codex?.usage?.planType ?? null;
      return {
        ...base(s.codex),
        account: s.codex?.usage?.email ?? null,
        plan: plan ? capitalize(plan) : null,
        tone: plan?.toLowerCase() === "free" ? "green" : "blue",
      };
    }
    case "claude": {
      const c = s.claude?.usage;
      const plan = !s.claude
        ? null
        : !c
          ? "unavailable"
          : !c.signedIn
            ? "not signed in"
            : (c.planName ?? (c.subscribed ? (c.subscriptionType ?? "subscribed") : "Free"));
      const tone: Tone =
        plan === "Free"
          ? "green"
          : plan === "unavailable" || plan === "not signed in"
            ? "gray"
            : "blue";
      return { ...base(s.claude), account: c?.email ?? null, plan, tone };
    }
  }
}

function ProviderDialog({
  id,
  status,
  settings,
  update,
  onClose,
}: {
  id: ProviderId;
  status: Status;
  settings: Settings;
  update: Update;
  onClose: () => void;
}) {
  const titleId = useId();
  const switchId = useId();
  const { name, enabledKey } = PROVIDERS[id];
  const enabled = settings[enabledKey];
  return (
    <Dialog labelledBy={titleId} onClose={onClose}>
      <div className="bg-(--p-bg) rounded-xl shadow-2xl w-[420px] border border-(--p-border) p-5">
        <div className="flex items-start justify-between mb-4 gap-2">
          <div className="min-w-0">
            <div className="flex items-center gap-1.5">
              <h3 id={titleId} className="text-base font-bold">
                {name}
              </h3>
              {status.plan && <Badge tone={status.tone}>{status.plan}</Badge>}
            </div>
            <div className="text-xs text-(--p-muted) truncate">
              {status.account ?? "account unavailable"}
            </div>
          </div>
          <IconButton label="Close" onClick={onClose}>
            <CloseIcon size={16} />
          </IconButton>
        </div>
        <div className="space-y-3">
          <div className="flex items-center gap-2">
            <Switch
              labelledBy={switchId}
              checked={enabled}
              onChange={(next) =>
                void update(
                  { [enabledKey]: next },
                  `${name} ${next ? "shown on" : "hidden from"} widget`,
                )
              }
            />
            <span id={switchId} className="text-sm text-(--p-textSecondary)">
              Show on widget
            </span>
          </div>
          <NicknameField
            value={settings.displayNames[id] ?? ""}
            onSave={(nick) => update({ displayNames: { [id]: nick } }, "Nickname saved")}
          />
          {status.authExpired && (
            <p className="text-xs text-(--p-muted)">
              The sign-in this app reads has expired. Open {name} to refresh it — this monitor never
              refreshes or writes another app&apos;s credentials.
            </p>
          )}
        </div>
      </div>
    </Dialog>
  );
}

export default function ProviderPanel({
  id,
  summaries,
  settings,
  update,
}: {
  id: ProviderId;
  summaries: Summaries;
  settings: Settings;
  update: Update;
}) {
  const [open, setOpen] = useState(false);
  const status = statusOf(id, summaries);
  const { name, enabledKey } = PROVIDERS[id];
  return (
    <div
      className={`bg-(--p-surface) border border-(--p-border) rounded-lg px-3 py-2 ${settings[enabledKey] ? "" : "border-dashed"}`}
    >
      <div className="flex items-center gap-2">
        <span className="flex-1 min-w-0 text-sm text-(--p-text) truncate">
          {status.account ?? "account unavailable"}
        </span>
        {!settings[enabledKey] && <Badge tone="gray">hidden</Badge>}
        {status.authExpired && (
          <Badge tone="amber" title={`Sign-in expired · open ${name} to refresh it`}>
            re-auth
          </Badge>
        )}
        {status.stale && <Badge tone="gray">cached</Badge>}
        <IconButton label={`${name} details`} onClick={() => setOpen(true)}>
          <GearIcon />
        </IconButton>
      </div>
      <div className="mt-1 flex items-center gap-2">
        <span className="flex-1 min-w-0 text-xs text-(--p-muted) truncate">
          {settings.displayNames[id] ?? "no nickname"}
        </span>
        {status.plan && <Badge tone={status.tone}>{status.plan}</Badge>}
      </div>
      {open && (
        <ProviderDialog
          id={id}
          status={status}
          settings={settings}
          update={update}
          onClose={() => setOpen(false)}
        />
      )}
    </div>
  );
}
