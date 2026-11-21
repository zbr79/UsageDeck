//! Read-only access to Chromium/Electron "safeStorage" secrets (Claude desktop, Grok Bot).
//! Port of lib/os-crypt.ts: the AES key in `Local State` (`os_crypt.encrypted_key`, base64,
//! "DPAPI" prefix) is DPAPI-protected for the current Windows user; values are sealed as
//! "v10" + 12-byte nonce + AES-256-GCM ciphertext + 16-byte tag.
//!
//! DPAPI is called directly (`CryptUnprotectData`), not through PowerShell as the old code did.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};

pub type AppKey = [u8; 32];

/// Copy a DPAPI output blob and free it.
fn take_blob(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    // SAFETY: on success DPAPI returns `cbData` bytes at `pbData`, allocated with LocalAlloc.
    unsafe {
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData.cast())));
        bytes
    }
}

/// DPAPI-decrypt `data` for the current user.
pub fn dpapi_unprotect(data: &[u8]) -> Option<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut out = CRYPT_INTEGER_BLOB::default();
    // SAFETY: `input` points at `data`, which outlives the call; `out` receives a LocalAlloc'd blob.
    unsafe { CryptUnprotectData(&input, None, None, None, None, 0, &mut out) }.ok()?;
    Some(take_blob(out))
}

/// DPAPI-encrypt `data` for the current user (tests only; this app never seals secrets).
#[cfg(test)]
pub fn dpapi_protect(data: &[u8]) -> Option<Vec<u8>> {
    use windows::Win32::Security::Cryptography::CryptProtectData;
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut out = CRYPT_INTEGER_BLOB::default();
    // SAFETY: as in `dpapi_unprotect`.
    unsafe { CryptProtectData(&input, None, None, None, None, 0, &mut out) }.ok()?;
    Some(take_blob(out))
}

/// Keyed by the encrypted blob, so a re-keyed app (new Local State) misses naturally. Only
/// successes are kept, so a failure is retried on the next refresh.
static KEY_CACHE: LazyLock<Mutex<HashMap<String, AppKey>>> = LazyLock::new(Default::default);

/// The AES-256 key protecting an app's safeStorage values, or None.
pub fn app_key(local_state: &Path) -> Option<AppKey> {
    app_key_or_why(local_state).ok()
}

/// The same key, or which step failed (for diagnostics; the text holds no secret).
pub fn app_key_or_why(local_state: &Path) -> Result<AppKey, &'static str> {
    let text = std::fs::read_to_string(local_state).map_err(|_| "Local State not found")?;
    let json: Value = serde_json::from_str(&text).map_err(|_| "Local State is not JSON")?;
    let encrypted = json["os_crypt"]["encrypted_key"]
        .as_str()
        .filter(|k| !k.is_empty())
        .ok_or("Local State has no os_crypt.encrypted_key")?;
    if let Some(key) = KEY_CACHE.lock().unwrap().get(encrypted) {
        return Ok(*key);
    }
    let blob = B64
        .decode(encrypted)
        .map_err(|_| "the Local State key is not base64")?;
    // Strip the 5-byte "DPAPI" prefix, as the old code did (`$enc[5..]`).
    let protected = blob.get(5..).ok_or("the Local State key is too short")?;
    let key = dpapi_unprotect(protected)
        .ok_or("Windows (DPAPI) would not decrypt the Local State key for this user")?;
    let key: AppKey = key
        .try_into()
        .map_err(|_| "the Local State key is not 32 bytes")?;
    KEY_CACHE.lock().unwrap().insert(encrypted.to_string(), key);
    Ok(key)
}

/// Decrypt a base64 "v10" sealed value; None when it is not one or fails authentication.
pub fn unseal(key: &AppKey, sealed_b64: Option<&Value>) -> Option<String> {
    let b = B64.decode(sealed_b64?.as_str()?).ok()?;
    unseal_bytes(key, &b).ok()
}

/// Decrypt a "v10" sealed value, or say which step failed (the text holds no secret).
pub fn unseal_bytes(key: &AppKey, sealed: &[u8]) -> Result<String, String> {
    if sealed.len() < 3 + 12 + 16 || &sealed[..3] != b"v10" {
        let prefix: String = sealed.iter().take(3).map(|b| format!("{b:02x}")).collect();
        return Err(format!(
            "not a v10 safeStorage value ({} bytes, starts 0x{prefix})",
            sealed.len()
        ));
    }
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|_| "the key has the wrong length".to_string())?;
    // aes-gcm expects ciphertext || tag, which is exactly the rest of the value.
    let plain = cipher
        .decrypt(Nonce::from_slice(&sealed[3..15]), &sealed[15..])
        .map_err(|_| "AES-GCM rejected the value (the key does not match it)".to_string())?;
    String::from_utf8(plain).map_err(|_| "the decrypted value is not text".to_string())
}

/// Seal a value the way Chromium does (tests only).
#[cfg(test)]
pub fn seal(key: &AppKey, plain: &str) -> String {
    let nonce = [7u8; 12];
    let cipher = Aes256Gcm::new_from_slice(key).unwrap();
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), plain.as_bytes())
        .unwrap();
    let mut out = b"v10".to_vec();
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    B64.encode(out)
}

/// Write a `Local State` whose key is DPAPI-protected for this user (tests only).
#[cfg(test)]
pub fn write_local_state(path: &Path, key: &AppKey) {
    let mut blob = b"DPAPI".to_vec();
    blob.extend_from_slice(&dpapi_protect(key).expect("CryptProtectData"));
    let json = serde_json::json!({ "os_crypt": { "encrypted_key": B64.encode(blob) } });
    std::fs::write(path, json.to_string()).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn os_crypt_round_trip() {
        let dir = std::env::temp_dir().join(format!("usagedeck-oscrypt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let local_state = dir.join("Local State");
        let key: AppKey = std::array::from_fn(|i| (i * 7 + 3) as u8);
        write_local_state(&local_state, &key);

        assert_eq!(app_key(&local_state), Some(key));
        let sealed = seal(&key, r#"{"hello":"world"}"#);
        assert_eq!(
            unseal(&key, Some(&json!(sealed))).as_deref(),
            Some(r#"{"hello":"world"}"#)
        );

        // wrong key, wrong prefix, not a string: no value
        let other: AppKey = [1; 32];
        assert_eq!(unseal(&other, Some(&json!(sealed))), None);
        let mut raw = B64.decode(&sealed).unwrap();
        raw[..3].copy_from_slice(b"v11");
        assert_eq!(unseal(&key, Some(&json!(B64.encode(raw)))), None);
        assert_eq!(unseal(&key, Some(&json!(5))), None);
        assert_eq!(unseal(&key, None), None);

        // a Local State without a key
        std::fs::write(&local_state, "{}").unwrap();
        assert_eq!(app_key(&local_state), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
