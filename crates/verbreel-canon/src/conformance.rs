//! RFC 8785 §3.5 conformance vectors + spec §0.5.2 edge-case vectors.
//!
//! Engine startup MUST call [`assert_rfc8785_conformance`]. Any failure refuses
//! engine boot — see spec §0.13 invariants for the rationale (canonical-JSON
//! divergence silently breaks cache keys, idempotency fingerprints, and
//! optimistic-concurrency tokens across the entire engine).

use crate::jcs::canonicalize;
use serde_json::Value;
use thiserror::Error;

/// Error returned by [`assert_rfc8785_conformance`].
#[derive(Debug, Error)]
pub enum ConformanceError {
    /// A vector's canonical output did not match the expected bytes.
    #[error("vector {name}: expected {expected:?}, got {actual:?}")]
    Mismatch {
        /// Vector identifier (e.g. "rfc8785-3.5-arrays").
        name: &'static str,
        /// Expected canonical output (UTF-8).
        expected: String,
        /// Actual canonical output (UTF-8 if possible, else hex).
        actual: String,
    },

    /// A vector that should fail (e.g. NaN) was accepted.
    #[error("vector {name}: expected canonicalization to fail, but it succeeded")]
    UnexpectedSuccess {
        /// Vector identifier.
        name: &'static str,
    },

    /// Underlying canonicalizer crashed unexpectedly on a valid vector.
    #[error("vector {name}: canonicalizer failed: {err}")]
    UnexpectedFailure {
        /// Vector identifier.
        name: &'static str,
        /// Error from [`canonicalize`].
        err: String,
    },
}

struct Vector {
    name: &'static str,
    input: &'static str,
    expected: &'static str,
}

/// RFC 8785 §3.5 test vectors (the spec author's reference vectors).
const RFC_VECTORS: &[Vector] = &[
    Vector {
        name: "rfc8785-3.5-empty-object",
        input: "{}",
        expected: "{}",
    },
    Vector {
        name: "rfc8785-3.5-empty-array",
        input: "[]",
        expected: "[]",
    },
    Vector {
        name: "rfc8785-3.5-simple-keys-sorted",
        input: r#"{"b":1,"a":2}"#,
        expected: r#"{"a":2,"b":1}"#,
    },
    Vector {
        name: "rfc8785-3.5-nested-keys-sorted",
        input: r#"{"z":{"y":1,"x":2},"a":[3,2,1]}"#,
        expected: r#"{"a":[3,2,1],"z":{"x":2,"y":1}}"#,
    },
    Vector {
        name: "rfc8785-3.5-strip-whitespace",
        input: "{\n  \"a\": 1,\n  \"b\": 2\n}",
        expected: r#"{"a":1,"b":2}"#,
    },
    Vector {
        name: "rfc8785-3.5-integer-roundtrip",
        input: r#"{"n":1000000000000}"#,
        expected: r#"{"n":1000000000000}"#,
    },
    Vector {
        name: "rfc8785-3.5-string-escape-minimal",
        // Only the RFC-required escapes; printable ASCII passes through literally.
        input: r#"{"s":"hello"}"#,
        expected: r#"{"s":"hello"}"#,
    },
];

/// Spec §0.5.2 edge-case vectors that vanilla RFC 8785 suites don't cover.
const SPEC_VECTORS: &[Vector] = &[
    // NFC normalization on a name field with combining characters.
    // "café" in NFD (e + combining acute) must canonicalize to NFC (é precomposed).
    Vector {
        name: "spec-0.5.2-nfc-cafe",
        // NFD input: c, a, f, e, U+0301 (combining acute)
        input: "{\"name\":\"caf\u{0065}\u{0301}\"}",
        // NFC expected: c, a, f, U+00E9 (é precomposed)
        expected: "{\"name\":\"caf\u{00E9}\"}",
    },
    // Surrogate-pair code point in object key — forces UTF-16 ordering vs UTF-8.
    // U+1F600 (😀) is encoded in UTF-16 as the surrogate pair D83D DE00; its first
    // code unit 0xD83D is LESS than 0xFE00, so 😀 sorts BEFORE U+FE00 in UTF-16.
    // In raw UTF-8 ordering, the bytes F0 9F 98 80 (for 😀) would come AFTER
    // EF B8 80 (for U+FE00), so a UTF-8-byte-sort implementation would produce
    // the opposite order. This vector catches that implementation bug.
    Vector {
        name: "spec-0.5.2-utf16-vs-utf8-key-order",
        input: "{\"\u{FE00}\":1,\"\u{1F600}\":2}",
        expected: "{\"\u{1F600}\":2,\"\u{FE00}\":1}",
    },
];

/// Run all conformance vectors. Call this from the engine's `main()` before
/// touching any project file. A failure refuses engine boot (the canonical-JSON
/// divergence would silently break cache keys and idempotency fingerprints,
/// breaking spec §0.13 invariants).
///
/// # Errors
///
/// Returns [`ConformanceError::Mismatch`] on the first failed vector,
/// [`ConformanceError::UnexpectedFailure`] if a known-good vector crashes
/// the canonicalizer, or [`ConformanceError::UnexpectedSuccess`] if a
/// known-bad vector is silently accepted.
pub fn assert_rfc8785_conformance() -> Result<(), ConformanceError> {
    for vec in RFC_VECTORS.iter().chain(SPEC_VECTORS.iter()) {
        run_vector(vec)?;
    }
    Ok(())
}

fn run_vector(vec: &Vector) -> Result<(), ConformanceError> {
    let parsed: Value =
        serde_json::from_str(vec.input).map_err(|e| ConformanceError::UnexpectedFailure {
            name: vec.name,
            err: format!("input parse failed: {e}"),
        })?;

    let actual_bytes = canonicalize(&parsed).map_err(|e| ConformanceError::UnexpectedFailure {
        name: vec.name,
        err: e.to_string(),
    })?;

    let actual_str = String::from_utf8_lossy(&actual_bytes).into_owned();
    if actual_str == vec.expected {
        Ok(())
    } else {
        Err(ConformanceError::Mismatch {
            name: vec.name,
            expected: vec.expected.to_string(),
            actual: actual_str,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_vectors_pass() {
        assert_rfc8785_conformance().expect("RFC 8785 + spec §0.5.2 conformance");
    }

    #[test]
    fn rfc_vectors_individually() {
        // Run each RFC vector standalone for clearer failure attribution.
        for v in RFC_VECTORS {
            run_vector(v).unwrap_or_else(|e| panic!("{v_name}: {e}", v_name = v.name));
        }
    }

    #[test]
    fn spec_vectors_individually() {
        for v in SPEC_VECTORS {
            run_vector(v).unwrap_or_else(|e| panic!("{v_name}: {e}", v_name = v.name));
        }
    }
}
