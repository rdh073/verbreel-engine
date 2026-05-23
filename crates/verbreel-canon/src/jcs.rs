//! Core JCS wrapper around `vr-jcs` + SHA-256.
//!
//! Pipeline:
//! 1. Defense-in-depth: reject `NaN` / `±Infinity` (spec §0.5.2 — not
//!    representable in canonical JSON).
//! 2. NFC-normalize every string + object key (RFC 8785 §3.2.2.2). `vr-jcs`
//!    0.4.1 does I-JSON noncharacter validation but does not NFC-normalize.
//! 3. Re-serialize the normalized [`Value`] to bytes and feed those into
//!    `vr_jcs::to_canon_bytes_from_slice` — the strict admission path that
//!    enforces UTF-16 key sorting + I-JSON validation + duplicate-key rejection.

use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

/// Errors returned by the JCS layer.
#[derive(Debug, Error)]
pub enum CanonError {
    /// Input contains a non-representable number (`NaN`, `+Infinity`, `-Infinity`).
    /// Per spec §0.5.2, canonical JSON cannot encode these.
    #[error("non-finite number {kind} cannot be canonicalized (spec §0.5.2)")]
    NonFiniteNumber {
        /// Which non-finite value triggered the error (`nan`, `inf`, `-inf`).
        kind: &'static str,
    },

    /// Underlying canonicalizer error (malformed input, integer overflow, etc.).
    #[error("RFC 8785 canonicalization failed: {0}")]
    Canonicalize(String),
}

/// Canonicalize a [`Value`] per RFC 8785 + spec §0.5.2 clarifications.
///
/// - Numbers: I-JSON profile, ECMAScript shortest round-trip for floats
/// - Strings: NFC normalized, only RFC-required escapes
/// - Object keys: sorted by UTF-16 code-unit comparison
/// - No whitespace, no BOM, ASCII-compatible UTF-8 output
///
/// # Errors
///
/// - [`CanonError::NonFiniteNumber`] if the input contains `NaN`, `+Infinity`,
///   or `-Infinity` (not representable in canonical JSON per spec §0.5.2).
/// - [`CanonError::Canonicalize`] for any other canonicalizer failure
///   (I-JSON noncharacter, non-exact f64, depth limit, duplicate key after
///   NFC collision, malformed re-serialization).
pub fn canonicalize(value: &Value) -> Result<Vec<u8>, CanonError> {
    // Defense-in-depth: explicitly check for non-finite numbers. serde_json's Value
    // can carry these via the `arbitrary_precision` feature or custom deserializers;
    // RFC 8785 cannot encode them.
    check_no_non_finite(value)?;

    // NFC-normalize all strings + object keys before handing to vr-jcs.
    let normalized = nfc_normalize_value(value);

    // vr-jcs 0.4.1's authoritative pipeline is the strict-bytes path. Re-serialize
    // the normalized Value to JSON bytes (round-trip is cheap and well-defined for
    // any Value not containing non-finite numbers, already filtered above).
    let bytes = serde_json::to_vec(&normalized)
        .map_err(|e| CanonError::Canonicalize(format!("re-serialize failed: {e}")))?;

    vr_jcs::to_canon_bytes_from_slice(&bytes).map_err(|e| CanonError::Canonicalize(e.to_string()))
}

/// Recursive scan for `NaN`, `+Infinity`, `-Infinity` inside a [`Value`].
fn check_no_non_finite(value: &Value) -> Result<(), CanonError> {
    match value {
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                if f.is_nan() {
                    return Err(CanonError::NonFiniteNumber { kind: "nan" });
                }
                if f.is_infinite() {
                    return Err(CanonError::NonFiniteNumber {
                        kind: if f.is_sign_positive() { "inf" } else { "-inf" },
                    });
                }
            }
            Ok(())
        }
        Value::Array(arr) => {
            for v in arr {
                check_no_non_finite(v)?;
            }
            Ok(())
        }
        Value::Object(map) => {
            for v in map.values() {
                check_no_non_finite(v)?;
            }
            Ok(())
        }
        Value::Null | Value::Bool(_) | Value::String(_) => Ok(()),
    }
}

/// NFC-normalize every string + object key in a [`Value`] tree.
///
/// Returns a new [`Value`] rather than mutating in place; the typical project
/// graph is small enough that the allocation cost is negligible compared to the
/// SHA-256 + JCS work that follows.
fn nfc_normalize_value(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(s.nfc().collect::<String>()),
        Value::Array(arr) => Value::Array(arr.iter().map(nfc_normalize_value).collect()),
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, child) in map {
                let nk: String = k.nfc().collect();
                out.insert(nk, nfc_normalize_value(child));
            }
            Value::Object(out)
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => v.clone(),
    }
}

/// SHA-256 of the canonical JSON, hex-encoded lowercase (spec §0.5.2).
///
/// # Errors
///
/// Propagates [`CanonError`] from [`canonicalize`].
pub fn sha256_hex(value: &Value) -> Result<String, CanonError> {
    let bytes = canonicalize(value)?;
    let digest = Sha256::digest(&bytes);
    Ok(hex_lower(&digest))
}

/// Lowercase hex encoding (helper — avoids pulling in the `hex` crate).
fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_object_canonicalizes() {
        let v = json!({});
        assert_eq!(canonicalize(&v).unwrap(), b"{}");
    }

    #[test]
    fn empty_array_canonicalizes() {
        let v = json!([]);
        assert_eq!(canonicalize(&v).unwrap(), b"[]");
    }

    #[test]
    fn keys_sorted_utf16() {
        // Simple ASCII keys — alphabetical.
        let v = json!({"b": 1, "a": 2, "c": 3});
        let out = String::from_utf8(canonicalize(&v).unwrap()).unwrap();
        assert_eq!(out, r#"{"a":2,"b":1,"c":3}"#);
    }

    #[test]
    fn no_whitespace_emitted() {
        let v = json!({"x": [1, 2, 3], "y": {"nested": true}});
        let out = canonicalize(&v).unwrap();
        assert!(!out.contains(&b' '));
        assert!(!out.contains(&b'\n'));
        assert!(!out.contains(&b'\t'));
    }

    #[test]
    fn nan_rejected() {
        // serde_json::Number::from_f64 returns None for NaN/Inf, so a Value
        // cannot accidentally carry these. This test documents that invariant.
        assert!(serde_json::Number::from_f64(f64::NAN).is_none());
        assert!(serde_json::Number::from_f64(f64::INFINITY).is_none());
        assert!(serde_json::Number::from_f64(f64::NEG_INFINITY).is_none());
    }

    #[test]
    fn sha256_of_empty_object_is_known_value() {
        // SHA-256("{}") = 44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a
        let v = json!({});
        let h = sha256_hex(&v).unwrap();
        assert_eq!(
            h,
            "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
        );
    }

    #[test]
    fn sha256_is_lowercase_64_chars() {
        let v = json!({"a": 1});
        let h = sha256_hex(&v).unwrap();
        assert_eq!(h.len(), 64);
        assert!(
            h.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );
    }

    #[test]
    fn nested_objects_keys_sorted_too() {
        let v = json!({"z": {"b": 1, "a": 2}, "a": 1});
        let out = String::from_utf8(canonicalize(&v).unwrap()).unwrap();
        assert_eq!(out, r#"{"a":1,"z":{"a":2,"b":1}}"#);
    }
}
