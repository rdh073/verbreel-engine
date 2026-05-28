//! Error types for schema construction and args validation.
//!
//! Two distinct surfaces:
//!
//! - [`SchemaError`] — failure to *build* a [`crate::Schema`] from a raw
//!   `serde_json::Value`. Surfaced when the schema itself is
//!   ill-formed (not an object, fails jsonschema's own meta-validation,
//!   …). Construction-time only.
//!
//! - [`ValidationError`] — failure at *runtime* when checking some
//!   args value against a registered schema. Surfaced by
//!   [`crate::validate`]. Two flavours: the verb is not registered, or
//!   the args value violates the registered schema.
//!
//! Keeping the two enums separate prevents callers from having to
//! match a single error type that means "either you misconfigured the
//! registry OR your runtime args are bad" — those are different
//! failure modes with different remediation.

use thiserror::Error;

/// Error returned by [`crate::Schema::from_value`] when the raw schema
/// value cannot be turned into a compiled validator.
#[derive(Debug, Error)]
pub enum SchemaError {
    /// The raw schema value is not a JSON object. Per JSON Schema 2020-12
    /// the only valid schema shapes are an object or a boolean; we
    /// reject the boolean form at this layer because every verb schema
    /// in the engine is an object with `type`/`properties`/`required`.
    /// Bare booleans would never produce useful per-field diagnostics.
    #[error("schema must be a JSON object, got {0}")]
    NotAnObject(&'static str),

    /// `jsonschema::validator_for` rejected the raw value. The inner
    /// string is the validator's own error message; we keep it as a
    /// plain `String` rather than holding a borrowed
    /// `jsonschema::ValidationError<'_>` because that type's lifetime
    /// is bound to the input value, which is dropped before the error
    /// returns.
    #[error("schema compile failed: {0}")]
    Compile(String),
}

/// Error returned by [`crate::validate`] when args fail to match the
/// registered schema for the requested verb.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    /// The verb name was not present in the [`crate::ArgsRegistry`].
    /// This is a programmer error — every verb the engine knows how to
    /// apply must have its schema registered before dispatch.
    #[error("unknown verb: {verb:?}")]
    UnknownVerb {
        /// The verb name that was looked up.
        verb: String,
    },

    /// The args value did not satisfy the registered schema. `detail`
    /// is the first violation reported by `jsonschema`; subsequent
    /// violations are not enumerated because callers reject the whole
    /// args payload on the first failure anyway.
    #[error("schema violation: {detail}")]
    SchemaViolation {
        /// Human-readable description of the first failing keyword,
        /// e.g. `"\"foo\" is a required property"`.
        detail: String,
    },
}
