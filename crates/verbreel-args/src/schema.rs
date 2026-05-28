//! [`Schema`] — pre-compiled JSON Schema for one verb's args.
//!
//! A [`Schema`] owns:
//!
//! 1. The raw `serde_json::Value` the caller supplied, so the schema
//!    can be re-serialised (round-trip, debug dumps,
//!    spec-introspection) without going back through the compiled
//!    validator's internal representation.
//! 2. A `jsonschema::Validator` built once at construction time. This
//!    keeps the hot `validate` path free of compilation cost.
//!
//! Construction is the *only* place schema well-formedness is checked.
//! Past construction, every call to [`Schema::validator`] hands out the
//! same compiled artifact.

use serde_json::Value;

use crate::error::SchemaError;

/// A pre-compiled JSON Schema for one verb's args.
///
/// Build via [`Schema::from_value`]. Borrow the raw schema via
/// [`Schema::as_value`] (for serialisation / round-trip) and the
/// compiled validator via [`Schema::validator`] (for runtime
/// validation).
///
/// `Schema` is intentionally NOT `Clone` — `jsonschema::Validator`
/// does not implement `Clone` and re-compiling on every clone would
/// defeat the whole point of caching. Hand out shared references
/// instead.
#[derive(Debug)]
pub struct Schema {
    raw: Value,
    validator: jsonschema::Validator,
}

impl Schema {
    /// Build a [`Schema`] from a raw JSON Schema value.
    ///
    /// The value must be a JSON object (per the
    /// [`SchemaError::NotAnObject`] contract) and must be a valid JSON
    /// Schema that `jsonschema::validator_for` can compile.
    ///
    /// # Errors
    ///
    /// - [`SchemaError::NotAnObject`] — the value is a boolean,
    ///   number, string, array, or null.
    /// - [`SchemaError::Compile`] — `jsonschema::validator_for`
    ///   rejected the value. The wrapped string carries the
    ///   underlying error message.
    pub fn from_value(raw: Value) -> Result<Self, SchemaError> {
        if !raw.is_object() {
            return Err(SchemaError::NotAnObject(kind_of(&raw)));
        }
        let validator =
            jsonschema::validator_for(&raw).map_err(|e| SchemaError::Compile(e.to_string()))?;
        Ok(Self { raw, validator })
    }

    /// Borrow the raw schema value the [`Schema`] was constructed from.
    ///
    /// Useful for round-tripping the schema back through `serde_json`
    /// (debug dumps, spec exports) without losing field order or the
    /// caller's original formatting choices.
    #[must_use]
    pub fn as_value(&self) -> &Value {
        &self.raw
    }

    /// Borrow the pre-compiled [`jsonschema::Validator`].
    ///
    /// Callers that want the first-error pattern should use
    /// [`crate::validate`] instead — this accessor is exposed for the
    /// rare advanced use case (iterating all violations, custom error
    /// shaping).
    #[must_use]
    pub fn validator(&self) -> &jsonschema::Validator {
        &self.validator
    }
}

/// Static label for the non-object JSON kinds. Used to fill
/// [`SchemaError::NotAnObject`] with a `&'static str` rather than
/// allocating a fresh `String` per error.
fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
