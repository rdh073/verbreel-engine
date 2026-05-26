//! `project.info` (§2.4) — fifty-seventh production verb in the engine.
//!
//! ## Spec quote (`spec/commands/project.md` §2.4, verbatim)
//!
//! > Compact summary (no clip-level detail).
//! >
//! > ```ts
//! > {
//! >   id: string; name: string; path: string;
//! >   canvas: { width: integer; height: integer };  // deliberate trim
//! >   fps_num: integer; fps_den: integer;
//! >   duration_tk: integer;
//! >   track_counts: { video: integer; audio: integer; text: integer; effect: integer };
//! >   asset_count: integer;
//! >   event_count: integer;
//! >   updated_at: string;
//! > }
//! > ```
//!
//! ## Read-only verb
//!
//! `project.info` does not mutate project state; the patch is always
//! `[]`, no warnings are returned, and `data` carries the compact
//! summary envelope above.
//!
//! ## Canvas trim (intentional)
//!
//! Per spec §2.4 "Canvas trim", the `canvas` field intentionally carries
//! only `{ width, height }` — `background`, `pixel_aspect_num`, and
//! `pixel_aspect_den` are stripped to keep the summary compact. Agents
//! needing the full canvas use `describe project:<id>` instead.
//!
//! ## Deferred fields (this slice)
//!
//! Two spec fields are emitted as fixed placeholders in this slice and
//! tracked as deferred work:
//!
//! - **`path`** — the on-disk project root path. The `Project` graph
//!   itself does not carry this; it is owned by `ProjectStore` (the
//!   load-path side of lifecycle). Wiring `ProjectStore::project_path`
//!   through the read-only verb surface is a separate slice — until
//!   then we emit `""` (empty string) and the §0.8 reconstructor
//!   round-trips it unchanged.
//! - **`event_count`** — the total line count of `events.jsonl`,
//!   including apply/undo/redo events. Counting this requires touching
//!   the event-log file I/O surface, which read-only verbs deliberately
//!   avoid in this slice (the verb stays a pure function of the in-memory
//!   graph). Emitted as `0` until the storage-side counter lands.
//!
//! Both deferred values are deterministic per the §0.8 reconstructor
//! contract (the post-state graph carries neither, so reconstructing the
//! same constants from the same args + post-state is trivially pure).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;

/// Arguments for `project.info`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfoArgs {
    /// Target project id.
    pub project_id: ProjectId,
}

/// Trimmed canvas envelope returned by `project.info` (`width` + `height`
/// only). The full canvas shape lives in [`crate::canvas::Canvas`]; per
/// spec §2.4 the secondary fields are intentionally stripped here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInfoCanvas {
    /// Canvas width in pixels.
    pub width: u32,

    /// Canvas height in pixels.
    pub height: u32,
}

/// Track-count breakdown by [`TrackKind`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInfoTrackCounts {
    /// Number of `TrackKind::Video` tracks.
    pub video: u32,
    /// Number of `TrackKind::Audio` tracks.
    pub audio: u32,
    /// Number of `TrackKind::Text` tracks.
    pub text: u32,
    /// Number of `TrackKind::Effect` tracks.
    pub effect: u32,
}

/// Envelope returned by `project.info`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInfoData {
    /// Project id (as a string per the spec's `id: string` shape).
    pub id: String,
    /// Human display name.
    pub name: String,
    /// On-disk project root path. **Deferred** — see module-level docs.
    pub path: String,
    /// Trimmed canvas envelope (`width` + `height` only).
    pub canvas: ProjectInfoCanvas,
    /// Frame-rate numerator.
    pub fps_num: u32,
    /// Frame-rate denominator.
    pub fps_den: u32,
    /// Total project duration in ticks.
    pub duration_tk: i64,
    /// Track-count breakdown by kind.
    pub track_counts: ProjectInfoTrackCounts,
    /// Number of registered assets.
    pub asset_count: u32,
    /// Total events.jsonl line count. **Deferred** — see module-level docs.
    pub event_count: u32,
    /// RFC 3339 timestamp of the last in-memory mutation.
    pub updated_at: String,
}

/// Verb-level error type for `project.info`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProjectInfoError {
    /// `project.info` has no runtime error variants.
    #[error("project.info: unreachable (no error variants)")]
    Unreachable,
}

fn count_tracks(prior: &Project) -> ProjectInfoTrackCounts {
    let mut counts = ProjectInfoTrackCounts {
        video: 0,
        audio: 0,
        text: 0,
        effect: 0,
    };
    for track in &prior.tracks {
        match track.kind {
            TrackKind::Video => counts.video += 1,
            TrackKind::Audio => counts.audio += 1,
            TrackKind::Text => counts.text += 1,
            TrackKind::Effect => counts.effect += 1,
        }
    }
    counts
}

fn build_envelope(prior: &Project) -> ProjectInfoData {
    let asset_count = u32::try_from(prior.assets.len()).unwrap_or(u32::MAX);

    ProjectInfoData {
        id: prior.id.to_string(),
        name: prior.name.clone(),
        // Deferred: §2.4 path requires project-load-path tracking which
        // is currently held by ProjectStore, not Project graph state —
        // wire through in follow-up.
        path: String::new(),
        canvas: ProjectInfoCanvas {
            width: prior.canvas.width,
            height: prior.canvas.height,
        },
        fps_num: prior.fps_num,
        fps_den: prior.fps_den,
        duration_tk: prior.duration_tk.get(),
        track_counts: count_tracks(prior),
        asset_count,
        // Deferred: §2.4 event_count requires events.jsonl line-count
        // I/O which read-only verbs avoid in this slice — wire through
        // in follow-up via a storage-side counter.
        event_count: 0,
        updated_at: prior.updated_at.clone(),
    }
}

/// Build the RFC-6902 patch for `project.info`.
///
/// # Errors
///
/// No runtime errors are expected from this verb itself.
pub fn compute_patch(
    prior: &Project,
    _args: &ProjectInfoArgs,
) -> Result<(Value, Vec<Value>, ProjectInfoData), ProjectInfoError> {
    Ok((json!([]), Vec::new(), build_envelope(prior)))
}

/// Build the data envelope from `(args, post_state)`.
///
/// # Errors
///
/// Reuses [`compute_patch`] logic — no runtime error variants.
pub fn data_envelope_from_post_state(
    args: &ProjectInfoArgs,
    post_state: &Project,
) -> Result<ProjectInfoData, ReconstructError> {
    let (_, _, data) =
        compute_patch(post_state, args).map_err(|e| ReconstructError::Custom(e.to_string()))?;
    Ok(data)
}

impl From<ProjectInfoError> for VerbError {
    fn from(value: ProjectInfoError) -> Self {
        match value {
            ProjectInfoError::Unreachable => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `project.info`.
#[derive(Debug, Default)]
pub struct ProjectInfoVerb;

impl Verb for ProjectInfoVerb {
    fn verb(&self) -> &'static str {
        "project.info"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ProjectInfoArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("project.info: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!("project.info: patch construction failed: {err}"))
        })?;
        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!("project.info: data envelope failed: {err}"))
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
        let typed: ProjectInfoArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ProjectInfoArgs",
            })?;
        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
