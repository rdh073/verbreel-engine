//! Top-level args validation entry point.
//!
//! Engine dispatchers call exactly one function from this crate:
//! [`validate`]. It folds the two failure modes (verb-not-registered,
//! schema-violation) into a single [`ValidationError`] so the caller
//! does not have to chain two lookups.
//!
//! Why first-error only? Per the brief, args validation is a
//! gate — the caller rejects the whole payload on the first failing
//! keyword. Enumerating every violation would do extra work for a
//! result that is never observed. Advanced callers that want the full
//! error list can reach through [`crate::Schema::validator`] and call
//! `iter_errors` themselves.

use serde_json::Value;

use crate::error::ValidationError;
use crate::registry::ArgsRegistry;

/// Validate `args` against the schema registered for `verb` in
/// `registry`.
///
/// Two-step dispatch:
///
/// 1. Look up `verb` in `registry`. Absent → [`ValidationError::UnknownVerb`].
/// 2. Run the pre-compiled validator. First violation →
///    [`ValidationError::SchemaViolation`] with the validator's own
///    error message in `detail`.
///
/// # Errors
///
/// - [`ValidationError::UnknownVerb`] — no schema registered for
///   `verb`.
/// - [`ValidationError::SchemaViolation`] — the schema rejected
///   `args`; the first violation message is included.
pub fn validate(verb: &str, args: &Value, registry: &ArgsRegistry) -> Result<(), ValidationError> {
    let schema = registry
        .get(verb)
        .ok_or_else(|| ValidationError::UnknownVerb {
            verb: verb.to_owned(),
        })?;

    // First-error pattern per jsonschema 0.46: `iter_errors().next()` is
    // O(1) — the validator short-circuits at the first failing keyword
    // when the iterator is dropped.
    if let Some(first) = schema.validator().iter_errors(args).next() {
        return Err(ValidationError::SchemaViolation {
            detail: first.to_string(),
        });
    }
    Ok(())
}
