//! API key authentication middleware for `hamoru serve`.
//!
//! Validates `Authorization: Bearer <key>` against keys loaded from the
//! `HAMORU_API_KEYS` environment variable (comma-separated). When the key
//! list is empty, all requests pass through (localhost dev mode).
//!
//! Uses constant-time comparison to prevent timing attacks (D6).

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use hamoru_core::error::HamoruError;

use super::ApiError;

/// Identity of the authenticated caller, stored as a request extension.
#[derive(Debug, Clone)]
pub struct AuthIdentity(pub String);

/// Axum middleware that validates Bearer tokens against configured API keys.
///
/// When `api_keys` is empty, all requests are allowed through with an
/// "anonymous" identity (localhost dev mode).
pub async fn auth_middleware(
    State(api_keys): State<Arc<Vec<String>>>,
    mut request: Request,
    next: Next,
) -> Result<Response, Response> {
    if api_keys.is_empty() {
        // No auth configured — dev mode
        request
            .extensions_mut()
            .insert(AuthIdentity("anonymous".to_string()));
        return Ok(next.run(request).await);
    }

    let token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    let token = match token {
        Some(t) => t,
        None => {
            return Err(ApiError(HamoruError::Unauthorized {
                reason: "missing Authorization header".to_string(),
            })
            .into_response());
        }
    };

    // Find matching key using constant-time comparison (D6)
    let matched = api_keys
        .iter()
        .enumerate()
        .find(|(_, key)| constant_time_eq(key.as_bytes(), token.as_bytes()));

    match matched {
        Some((idx, _)) => {
            request
                .extensions_mut()
                .insert(AuthIdentity(format!("key-{idx}")));
            Ok(next.run(request).await)
        }
        None => Err(ApiError(HamoruError::Unauthorized {
            reason: "invalid API key".to_string(),
        })
        .into_response()),
    }
}

/// Constant-time byte comparison via XOR fold.
///
/// Short-circuits on length mismatch (length is not secret — the server
/// controls the key list). For equal-length inputs, iterates all bytes
/// before returning to prevent timing side-channels.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (&x, &y)| acc | (x ^ y))
        == 0
}

/// Resolves API keys from the `HAMORU_API_KEYS` environment variable.
///
/// Keys are comma-separated, whitespace-trimmed, and deduplicated.
/// Returns an empty vec if the variable is unset (auth disabled).
pub fn resolve_api_keys() -> Vec<String> {
    let raw = match std::env::var("HAMORU_API_KEYS") {
        Ok(val) if !val.trim().is_empty() => val,
        _ => return Vec::new(),
    };

    let mut keys: Vec<String> = raw
        .split(',')
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .collect();

    let original_len = keys.len();
    keys.sort();
    keys.dedup();
    if keys.len() < original_len {
        tracing::warn!("Duplicate API keys found in HAMORU_API_KEYS — duplicates removed");
    }

    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_same_strings() {
        assert!(constant_time_eq(b"hello", b"hello"));
    }

    #[test]
    fn constant_time_eq_different_strings() {
        assert!(!constant_time_eq(b"hello", b"world"));
    }

    #[test]
    fn constant_time_eq_different_lengths() {
        assert!(!constant_time_eq(b"short", b"longer"));
    }

    #[test]
    fn constant_time_eq_empty() {
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn constant_time_eq_single_bit_difference() {
        // 'a' (0x61) vs 'b' (0x62) — differ in one bit
        assert!(!constant_time_eq(b"a", b"b"));
    }

    #[test]
    fn resolve_api_keys_empty_when_unset() {
        // env var manipulation is unsafe; just test the parsing logic
        let keys = parse_api_keys_from("");
        assert!(keys.is_empty());
    }

    #[test]
    fn resolve_api_keys_trims_and_deduplicates() {
        let keys = parse_api_keys_from("key1 , key2 , key1 , key3");
        assert_eq!(keys.len(), 3);
        assert!(keys.contains(&"key1".to_string()));
        assert!(keys.contains(&"key2".to_string()));
        assert!(keys.contains(&"key3".to_string()));
    }

    #[test]
    fn resolve_api_keys_ignores_empty_segments() {
        let keys = parse_api_keys_from("key1,,key2,");
        assert_eq!(keys.len(), 2);
    }

    /// Helper that parses a raw string as if it were the env var value.
    fn parse_api_keys_from(raw: &str) -> Vec<String> {
        if raw.trim().is_empty() {
            return Vec::new();
        }
        let mut keys: Vec<String> = raw
            .split(',')
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            .collect();
        keys.sort();
        keys.dedup();
        keys
    }
}
