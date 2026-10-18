//! Codex usage per signed-in account.
//!
//! STUB (wave 1). Port from: lib/codex.ts and app/api/codex/route.ts.
//! Build on the shared framework in `providers/mod.rs`: `CachedProvider`, `UsageCache`,
//! `collect_accounts`, `fetch_json` / `parse_reply`, `ProviderError`.

use crate::Core;
use serde_json::Value;

/// Rows for the `accounts` array of the old route's JSON body (`TrackedAccount<CodexUsage>` each).
pub async fn accounts(core: &Core) -> Vec<Value> {
    let _ = core;
    Vec::new()
}
