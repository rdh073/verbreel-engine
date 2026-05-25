//! `clip.list` (§5.14) — twenty-fourth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/clip.md` §5.14, verbatim)
//!
//! > Lists clips, optionally scoped to a single track and/or to clips
//! > overlapping a given timeline tick.
//!
//! ## Read-only verb
//!
//! `clip.list` does not mutate project state; the patch is always
//! `[]`, no warnings are returned, and `data` carries the sorted clip list.
//!
//! ## Ordering
//!
//! Clips are sorted by `track_id` (lexicographic string order),
//! with `track_position_tk` ascending as the secondary key.
//! The result ordering is deterministic and stable.
use crate::clip::Clip;
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ProjectId, TrackId};

/// Args for `clip.list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipListArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Optional track selector (`UUIDv7`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,

    /// Optional timeline tick filter with half-open overlap semantics:
    /// `track_position_tk <= at_tk < track_position_tk + duration`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_tk: Option<i64>,
}

/// Envelope `data` returned by `clip.list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipListData {
    /// Sorted and filtered clip list.
    pub clips: Vec<Clip>,
}

/// Verb-level argument validation failures for `clip.list`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClipListError {
    /// `args.track` is not parseable as a `TrackId`.
    #[error("clip.list: `track` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// A track filter was provided but no matching track exists.
    #[error("clip.list: track `{track_id}` not found")]
    TrackNotFound {
        /// Track id string from the request.
        track_id: String,
    },
}

/// Compute timeline duration in ticks from source slice and playback speed.
#[must_use]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::cast_possible_truncation)]
pub fn clip_timeline_duration_tk(clip: &Clip) -> i64 {
    let source = clip.source_out_tk.0 - clip.source_in_tk.0;
    let speed = clip.speed.max(0.001);
    ((source as f64) / speed).ceil() as i64
}

fn collect_clips(
    project: &Project,
    track_filter: Option<&TrackId>,
    at_tk: Option<i64>,
) -> Vec<Clip> {
    let mut collected: Vec<(TrackId, Clip)> = Vec::new();

    for track in &project.tracks {
        if let Some(tid) = track_filter
            && &track.id != tid
        {
            continue;
        }

        for clip in &track.clips {
            if let Some(at) = at_tk {
                let pos = clip.track_position_tk.0;
                let dur = clip_timeline_duration_tk(clip);
                if !(pos <= at && at < pos + dur) {
                    continue;
                }
            }

            collected.push((track.id, clip.clone()));
        }
    }

    collected.sort_by(|(ta, ca), (tb, cb)| {
        ta.to_string()
            .cmp(&tb.to_string())
            .then_with(|| ca.track_position_tk.0.cmp(&cb.track_position_tk.0))
    });

    collected.into_iter().map(|(_, clip)| clip).collect()
}

/// Build the RFC-6902 patch for `clip.list`.
///
/// # Errors
///
/// - [`ClipListError::BadSelector`] when `track` is not a valid track id.
/// - [`ClipListError::TrackNotFound`] when a `track` filter is provided but no
///   track exists in `prior`.
pub fn compute_patch(
    prior: &Project,
    args: &ClipListArgs,
) -> Result<(Value, Vec<Value>, ClipListData), ClipListError> {
    let track_filter: Option<TrackId> = match &args.track {
        Some(s) => Some(
            s.parse::<TrackId>()
                .map_err(|err| ClipListError::BadSelector {
                    detail: err.to_string(),
                })?,
        ),
        None => None,
    };

    if let Some(tid) = track_filter.as_ref()
        && !prior.tracks.iter().any(|track| &track.id == tid)
    {
        return Err(ClipListError::TrackNotFound {
            track_id: args.track.clone().unwrap_or_default(),
        });
    }

    let clips = collect_clips(prior, track_filter.as_ref(), args.at_tk);

    Ok((json!([]), Vec::new(), ClipListData { clips }))
}

/// Rebuilds the data envelope from `(args, post_state)`.
///
/// # Errors
///
/// - [`ReconstructError::TypeMismatch`] when `args.track` is not a valid track
///   id.
/// - [`ReconstructError::PostStateMissing`] when the requested track id does not
///   exist in `post_state`.
pub fn data_envelope_from_post_state(
    args: &ClipListArgs,
    post_state: &Project,
) -> Result<ClipListData, ReconstructError> {
    let track_filter = match &args.track {
        Some(s) => Some(
            s.parse::<TrackId>()
                .map_err(|_| ReconstructError::TypeMismatch {
                    name: "args.track",
                    expected: "UUIDv7 TrackId string",
                })?,
        ),
        None => None,
    };

    if let Some(tid) = track_filter.as_ref()
        && !post_state.tracks.iter().any(|track| &track.id == tid)
    {
        return Err(ReconstructError::PostStateMissing {
            detail: format!("track `{tid}` not found"),
        });
    }

    let clips = collect_clips(post_state, track_filter.as_ref(), args.at_tk);
    Ok(ClipListData { clips })
}

impl From<ClipListError> for VerbError {
    fn from(value: ClipListError) -> Self {
        match value {
            ClipListError::BadSelector { .. } | ClipListError::TrackNotFound { .. } => {
                VerbError::BadArgs {
                    detail: value.to_string(),
                }
            }
        }
    }
}

/// The §0.8 verb for `clip.list`.
#[derive(Debug, Default)]
pub struct ClipListVerb;

impl Verb for ClipListVerb {
    fn verb(&self) -> &'static str {
        "clip.list"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipListArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.list: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("clip.list: patch construction failed: {err}"))
            })?;

        let data = serde_json::to_value(&data)
            .map_err(|err| VerbError::Custom(format!("clip.list: data envelope failed: {err}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: ClipListArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ClipListArgs",
            })?;

        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
