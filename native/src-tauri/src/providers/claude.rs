//! Claude usage per signed-in account (Claude desktop token, then Claude Code token).
//!
//! STUB (wave 1). Port from: lib/claude.ts, lib/os-crypt.ts and app/api/claude/route.ts.
//! Build on the shared framework in `providers/mod.rs`: `CachedProvider`, `UsageCache`,
//! `collect_accounts`, `fetch_json` / `parse_reply`, `ProviderError`.

use crate::Core;
use serde_json::Value;

/// Rows for the `accounts` array of the old route's JSON body (`TrackedAccount<ClaudeUsage>` each).
pub async fn accounts(core: &Core) -> Vec<Value> {
    let _ = core;
    Vec::new()
}
