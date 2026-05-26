//! `schema` (§1.2) — sixty-sixth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/meta.md` §1.2)
//!
//! > CLI: `verbreel schema --target <project|command|effect> [--name <name>]`
//! > MCP: `schema`
//! > Args: `target: "project" | "command" | "effect"`, `name?: string`
//! > Returns (`data`): `{ schema: object; schema_id: string }`
//! > Errors: `E_UNKNOWN_TARGET`, `E_UNKNOWN_KIND`
//!
//! ## Returns JSON Schemas for the project graph, a command, or an effect.
//!
//! `schema` is read-only and does not read or mutate project state. Three
//! dispatch branches based on `target`:
//!
//! - `target=Project` → returns the embedded `project-schema.json`
//!   ([`crate::schemas::PROJECT_JSON`]) parsed as a JSON Value with
//!   `schema_id == "verbreel:project:v1"`. The `name` arg is accepted
//!   but ignored (spec is silent on the combination).
//! - `target=Command` → looks up `name` in [`crate::verbs::default_registry`]
//!   and returns a v1 vacuous schema (`{type: object, additionalProperties:
//!   true, title: "<verb> args"}`) with `schema_id ==
//!   "verbreel:command:<verb>:v1"`. Missing `name` is `MissingName`;
//!   unresolved name is `UnknownKind`.
//! - `target=Effect` → looks up `name` in
//!   [`crate::verbs::effect_list_available::bundled_effects`] and returns
//!   the same vacuous-schema shape with `title: "<kind> params"` and
//!   `schema_id == "verbreel:effect:<kind>:v1"`.
//!
//! ## Vacuous schemas (spec gap, v1 placeholder)
//!
//! Real per-command and per-effect JSON Schemas require a JSON-Schema
//! derivation crate (`schemars` + per-`Args` derive) wired through every
//! verb and effect module. v1 ships the placeholder shape so the
//! response wire-format is stable (`{schema, schema_id}`) and agents
//! that cache `schema_id` strings will see them resolve to real schemas
//! when the derivation infrastructure lands — without any wire-format
//! migration. The `:v1` suffix on every `schema_id` anticipates that
//! future schema-version bump.
//!
//! ## Error name disambiguation
//!
//! The verb's error type is named `SchemaError` (per spec naming). The
//! existing crate-root `SchemaError` re-export from
//! [`crate::verbs::validate_command`] is unrelated — it is a per-error
//! entry struct (`{path, message}`) inside the `errors[]` array of an
//! Invalid `validate_command` response. To keep both types accessible
//! without breaking downstream, this module's error type is re-exported
//! at the crate root as `SchemaVerbError`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::schemas;
use crate::verbs::effect_list_available::bundled_effects;

/// `target` discriminator for [`SchemaArgs`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SchemaTarget {
    /// Return the canonical project graph schema.
    Project,
    /// Return the schema for a registered verb's args.
    Command,
    /// Return the schema for a bundled effect's params.
    Effect,
}

/// Arguments for `schema`.
///
/// `project_id` is required for `Verb` trait compatibility; the impl
/// ignores it (the spec defines only `target` and `name?`). See
/// `list_capabilities` and `help` for the same accommodation pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaArgs {
    /// Required by the `Verb` trait shape; not read by the impl.
    pub project_id: ProjectId,

    /// Which schema to return (`project` / `command` / `effect`).
    pub target: SchemaTarget,

    /// Verb id (for `target=Command`) or effect kind (for `target=Effect`).
    /// Ignored for `target=Project`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Envelope `data` returned by `schema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaData {
    /// The JSON Schema as a parsed JSON object.
    pub schema: Value,
    /// Stable identifier for this schema. Format:
    /// - project → `"verbreel:project:v1"`
    /// - command → `"verbreel:command:<verb>:v1"`
    /// - effect  → `"verbreel:effect:<kind>:v1"`
    pub schema_id: String,
}

/// Verb-level failures for `schema`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SchemaError {
    /// `target` value did not match any [`SchemaTarget`] variant. Maps
    /// to `E_UNKNOWN_TARGET`. Carries the raw serde error string for
    /// diagnostics.
    #[error("schema: E_UNKNOWN_TARGET {0}")]
    UnknownTarget(String),

    /// `target` is `command`/`effect` but `name` was not supplied. The
    /// `target` field names which branch surfaced it. Maps to
    /// `BadArgs` at the `Verb` trait surface.
    #[error("schema: missing required `name` for target `{target}`")]
    MissingName {
        /// Target branch that needed `name` (`"command"` or `"effect"`).
        target: String,
    },

    /// `name` was supplied but did not resolve in the relevant registry
    /// (unknown verb id for `target=Command`; unknown effect kind for
    /// `target=Effect`). Maps to `E_UNKNOWN_KIND`.
    #[error("schema: E_UNKNOWN_KIND `{kind}`")]
    UnknownKind {
        /// The unresolved name.
        kind: String,
    },

    /// The embedded `project-schema.json` failed to parse as a JSON
    /// Value. Unreachable in practice (the file is checked-in spec
    /// content) but surfaced explicitly rather than panicked.
    #[error("schema: project schema embed corrupted: {0}")]
    EmbedCorrupted(String),
}

/// Build the `schema` data envelope for `args`.
fn build_data(args: &SchemaArgs) -> Result<SchemaData, SchemaError> {
    match args.target {
        SchemaTarget::Project => {
            let schema: Value = serde_json::from_str(schemas::PROJECT_JSON)
                .map_err(|err| SchemaError::EmbedCorrupted(err.to_string()))?;
            Ok(SchemaData {
                schema,
                schema_id: "verbreel:project:v1".to_string(),
            })
        }
        SchemaTarget::Command => {
            let name = args
                .name
                .as_deref()
                .ok_or_else(|| SchemaError::MissingName {
                    target: "command".to_string(),
                })?;
            let registry = crate::verbs::default_registry();
            if registry.get(name).is_none() {
                return Err(SchemaError::UnknownKind {
                    kind: name.to_string(),
                });
            }
            let schema = json!({
                "type": "object",
                "additionalProperties": true,
                "title": format!("{name} args"),
            });
            Ok(SchemaData {
                schema,
                schema_id: format!("verbreel:command:{name}:v1"),
            })
        }
        SchemaTarget::Effect => {
            let name = args
                .name
                .as_deref()
                .ok_or_else(|| SchemaError::MissingName {
                    target: "effect".to_string(),
                })?;
            let effects = bundled_effects();
            if !effects.iter().any(|e| e.kind == name) {
                return Err(SchemaError::UnknownKind {
                    kind: name.to_string(),
                });
            }
            let schema = json!({
                "type": "object",
                "additionalProperties": true,
                "title": format!("{name} params"),
            });
            Ok(SchemaData {
                schema,
                schema_id: format!("verbreel:effect:{name}:v1"),
            })
        }
    }
}

/// Build the (empty) RFC-6902 patch and data envelope for `schema`.
///
/// The patch is always `[]` and the warnings vec is always empty — this
/// is a read-only verb that ignores its `prior` project.
///
/// # Errors
///
/// Returns [`SchemaError`] when `target` requires a `name` that was not
/// supplied ([`SchemaError::MissingName`]), when the supplied `name`
/// does not resolve ([`SchemaError::UnknownKind`]), or when the
/// embedded project schema fails to parse ([`SchemaError::EmbedCorrupted`]).
pub fn compute_patch(
    _prior: &Project,
    args: &SchemaArgs,
) -> Result<(Value, Vec<Value>, SchemaData), SchemaError> {
    let data = build_data(args)?;
    Ok((json!([]), Vec::new(), data))
}

/// Rebuild the data envelope from `(args, post_state)`.
///
/// `schema` is project-agnostic, so the post-state is ignored — the
/// envelope depends solely on `args` and the embedded / registered
/// content of the current build.
///
/// # Errors
///
/// Returns [`ReconstructError`] when the inner build path surfaces any
/// [`SchemaError`] variant.
pub fn data_envelope_from_post_state(
    args: &SchemaArgs,
    _post_state: &Project,
) -> Result<SchemaData, ReconstructError> {
    build_data(args).map_err(|err| ReconstructError::Custom(err.to_string()))
}

impl From<SchemaError> for VerbError {
    fn from(value: SchemaError) -> Self {
        match value {
            SchemaError::UnknownTarget(_)
            | SchemaError::MissingName { .. }
            | SchemaError::UnknownKind { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
            SchemaError::EmbedCorrupted(_) => VerbError::Custom(value.to_string()),
        }
    }
}

/// The §0.8 verb for `schema`.
#[derive(Debug, Default)]
pub struct SchemaVerb;

impl Verb for SchemaVerb {
    fn verb(&self) -> &'static str {
        "schema"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: SchemaArgs = serde_json::from_value(args.clone()).map_err(|err| {
            // Distinguish a bad `target` enum value (E_UNKNOWN_TARGET) from
            // any other deserialization failure. Serde surfaces an enum
            // mismatch as `unknown variant \`X\`, expected one of …`;
            // that's the only path through `SchemaArgs` that yields a
            // `unknown variant` message since `name` is a string and the
            // remaining fields have no enum surface. Both branches map
            // to BadArgs at the trait level — the E_UNKNOWN_TARGET code
            // is embedded for diagnostics.
            let msg = err.to_string();
            let detail = if msg.contains("unknown variant") {
                format!("schema: E_UNKNOWN_TARGET args deserialize failed: {msg}")
            } else {
                format!("schema: args deserialize failed: {msg}")
            };
            VerbError::BadArgs { detail }
        })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!("schema: patch construction failed: {err}"))
        })?;
        let data = serde_json::to_value(&data)
            .map_err(|err| VerbError::Custom(format!("schema: data envelope failed: {err}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: SchemaArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "SchemaArgs",
            })?;
        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
