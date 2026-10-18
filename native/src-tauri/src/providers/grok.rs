//! Grok Bot usage (Grok Bot sign-in, Cursor as fallback).
//!
//! STUB (wave 1). Port from: lib/grokbot.ts, lib/os-crypt.ts and app/api/grok/route.ts.
//! Build on the shared framework in `providers/mod.rs`: `CachedProvider`, `UsageCache`,
//! `collect_accounts`, `fetch_json` / `parse_reply`, `ProviderError`.

use crate::Core;
use serde_json::Value;

/// Rows for the `accounts` array of the old route's JSON body (`TrackedAccount<GrokUsage>` each).
pub async fn accounts(core: &Core) -> Vec<Value> {
    let _ = core;
    Vec::new()
}
