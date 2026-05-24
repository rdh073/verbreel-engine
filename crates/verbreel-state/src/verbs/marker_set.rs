//! `marker.set` (§13.2) — sixth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/marker.md` §13.2, verbatim)
//!
//! > Updates one or more `Marker` fields. Omitted fields are left unchanged.
//! > To clear or reset a field, use the null-semantics rules below.
//! >
//! > | Field | Pass `null` | CLI shorthand |
//! > |---|---|---|
//! > | `color` | reverts to the schema default `"#ffaa00ff"` |
//! > | `note` | removes the field from the marker |
//! >
//! > **Args**: `project_id: string`, `marker: string`, `time_tk?: integer`,
//! > `label?: string`, `color?: string | null`, `note?: string | null`
//! > **Returns** (`data`): `{ marker: Marker }`
//! >
//! > **Errors**: `E_NOT_FOUND`, `E_BAD_TIME`,
//! > `E_SCHEMA_VIOLATION`, `E_ARGS_INCOMPATIBLE`.
//!
//! ## Marker partial updates
//!
//! This verb supports partial updates on `time_tk` and `label` (`Option<T>`
//! fields) and tri-state null-semantics on `color` and `note`
//! (`Option<Option<T>>`):
//!
//! - omitted (`None`) → field unchanged;
//! - present as JSON `null` (`Some(None)`) → restore schema default / remove,
//!   depending on field;
//! - present as value (`Some(Some(v))`) → validate and store value.
//!
//! The first tri-state verb in the production sequence uses a small
//! explicit `deserialize_with` shim so we get:
//!
//! - absent → `None`
//! - `null` → `Some(None)`
//! - `"value"` → `Some(Some("value"))`
//!
//! This keeps the intent explicit and avoids future serde
//! behavior/version surprises.
use serde::de::Deserializer;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{MarkerId, ProjectId};

use crate::marker::Marker;
use crate::newtypes::Color;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::verbs::marker_add::{
    DEFAULT_MARKER_COLOR as MARKER_ADD_DEFAULT_COLOR, LABEL_MAX, NOTE_MAX,
};

/// Runtime-filled `color` default per `$defs/Marker`.
pub const DEFAULT_MARKER_COLOR: &str = MARKER_ADD_DEFAULT_COLOR;

/// Args for `marker.set`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkerSetArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Marker id (`UUIDv7`, as string, not a selector).
    pub marker: String,

    /// Optional timestamp update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_tk: Option<i64>,

    /// Optional label update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    /// Tri-state `color` update.
    ///
    /// - `None`: field unchanged.
    /// - `Some(None)`: revert to [`DEFAULT_MARKER_COLOR`].
    /// - `Some(Some(value))`: validate `value` and store canonical lowercase.
    #[serde(
        default,
        deserialize_with = "deserialize_double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub color: Option<Option<String>>,

    /// Tri-state `note` update.
    ///
    /// - `None`: field unchanged.
    /// - `Some(None)`: remove `note`.
    /// - `Some(Some(value))`: validate length and store.
    #[serde(
        default,
        deserialize_with = "deserialize_double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub note: Option<Option<String>>,
}

#[allow(clippy::option_option)]
fn deserialize_double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: Deserializer<'de>,
{
    let value = Option::<T>::deserialize(deserializer)?;
    Ok(Some(value))
}

/// Envelope `data` shape returned by `marker.set`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkerSetData {
    /// The post-state marker.
    pub marker: Marker,
}

/// Verb-level validation failures surfaced by [`compute_patch`]. All map to
/// [`VerbError::BadArgs`] for kernel error-translation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MarkerSetError {
    /// Marker id is not a valid `UUIDv7` string.
    #[error("marker.set: marker {detail} is invalid UUIDv7")]
    MarkerIdInvalid {
        /// Raw invalid id payload from args.
        detail: String,
    },

    /// No marker with the requested id exists in `prior.markers`.
    #[error("marker.set: marker `{marker_id}` not found in project")]
    MarkerNotFound {
        /// Requested marker id string.
        marker_id: String,
    },

    /// `time_tk < 0`.
    #[error("marker.set: time_tk {time_tk} is before tick 0")]
    TimeBeforeProjectStart {
        /// Rejected value.
        time_tk: i64,
    },

    /// Empty label.
    #[error("marker.set: label must be non-empty")]
    LabelEmpty,

    /// Label exceeds [`LABEL_MAX`].
    #[error("marker.set: label has {actual} chars, exceeds maximum {max}")]
    LabelTooLong {
        /// Actual label length (char count).
        actual: usize,
        /// Cap (`LABEL_MAX`).
        max: usize,
    },

    /// Invalid `color` value.
    #[error("marker.set: color invalid: {detail}")]
    ColorInvalid {
        /// `Color`-parser details.
        detail: String,
    },

    /// `note` exceeds [`NOTE_MAX`].
    #[error("marker.set: note has {actual} chars, exceeds maximum {max}")]
    NoteTooLong {
        /// Actual note length (char count).
        actual: usize,
        /// Cap (`NOTE_MAX`).
        max: usize,
    },
}

/// Build the RFC 6902 patch for `marker.set`.
///
/// Returns:
///
/// - patch as `serde_json::Value` (`replace` on `/markers/<idx>`),
/// - empty warnings vector.
///
/// # Errors
/// Returns [`MarkerSetError`] when marker id parsing fails, marker id is not
/// present in `prior`, `time_tk` is negative, required schema checks fail
/// (empty/too-long label or note, invalid color).
///
/// # Panics
/// Panics if serializing the patched `Marker` into JSON fails, which should
/// never happen for this in-memory typed value.
pub fn compute_patch(
    prior: &Project,
    args: &MarkerSetArgs,
) -> Result<(Value, Vec<Value>), MarkerSetError> {
    args.marker
        .parse::<MarkerId>()
        .map_err(|err| MarkerSetError::MarkerIdInvalid {
            detail: err.to_string(),
        })?;

    let marker_idx = prior
        .markers
        .iter()
        .position(|marker| marker.id.to_string() == args.marker)
        .ok_or_else(|| MarkerSetError::MarkerNotFound {
            marker_id: args.marker.clone(),
        })?;

    let mut marker = prior.markers[marker_idx].clone();

    if let Some(time_tk) = args.time_tk {
        if time_tk < 0 {
            return Err(MarkerSetError::TimeBeforeProjectStart { time_tk });
        }
        marker.time_tk = verbreel_types::Tick::new(time_tk);
    }

    if let Some(label) = args.label.clone() {
        if label.is_empty() {
            return Err(MarkerSetError::LabelEmpty);
        }
        let label_len = label.chars().count();
        if label_len > LABEL_MAX {
            return Err(MarkerSetError::LabelTooLong {
                actual: label_len,
                max: LABEL_MAX,
            });
        }
        marker.label = label;
    }

    if let Some(color_state) = args.color.clone() {
        match color_state {
            Some(value) => {
                marker.color = Color::try_from(value)
                    .map(|color| color.as_str().to_string())
                    .map_err(|err| MarkerSetError::ColorInvalid {
                        detail: err.to_string(),
                    })?;
            }
            None => {
                marker.color = DEFAULT_MARKER_COLOR.to_string();
            }
        }
    }

    if let Some(note_state) = args.note.clone() {
        match note_state {
            Some(note) => {
                let note_len = note.chars().count();
                if note_len > NOTE_MAX {
                    return Err(MarkerSetError::NoteTooLong {
                        actual: note_len,
                        max: NOTE_MAX,
                    });
                }
                marker.note = Some(note);
            }
            None => {
                marker.note = None;
            }
        }
    }

    let marker_value = serde_json::to_value(&marker).expect("marker serializes");
    let patch = json!([{
        "op": "replace",
        "path": format!("/markers/{marker_idx}"),
        "value": marker_value,
    }]);

    Ok((patch, Vec::new()))
}

/// Build the envelope `data` from post-state.
///
/// Locates the same marker id in `post_state.markers` and returns the full marker.
///
/// # Errors
/// Returns [`ReconstructError::TypeMismatch`] when `args.marker` is not a `UUIDv7`.
/// Returns [`ReconstructError::PostStateMissing`] when the marker id is absent
/// from `post_state`.
pub fn data_envelope_from_post_state(
    args: &MarkerSetArgs,
    post_state: &Project,
) -> Result<MarkerSetData, ReconstructError> {
    let marker_id: MarkerId = args
        .marker
        .parse()
        .map_err(|_| ReconstructError::TypeMismatch {
            name: "marker",
            expected: "UUIDv7",
        })?;

    let marker = post_state
        .markers
        .iter()
        .find(|marker| marker.id == marker_id)
        .ok_or_else(|| ReconstructError::PostStateMissing {
            detail: format!("marker `{marker_id}` missing in post_state"),
        })?
        .clone();

    Ok(MarkerSetData { marker })
}

/// Funnel argument/validation failures into verb-layer error tax.
impl From<MarkerSetError> for VerbError {
    fn from(value: MarkerSetError) -> Self {
        match value {
            MarkerSetError::MarkerIdInvalid { .. }
            | MarkerSetError::MarkerNotFound { .. }
            | MarkerSetError::TimeBeforeProjectStart { .. }
            | MarkerSetError::LabelEmpty
            | MarkerSetError::LabelTooLong { .. }
            | MarkerSetError::ColorInvalid { .. }
            | MarkerSetError::NoteTooLong { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `marker.set`.
#[derive(Debug, Default)]
pub struct MarkerSetVerb;

impl Verb for MarkerSetVerb {
    fn verb(&self) -> &'static str {
        "marker.set"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: MarkerSetArgs =
            serde_json::from_value(args.clone()).map_err(|e| VerbError::BadArgs {
                detail: format!("marker.set: args deserialize failed: {e}"),
            })?;

        let (patch_value, _warnings) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|e| {
                VerbError::Custom(format!("marker.set: patch construction failed: {e}"))
            })?;

        let post_state = prior.apply(&patch).map_err(|e| {
            VerbError::Custom(format!("marker.set: patch apply to prior failed: {e}"))
        })?;

        let data = data_envelope_from_post_state(&typed, &post_state)
            .and_then(|envelope| {
                serde_json::to_value(&envelope).map_err(|e| ReconstructError::Custom(e.to_string()))
            })
            .map_err(|e: ReconstructError| {
                VerbError::Custom(format!(
                    "marker.set: data envelope reconstruction failed: {e}"
                ))
            })?;

        Ok((patch, data, Vec::new()))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let args: MarkerSetArgs = serde_json::from_value(args.clone()).map_err(|_| {
            ReconstructError::Custom("marker.set: args deserialize failed".to_string())
        })?;
        let envelope = data_envelope_from_post_state(&args, post_state)?;
        serde_json::to_value(&envelope).map_err(|e| ReconstructError::Custom(e.to_string()))
    }
}
