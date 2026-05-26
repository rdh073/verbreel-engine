//! `timeline.snapshot` (§12.1) — sixty-first production verb in the engine.
//!
//! ## Spec quote (`spec/commands/timeline.md` §12.1, condensed)
//!
//! > Returns a token identifying the current head of the event log. The
//! > token is `event_id` of the latest event (or the special string
//! > `"empty"` for a project with no events).
//! >
//! > **Args**: `project_id: string`
//! > **Returns** (`data`): `{ event_id: string; project_hash: string }`
//! >
//! > `project_hash` is the SHA-256 of the canonical JSON of `project.json`
//! > with `Project.updated_at` and `Project.last_saved_event_id` excluded
//! > from the serialization — see §0.5.2's "`project_hash` field
//! > projection" rule.
//!
//! ## Read-only verb
//!
//! `timeline.snapshot` does not mutate project state; the patch is always
//! `[]`, no warnings are returned, and `data` carries the two-field
//! envelope above.
//!
//! ## Delegation
//!
//! `project_hash` is delegated to [`verbreel_canon::project_hash`], which
//! already strips `updated_at` + `last_saved_event_id` per §0.5.2. This
//! verb is a thin wrapper: serialize `Project` → `Value`, hand off to
//! canon.
//!
//! ## Deferred field (this slice)
//!
//! - **`event_id` for applied-but-unsaved events**: spec §12.1 asks for
//!   "current head of the event log". The actual head for an
//!   applied-but-not-yet-saved project lives on
//!   `ProjectStore.last_applied_event_id`
//!   (`crates/verbreel-state/src/lifecycle.rs:342`), NOT on the
//!   [`Project`] graph. The [`Verb`] trait operates on `&Project` alone,
//!   so this slice approximates with `Project.last_saved_event_id` and
//!   emits the literal `"empty"` sentinel when it is `None`.
//!
//!   This mirrors the precedent set by `project.info` (see
//!   [`crate::verbs::project_info`] — `event_count = 0` is emitted with
//!   the same justification: the count lives on the lifecycle layer, not
//!   on the graph). A future slice will either extend the [`Verb`] trait
//!   with a context argument or wire `ProjectStore.last_applied_event_id`
//!   through a parallel read surface.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_canon::CanonError;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Arguments for `timeline.snapshot`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineSnapshotArgs {
    /// Target project id.
    pub project_id: ProjectId,
}

/// Envelope returned by `timeline.snapshot`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineSnapshotData {
    /// Event id of the latest event in `events.jsonl`, or the literal
    /// string `"empty"` for a project that has never been saved.
    ///
    /// Approximation: this slice sources from
    /// [`Project::last_saved_event_id`] — see the module-level deferral
    /// note. The `"empty"` sentinel is verbatim from spec §12.1 and
    /// accepted by `timeline.diff` / `timeline.history` as the
    /// beginning-of-history marker.
    pub event_id: String,

    /// SHA-256 of the canonical JSON of `project.json` with `updated_at`
    /// and `last_saved_event_id` stripped per §0.5.2. 64-char lowercase
    /// hex.
    pub project_hash: String,
}

/// Verb-level error type for `timeline.snapshot`.
#[derive(Debug, Error)]
pub enum TimelineSnapshotError {
    /// Argument deserialization failed (shape mismatch, missing required
    /// field, wrong type).
    #[error("timeline.snapshot: args deserialize failed: {0}")]
    BadArgs(String),

    /// `Project` failed to serialize to a JSON [`Value`] before being
    /// handed to [`verbreel_canon::project_hash`].
    #[error("timeline.snapshot: project serialize failed: {0}")]
    Serialize(String),

    /// [`verbreel_canon::project_hash`] returned an error
    /// (canonicalization failure — e.g. non-finite number in the graph).
    #[error("timeline.snapshot: project_hash failed: {0}")]
    Canon(#[from] CanonError),
}

fn build_envelope(prior: &Project) -> Result<TimelineSnapshotData, TimelineSnapshotError> {
    // Deferred: see module docs — head for applied-but-unsaved state
    // lives on ProjectStore.last_applied_event_id (lifecycle.rs:342),
    // not on Project. Mirrors project.info's event_count = 0 deferral.
    let event_id = match prior.last_saved_event_id {
        None => "empty".to_string(),
        Some(id) => id.to_string(),
    };

    let project_json = serde_json::to_value(prior)
        .map_err(|err| TimelineSnapshotError::Serialize(err.to_string()))?;
    let project_hash = verbreel_canon::project_hash(&project_json)?;

    Ok(TimelineSnapshotData {
        event_id,
        project_hash,
    })
}

/// Build the RFC-6902 patch and data envelope for `timeline.snapshot`.
///
/// The patch is always `[]` and the warnings vec is always empty — this
/// is a read-only verb.
///
/// # Errors
///
/// - [`TimelineSnapshotError::Serialize`] — `Project` failed to serialize
///   to `Value` before canonicalization.
/// - [`TimelineSnapshotError::Canon`] — [`verbreel_canon::project_hash`]
///   rejected the serialized project (e.g. non-finite number).
pub fn compute_patch(
    prior: &Project,
    _args: &TimelineSnapshotArgs,
) -> Result<(Value, Vec<Value>, TimelineSnapshotData), TimelineSnapshotError> {
    let data = build_envelope(prior)?;
    Ok((json!([]), Vec::new(), data))
}

/// Rebuild the data envelope from `(args, post_state)`.
///
/// For a read-only verb the post-state equals the pre-state, so the same
/// envelope drops out of [`build_envelope`].
///
/// # Errors
///
/// Returns [`ReconstructError::Custom`] wrapping a
/// [`TimelineSnapshotError`] string if canonicalization fails on the
/// post-state graph (which would be a verb-author bug at the §0.8 gate).
pub fn data_envelope_from_post_state(
    _args: &TimelineSnapshotArgs,
    post_state: &Project,
) -> Result<TimelineSnapshotData, ReconstructError> {
    build_envelope(post_state).map_err(|err| ReconstructError::Custom(err.to_string()))
}

impl From<TimelineSnapshotError> for VerbError {
    fn from(value: TimelineSnapshotError) -> Self {
        match value {
            TimelineSnapshotError::BadArgs(detail) => VerbError::BadArgs { detail },
            TimelineSnapshotError::Serialize(_) | TimelineSnapshotError::Canon(_) => {
                VerbError::Custom(value.to_string())
            }
        }
    }
}

/// The §0.8 verb for `timeline.snapshot`.
#[derive(Debug, Default)]
pub struct TimelineSnapshotVerb;

impl Verb for TimelineSnapshotVerb {
    fn verb(&self) -> &'static str {
        "timeline.snapshot"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TimelineSnapshotArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("timeline.snapshot: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!(
                "timeline.snapshot: patch construction failed: {err}"
            ))
        })?;
        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("timeline.snapshot: data envelope failed: {err}"))
        })?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: TimelineSnapshotArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "TimelineSnapshotArgs",
            })?;
        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
