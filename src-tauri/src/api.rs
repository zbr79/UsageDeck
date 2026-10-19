//! The `api(method, path, body)` command: the old Next.js routes, same paths and JSON bodies.
//!
//! Ports app/api/{settings,accounts,key,usage,cursor,grok,codex,claude}/route.ts. The old
//! local-secret check (`requireLocalSecret`) is gone: only the app's own windows can invoke.

use crate::providers;
use crate::store::key_hint;
use crate::Core;
use serde::Serialize;
use serde_json::{json, Value};

const MAX_EMAIL_LENGTH: usize = 320;
const MAX_KEY_LENGTH: usize = 512;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ApiResponse {
    pub status: u16,
    pub body: Value,
    /// settings or accounts were saved: the widget should reload them
    #[serde(skip)]
    pub settings_changed: bool,
    /// a settings save changed `startAtLogin` to this value: the shell applies it to the Run key
    #[serde(skip)]
    pub start_at_login: Option<bool>,
}

fn ok(body: Value) -> ApiResponse {
    ApiResponse {
        status: 200,
        body,
        settings_changed: false,
        start_at_login: None,
    }
}

fn saved(body: Value) -> ApiResponse {
    ApiResponse {
        status: 200,
        body,
        settings_changed: true,
        start_at_login: None,
    }
}

fn error(status: u16, message: &str) -> ApiResponse {
    ApiResponse {
        status,
        body: json!({ "error": message }),
        settings_changed: false,
        start_at_login: None,
    }
}

/// JS `String(value ?? "")` for the shapes a JSON body can hold.
fn js_string(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| match v {
                Value::Null => String::new(),
                other => js_string(Some(other)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".to_string(),
    }
}

fn field(body: &Value, name: &str) -> String {
    js_string(body.get(name)).trim().to_string()
}

/// Split "/api/x?email=a%40b.com" into the path and its query pairs.
fn split_path(path: &str) -> (String, Vec<(String, String)>) {
    match reqwest::Url::parse(&format!("http://app{path}")) {
        Ok(url) => (
            url.path().to_string(),
            url.query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect(),
        ),
        Err(_) => (path.to_string(), Vec::new()),
    }
}

fn store_failed(e: impl std::fmt::Display) -> ApiResponse {
    log::error!("store error: {e}");
    error(500, "could not save")
}

/// The full key of an OpenCode account (email matched case-insensitively), for `copy_key`.
pub async fn account_key(core: &Core, email: &str) -> Option<String> {
    let lower = email.to_lowercase();
    core.store
        .get_accounts()
        .await
        .into_iter()
        .find(|a| a.email.to_lowercase() == lower)
        .map(|a| a.key)
}

pub async fn dispatch(core: &Core, method: &str, path: &str, body: Option<Value>) -> ApiResponse {
    let body = body.unwrap_or(Value::Null);
    let (route, _query) = split_path(path);
    let method = method.to_ascii_uppercase();
    match (route.as_str(), method.as_str()) {
        ("/api/settings", "GET") => ok(json!({ "settings": core.store.get_settings() })),
        ("/api/settings", "POST") => {
            let mut before = None;
            match core
                .store
                .update_settings(|current| {
                    before = Some(current.start_at_login);
                    crate::store::apply_settings_patch(&current, &body)
                })
                .await
            {
                Ok(settings) => {
                    let after = settings.start_at_login;
                    let mut r = saved(json!({ "ok": true, "settings": settings }));
                    if before != Some(after) {
                        r.start_at_login = Some(after);
                    }
                    r
                }
                Err(e) => store_failed(e),
            }
        }
        ("/api/accounts", "GET") => {
            let accounts: Vec<Value> = core
                .store
                .get_accounts()
                .await
                .into_iter()
                .map(|a| json!({ "email": a.email, "keyHint": key_hint(&a.key) }))
                .collect();
            ok(json!({ "accounts": accounts }))
        }
        ("/api/accounts", "POST") => {
            let email = field(&body, "email");
            let key = field(&body, "key");
            if email.is_empty() || email.encode_utf16().count() > MAX_EMAIL_LENGTH {
                return error(400, "an email or label is required");
            }
            if !key.starts_with("sk-")
                || key.encode_utf16().count() > MAX_KEY_LENGTH
                || key.chars().any(char::is_whitespace)
            {
                return error(400, "a valid sk- key is required");
            }
            let result = core
                .store
                .update_accounts(|mut accounts| {
                    let lower = email.to_lowercase();
                    match accounts
                        .iter_mut()
                        .find(|a| a.email.to_lowercase() == lower)
                    {
                        Some(existing) => existing.key = key.clone(),
                        None => accounts.push(crate::store::Account {
                            email: email.clone(),
                            key: key.clone(),
                        }),
                    }
                    accounts
                })
                .await;
            match result {
                Ok(_) => saved(json!({ "ok": true })),
                Err(e) => store_failed(e),
            }
        }
        ("/api/accounts", "DELETE") => {
            let email = field(&body, "email");
            if email.is_empty() {
                return error(400, "email is required");
            }
            let lower = email.to_lowercase();
            let result = core
                .store
                .update_accounts(|accounts| {
                    accounts
                        .into_iter()
                        .filter(|a| a.email.to_lowercase() != lower)
                        .collect()
                })
                .await;
            if let Err(e) = result {
                return store_failed(e);
            }
            match core.store.forget_account(&email).await {
                Ok(_) => saved(json!({ "ok": true })),
                Err(e) => store_failed(e),
            }
        }
        ("/api/usage", "GET") => {
            ok(json!({ "accounts": providers::opencode::accounts(core).await }))
        }
        ("/api/cursor", "GET") => {
            ok(json!({ "accounts": providers::cursor::accounts(core).await }))
        }
        ("/api/grok", "GET") => ok(json!({ "accounts": providers::grok::accounts(core).await })),
        ("/api/codex", "GET") => ok(json!({ "accounts": providers::codex::accounts(core).await })),
        ("/api/claude", "GET") => {
            ok(json!({ "accounts": providers::claude::accounts(core).await }))
        }
        (
            "/api/settings" | "/api/accounts" | "/api/usage" | "/api/cursor" | "/api/grok"
            | "/api/codex" | "/api/claude",
            _,
        ) => error(405, "method not allowed"),
        _ => error(404, "not found"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{MemoryVault, Store};

    fn core(tag: &str) -> Core {
        let dir = std::env::temp_dir().join(format!(
            "usagedeck-test-api-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        Core::new(Store::new(dir, Box::new(MemoryVault::default())))
    }

    #[tokio::test]
    async fn settings_and_accounts_routes() {
        let core = core("routes");
        let r = dispatch(
            &core,
            "POST",
            "/api/settings",
            Some(json!({ "grokEnabled": false, "x": 1 })),
        )
        .await;
        assert_eq!(r.status, 200);
        assert!(r.settings_changed);
        assert_eq!(r.body["settings"]["grokEnabled"], json!(false));
        assert_eq!(r.start_at_login, None);
        let r = dispatch(&core, "GET", "/api/settings", None).await;
        assert_eq!(r.body["settings"]["grokEnabled"], json!(false));
        assert_eq!(r.body["settings"]["startAtLogin"], json!(true));

        // turning start-at-login off asks the shell to apply it; saving it unchanged does not
        let off = json!({ "startAtLogin": false });
        let r = dispatch(&core, "POST", "/api/settings", Some(off.clone())).await;
        assert_eq!(r.body["settings"]["startAtLogin"], json!(false));
        assert_eq!(r.start_at_login, Some(false));
        let r = dispatch(&core, "POST", "/api/settings", Some(off)).await;
        assert_eq!(r.start_at_login, None);
        let on = json!({ "startAtLogin": true });
        let r = dispatch(&core, "POST", "/api/settings", Some(on)).await;
        assert_eq!(r.start_at_login, Some(true));

        let bad = dispatch(
            &core,
            "POST",
            "/api/accounts",
            Some(json!({ "email": "a@b.com", "key": "sk-a b" })),
        )
        .await;
        assert_eq!(
            (bad.status, bad.body["error"].clone()),
            (400, json!("a valid sk- key is required"))
        );
        let bad = dispatch(
            &core,
            "POST",
            "/api/accounts",
            Some(json!({ "key": "sk-abc" })),
        )
        .await;
        assert_eq!(bad.status, 400);

        let r = dispatch(
            &core,
            "POST",
            "/api/accounts",
            Some(json!({ "email": " A@b.com ", "key": "sk-1234567890" })),
        )
        .await;
        assert_eq!(r.body, json!({ "ok": true }));
        let r = dispatch(&core, "GET", "/api/accounts", None).await;
        assert_eq!(
            r.body,
            json!({ "accounts": [{ "email": "A@b.com", "keyHint": "sk-…7890" }] })
        );
        // the file holds emails only
        let file = core.store.read_json(crate::store::ACCOUNTS_FILE).unwrap();
        assert_eq!(file, json!([{ "email": "A@b.com" }]));

        // the full key never goes to the page: copy_key writes it to the clipboard in Rust
        for method in ["GET", "POST"] {
            let r = dispatch(&core, method, "/api/key?email=a%40B.com", None).await;
            assert_eq!(r.status, 404);
            assert!(!r.body.to_string().contains("sk-1234567890"));
        }
        assert_eq!(
            account_key(&core, "a@B.com").await,
            Some("sk-1234567890".to_string())
        );
        assert_eq!(account_key(&core, "nobody@b.com").await, None);
        let r = dispatch(
            &core,
            "DELETE",
            "/api/accounts",
            Some(json!({ "email": "a@b.com" })),
        )
        .await;
        assert_eq!(r.status, 200);

        assert_eq!(dispatch(&core, "GET", "/api/nope", None).await.status, 404);
        assert_eq!(
            dispatch(&core, "DELETE", "/api/settings", None)
                .await
                .status,
            405
        );
        let r = dispatch(&core, "GET", "/api/cursor", None).await;
        assert_eq!(r.body, json!({ "accounts": [] }));
    }
}
