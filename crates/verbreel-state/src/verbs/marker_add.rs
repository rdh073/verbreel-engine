//! `marker.add` (§13.1) — fifth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/marker.md` §13.1, verbatim)
//!
//! > CLI: `verbreel marker add [--project <id>] --time_tk <int> --label <str> [--color <#rgba>] [--note <str>]`
//! > Marker args: `project_id: string`, `time_tk: integer`, `label: string`,
//! > `color?: string`, `note?: string`
//! > Returns (`data`): `{ marker: Marker }`
//! > Errors: `E_BAD_TIME` (negative or non-integer `time_tk`),
//! > `E_SCHEMA_VIOLATION` (malformed `color`, empty/oversize `label`).
//!
//! ## First add-op verb in the registry
//!
//! This is the first production verb that uses an RFC 6902 `add` op.
//! Prior verbs exclusively used `replace`.
//!
//! ## §0.8 reconstructor purity and ID minting
//!
//! `data_envelope_from_patch()` is the single source of truth for the
//! envelope:
//!
//! - `compute_patch()` mints `MarkerId::now()` once and stores it in the
//!   patch payload (`{ op: "add", path: "/markers/-", value: {...} }`).
//! - `reconstruct()` reads the id back from the same patch value,
//!   parses `Marker` from `patch[0].value`, and serializes it.
//! - The verb `compute_patch()` method also calls
//!   `data_envelope_from_patch()` on the patch it just built, so forward
//!   and replay paths share the same extraction path and cannot drift.
//!
//! > "a verb whose `data` carries a newly-minted UUID must record that
//! > UUID in the patch (as the `id` field of an `add` op). `randomUUID()`
//! > happens only once, during the original execution, and the patch is
//! > the record-of-truth for the value."

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;
use verbreel_types::{MarkerId, ProjectId};

use crate::marker::Marker;
use crate::newtypes::Color;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Maximum label length per schema (`maxLength: 256`).
pub const LABEL_MAX: usize = 256;

/// Maximum `note` length per schema (`maxLength: 4096`).
pub const NOTE_MAX: usize = 4096;

/// Runtime-fills `color` default when omitted.
pub const DEFAULT_MARKER_COLOR: &str = "#ffaa00ff";

/// Args for `marker.add`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkerAddArgs {
    /// Target project id. Strongly-typed [`ProjectId`] so malformed ids
    /// fail serde deserialize before patch construction.
    pub project_id: ProjectId,

    /// Marker time in ticks (`Tick` is `i64` in the crate types).
    pub time_tk: i64,

    /// Marker label. Required, `1..=256` characters (char-count).
    pub label: String,

    /// Optional marker color as a schema-legal color hex.
    /// Omitted entries are omitted on the wire in input and filled with
    /// `DEFAULT_MARKER_COLOR` by `compute_patch()`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,

    /// Optional note annotation.
    /// Omitted entries are omitted from patch output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Return value (`data`) shape for `marker.add`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkerAddData {
    /// Newly-added marker.
    pub marker: Marker,
}

/// Verb-level errors surfaced by [`compute_patch`]. All variants map to
/// [`VerbError::BadArgs`] because they are argument/contract-shape
/// failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MarkerAddError {
    /// `args.time_tk < 0`.
    #[error("marker.add: time_tk {time_tk} is before tick 0")] // E_BAD_TIME
    TimeBeforeProjectStart {
        /// Rejected `time_tk` value.
        time_tk: i64,
    },

    /// Empty marker label.
    #[error("marker.add: label must be non-empty")]
    LabelEmpty,

    /// `label` exceeds `LABEL_MAX` chars.
    #[error("marker.add: label has {actual} chars, exceeds maximum {max}")]
    LabelTooLong {
        /// Actual char-count in the input label.
        actual: usize,
        /// Maximum allowed by schema.
        max: usize,
    },

    /// `Color::try_from` rejected `args.color`.
    #[error("marker.add: color invalid: {detail}")]
    ColorInvalid {
        /// `Color` newtype rejection detail.
        detail: String,
    },

    /// `note` exceeds `NOTE_MAX` chars.
    #[error("marker.add: note has {actual} chars, exceeds maximum {max}")]
    NoteTooLong {
        /// Actual char-count in the input note.
        actual: usize,
        /// Maximum allowed by schema.
        max: usize,
    },
}

/// Build the RFC 6902 patch for `marker.add`.
///
/// Pure, no I/O, no clock reads, no RNG except the explicit
/// `MarkerId::now()` mint.
///
/// Returns:
///
/// - the patch as a `serde_json::Value` (`/markers/-` add op),
/// - an empty warnings vector.
///
/// # Errors
/// Returns [`MarkerAddError::TimeBeforeProjectStart`] if `time_tk < 0`.
/// Returns [`MarkerAddError::LabelEmpty`] if `label` is empty.
/// Returns [`MarkerAddError::LabelTooLong`] if `label` exceeds [`LABEL_MAX`].
/// Returns [`MarkerAddError::ColorInvalid`] if `color` is malformed.
/// Returns [`MarkerAddError::NoteTooLong`] if `note` exceeds [`NOTE_MAX`].
pub fn compute_patch(
    _prior: &Project,
    args: &MarkerAddArgs,
) -> Result<(Value, Vec<Value>), MarkerAddError> {
    if args.time_tk < 0 {
        return Err(MarkerAddError::TimeBeforeProjectStart {
            time_tk: args.time_tk,
        });
    }

    if args.label.is_empty() {
        return Err(MarkerAddError::LabelEmpty);
    }

    let label_len = args.label.chars().count();
    if label_len > LABEL_MAX {
        return Err(MarkerAddError::LabelTooLong {
            actual: label_len,
            max: LABEL_MAX,
        });
    }

    if let Some(note) = args.note.as_ref() {
        let note_len = note.chars().count();
        if note_len > NOTE_MAX {
            return Err(MarkerAddError::NoteTooLong {
                actual: note_len,
                max: NOTE_MAX,
            });
        }
    }

    let marker_id = MarkerId::now();

    let color = if let Some(color) = args.color.as_ref() {
        Color::try_from(color.clone())
            .map(|value| value.as_str().to_string())
            .map_err(|err| MarkerAddError::ColorInvalid {
                detail: err.to_string(),
            })?
    } else {
        DEFAULT_MARKER_COLOR.to_string()
    };

    let mut value_map = Map::new();
    value_map.insert("id".to_string(), Value::String(marker_id.to_string()));
    value_map.insert("time_tk".to_string(), Value::from(args.time_tk));
    value_map.insert("label".to_string(), Value::String(args.label.clone()));
    value_map.insert("color".to_string(), Value::String(color));
    if let Some(note) = args.note.clone() {
        value_map.insert("note".to_string(), Value::String(note));
    }

    let patch = json!([{
        "op": "add",
        "path": "/markers/-",
        "value": Value::Object(value_map),
    }]);

    Ok((patch, Vec::new()))
}

/// Rebuild `MarkerAddData` from recorded patch value.
///
/// The patch is asserted to be a single-op append to `/markers/-` with
/// payload in `value`.
///
/// # Errors
/// Returns [`ReconstructError::TypeMismatch`] when patch shape or required
/// fields are invalid.
/// Returns [`ReconstructError::MissingField`] when a required field is
/// absent.
/// Returns [`ReconstructError::Custom`] when marker payload fails to parse.
pub fn data_envelope_from_patch(patch: &Value) -> Result<MarkerAddData, ReconstructError> {
    let ops = patch.as_array().ok_or(ReconstructError::TypeMismatch {
        name: "patch",
        expected: "array",
    })?;

    if ops.len() != 1 {
        return Err(ReconstructError::TypeMismatch {
            name: "patch",
            expected: "single-element array",
        });
    }

    let op = ops
        .first()
        .and_then(Value::as_object)
        .ok_or(ReconstructError::TypeMismatch {
            name: "patch[0]",
            expected: "object",
        })?;

    let op_value = op
        .get("op")
        .and_then(Value::as_str)
        .ok_or(ReconstructError::MissingField {
            name: "patch[0].op",
        })?;
    if op_value != "add" {
        return Err(ReconstructError::TypeMismatch {
            name: "patch[0].op",
            expected: "add",
        });
    }

    let op_path = op
        .get("path")
        .and_then(Value::as_str)
        .ok_or(ReconstructError::MissingField {
            name: "patch[0].path",
        })?;
    if op_path != "/markers/-" {
        return Err(ReconstructError::TypeMismatch {
            name: "patch[0].path",
            expected: "/markers/-",
        });
    }

    let value = op
        .get("value")
        .cloned()
        .ok_or(ReconstructError::MissingField {
            name: "patch[0].value",
        })?;

    let marker =
        serde_json::from_value(value).map_err(|err| ReconstructError::Custom(err.to_string()))?;

    Ok(MarkerAddData { marker })
}

/// Funnel marker-add argument failures into the verb-layer error
/// taxonomy.
impl From<MarkerAddError> for VerbError {
    fn from(value: MarkerAddError) -> Self {
        match value {
            MarkerAddError::TimeBeforeProjectStart { .. }
            | MarkerAddError::LabelEmpty
            | MarkerAddError::LabelTooLong { .. }
            | MarkerAddError::ColorInvalid { .. }
            | MarkerAddError::NoteTooLong { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `marker.add`.
#[derive(Debug, Default)]
pub struct MarkerAddVerb;

impl Verb for MarkerAddVerb {
    fn verb(&self) -> &'static str {
        "marker.add"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: MarkerAddArgs =
            serde_json::from_value(args.clone()).map_err(|e| VerbError::BadArgs {
                detail: format!("marker.add: args deserialize failed: {e}"),
            })?;

        let (patch_value, _warnings) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|e| {
                VerbError::Custom(format!("marker.add: patch construction failed: {e}"))
            })?;

        let envelope = data_envelope_from_patch(&patch_value).map_err(|e| {
            VerbError::Custom(format!(
                "marker.add: data envelope reconstruction failed: {e}"
            ))
        })?;
        let data = serde_json::to_value(&envelope).map_err(|e| {
            VerbError::Custom(format!("marker.add: data envelope serialize failed: {e}"))
        })?;

        Ok((patch, data, Vec::new()))
    }

    fn reconstruct(
        &self,
        _args: &Value,
        patch: &Value,
        _warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let envelope = data_envelope_from_patch(patch)?;
        serde_json::to_value(&envelope).map_err(|e| ReconstructError::Custom(e.to_string()))
    }
}
