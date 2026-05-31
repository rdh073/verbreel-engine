//! Flag → args-JSON mapping (§0.2 coercion, §0.5 universal flags).
//!
//! `Engine::schema_for` returns a permissive `additionalProperties:true`
//! object for most verbs, so the CLI cannot drive a typed clap arg per
//! flag. Instead the verb tail (`raw[2..]`) is walked by hand into a
//! `serde_json::Map`, with two merged input modes:
//!
//! - repeated `--flag value` pairs (scalar-coerced), and
//! - a single `--args <JSON-object>` escape hatch.
//!
//! Precedence on key collision: **repeated `--flag` pairs win** over the
//! `--args` blob. Rationale: the explicit, closer-to-the-cursor flag is
//! the more specific intent; the blob is the bulk default. Documented
//! here because it is a load-bearing, otherwise-invisible choice.

use serde_json::{Map, Value};

use crate::duration::duration_str_to_ticks;

/// A parse-time failure that maps to §0.1 exit code 1 with an
/// envelope-shaped error on stdout (not a clap exit-2 parse failure —
/// the argv was well-formed, the *content* was rejected).
#[derive(Debug)]
pub struct CliArgError {
    /// §0.7 error code (e.g. `E_BAD_ARGS`).
    pub code: String,
    /// One-line human message.
    pub message: String,
}

impl CliArgError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }
}

/// Coerce a raw flag value string to the most specific JSON scalar.
///
/// Order (first that parses wins): duration-string → ticks (`i64`),
/// `i64`, `f64`, `bool` (`"true"`/`"false"` only), else `String`.
///
/// The duration probe runs first and is **best-effort by grammar match,
/// not by flag name**: the CLI has no per-flag schema to tell it which
/// flags are time-typed, so any value matching the duration grammar
/// (`5s` / `5000ms` / `1200000tk` / `HH:MM:SS.mmm`) is converted to its
/// integer tick count. A bare integer is never ambiguous — it is already
/// a tick count and stays an `i64` via the normal numeric branch (the
/// duration grammar deliberately does not match a bare integer).
///
/// NOTE (§0.2 deliberate footgun, review [P2-4]): under a permissive
/// `additionalProperties:true` schema a non-temporal value like
/// `--label 30s` would also coerce to ticks. This is the documented §0.2
/// "grammar match, not flag name" choice — accepted because the spec
/// pins duration syntax as load-bearing and the blast radius is narrow
/// (values ending `s`/`ms`/`tk` or matching `HH:MM:SS`). Not changed
/// here: a per-flag time-type schema is out of scope for the generic
/// dispatcher.
#[must_use]
pub fn coerce_scalar(raw: &str) -> Value {
    if let Some(ticks) = duration_str_to_ticks(raw) {
        return Value::from(ticks);
    }
    if let Ok(i) = raw.parse::<i64>() {
        return Value::from(i);
    }
    if let Ok(f) = raw.parse::<f64>() {
        return Value::from(f);
    }
    match raw {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        other => Value::String(other.to_string()),
    }
}

/// Result of walking the flag tail: the verb args (already merged) plus
/// the two CLI-only routing flags pulled out of the stream.
#[derive(Debug, Default)]
pub struct ParsedFlags {
    /// The merged verb-args object (repeated flags over `--args` blob).
    pub args: Map<String, Value>,
    /// The value of `--project`, if supplied (routing, not a verb arg).
    pub project: Option<String>,
}

/// Walk `tail` (the flags after `<noun> <verb>`) into a verb-args map.
///
/// - `--name value` → `name: coerce_scalar(value)`.
/// - `--name` with no following value (end of tail, or next token is a
///   `--flag`) → `name: true` (bare boolean flag, e.g. `--overwrite`).
/// - `--args <JSON>` → parsed object, merged **under** the repeated
///   flags (repeated flags win on collision).
/// - `--project <id>` → captured into [`ParsedFlags::project`], not the
///   args map (the caller injects it as `project_id` after context
///   resolution).
///
/// Flag names are normalized by stripping the leading `--`/`-`; a
/// positional token (no leading dash) at a value position is consumed as
/// the preceding flag's value, otherwise it is ignored (the generic
/// dispatcher has no positional verb args).
///
/// # Errors
///
/// Returns [`CliArgError`] (`E_BAD_ARGS`) if `--args` is present but its
/// value is missing or is not a JSON **object**.
pub fn parse_flags(tail: &[String]) -> Result<ParsedFlags, CliArgError> {
    let mut flag_args: Map<String, Value> = Map::new();
    let mut blob_args: Map<String, Value> = Map::new();
    let mut project: Option<String> = None;

    let mut i = 0;
    while i < tail.len() {
        let token = &tail[i];
        let Some(name) = token.strip_prefix("--").or_else(|| token.strip_prefix('-')) else {
            // Stray positional with no owning flag — the generic
            // dispatcher takes no positional verb args, so skip it.
            i += 1;
            continue;
        };

        // Does a value follow? A value is the next token that is not
        // itself a flag. `allow_hyphen_values` lets negative numbers
        // (`-10`) through as values, but a `--name`-shaped next token is
        // a new flag, so the current flag is bare-boolean.
        let value = tail.get(i + 1).filter(|next| !is_flag(next));

        match (name, value) {
            ("project", Some(v)) => {
                project = Some((*v).clone());
                i += 2;
            }
            ("args", Some(v)) => {
                let parsed: Value = serde_json::from_str(v).map_err(|e| {
                    CliArgError::new("E_BAD_ARGS", format!("--args is not valid JSON: {e}"))
                })?;
                let Value::Object(obj) = parsed else {
                    return Err(CliArgError::new(
                        "E_BAD_ARGS",
                        "--args must be a JSON object",
                    ));
                };
                for (k, val) in obj {
                    blob_args.insert(k, val);
                }
                i += 2;
            }
            ("args", None) => {
                return Err(CliArgError::new(
                    "E_BAD_ARGS",
                    "--args requires a JSON-object value",
                ));
            }
            ("project", None) => {
                return Err(CliArgError::new(
                    "E_BAD_ARGS",
                    "--project requires a project id value",
                ));
            }
            (name, Some(v)) => {
                flag_args.insert(name.to_string(), coerce_scalar(v));
                i += 2;
            }
            (name, None) => {
                flag_args.insert(name.to_string(), Value::Bool(true));
                i += 1;
            }
        }
    }

    // Merge: start from the blob, then let explicit flags overwrite.
    let mut merged = blob_args;
    for (k, v) in flag_args {
        merged.insert(k, v);
    }

    Ok(ParsedFlags {
        args: merged,
        project,
    })
}

/// A token is a "flag" if it starts with `--` (a long flag) or is a
/// single `-` (degenerate). A `-10`-shaped token is a hyphen *value*, not
/// a flag, so that `--from_tk -10` parses `-10` as the value.
///
/// NOTE (review P3): the flag-name strip above accepts `--`/`-`, but this
/// predicate only treats `--<alpha>` (and bare `-`) as flags, so a
/// single-dash `-v` or a `--<digit>` token at a *value* position is
/// consumed as a value while the same token at a *flag* position parses
/// as a flag. No corruption path (hyphen values are consumed before
/// reaching a flag position); the spec's CLI grammar uses only long
/// `--flag` names, so single-dash short flags are not a supported surface.
fn is_flag(token: &str) -> bool {
    token == "-"
        || token
            .strip_prefix("--")
            .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_alphabetic()))
}
