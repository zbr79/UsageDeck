//! Cursor usage per signed-in account (read-only sign-in database via rusqlite).
//!
//! STUB (wave 1). Port from: lib/cursor.ts and app/api/cursor/route.ts.
//! Build on the shared framework in `providers/mod.rs`: `CachedProvider`, `UsageCache`,
//! `collect_accounts`, `fetch_json` / `parse_reply`, `ProviderError`.

use crate::Core;
use serde_json::Value;

/// Rows for the `accounts` array of the old route's JSON body (`TrackedAccount<CursorUsage>` each).
pub async fn accounts(core: &Core) -> Vec<Value> {
    let _ = core;
    Vec::new()
}
