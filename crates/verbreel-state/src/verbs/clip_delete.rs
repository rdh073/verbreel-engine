//! `clip.delete` (§5.5) — thirty-ninth production verb in the engine.
//!
//! ## Basic slice
//!
//! This slice implements non-ripple deletion only. The argument shape
//! accepts `ripple` and `ripple_scope` so the future ripple slice can
//! extend the same verb contract, but `ripple: true` and any supplied
//! `ripple_scope` currently return a schema violation.
//!
//! ## Reconstructor compatibility
//!
//! Deleted clips are absent from post-state, so `reconstruct()` cannot
//! derive the data envelope from post-state alone. The forward path
//! therefore emits one internal warning (`W_CLIP_DELETE_ENVELOPE`)
//! carrying every id list. The reconstructor reads that warning back
//! into [`ClipDeleteData`], matching the destructive-verb envelope
//! pattern used by `track.remove`.

use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ClipId, LinkGroupId, ProjectId};

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Maximum allowed batch length for `clips` (§0.8).
pub const CLIPS_MAX_BATCH: usize = 10_000;

/// Internal warning code carrying the destructive data envelope.
pub const W_CLIP_DELETE_ENVELOPE_CODE: &str = "W_CLIP_DELETE_ENVELOPE";

/// Warning code emitted when singleton cleanup clears a locked survivor.
pub const W_LINK_GROUP_CLEARED_ON_LOCKED_CODE: &str = "W_LINK_GROUP_CLEARED_ON_LOCKED";

const CLIPS_FIELD: &str = "clips";
const RIPPLE_FIELD: &str = "ripple";
const RIPPLE_SCOPE_FIELD: &str = "ripple_scope";
const SPLIT_BATCH_HINT: &str = "split the batch into smaller calls";
const RIPPLE_DEFERRED_HINT: &str = "ripple semantics deferred to follow-up";

/// Arguments for `clip.delete`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipDeleteArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Clip ids to remove, as bare `UUIDv7` strings.
    pub clips: Vec<String>,

    /// `true` downgrades missing ids from error to the data envelope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft: Option<bool>,

    /// Future ripple flag. `true` is deferred in this slice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ripple: Option<bool>,

    /// Future ripple scope. Any supplied value is deferred in this slice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ripple_scope: Option<String>,
}

impl ClipDeleteArgs {
    fn soft(&self) -> bool {
        self.soft.unwrap_or(false)
    }
}

/// Envelope returned by `clip.delete`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipDeleteData {
    /// IDs successfully removed, sorted by UUID string.
    pub removed_clip_ids: Vec<ClipId>,

    /// Missing IDs skipped under `soft=true`, sorted by UUID string.
    pub missing_clip_ids: Vec<ClipId>,

    /// Surviving clip ids whose `link_group` was cleared, sorted by UUID string.
    pub cleared_link_group_clip_ids: Vec<ClipId>,
}

/// Verb-level validation failures for `clip.delete`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClipDeleteError {
    /// `args.clips.len() > CLIPS_MAX_BATCH` or a deferred ripple field was supplied.
    #[error("clip.delete: schema violation on `{field}`: {hint}")]
    SchemaViolation {
        /// Field that violated this slice's accepted schema.
        field: &'static str,
        /// Actionable hint.
        hint: &'static str,
        /// Actual number of clip ids supplied, for batch-cap violations.
        actual: Option<usize>,
        /// Hard maximum, for batch-cap violations.
        max: Option<usize>,
    },

    /// Clip id failed bare-UUID parsing.
    #[error(
        "clip.delete: clip `{failed_target}` selector failed at index {failed_index}: {detail}"
    )]
    BadSelector {
        /// Input index where parsing failed.
        failed_index: usize,
        /// Raw offending string.
        failed_target: String,
        /// Parse failure detail.
        detail: String,
    },

    /// Missing clip in strict mode.
    #[error("clip.delete: clip `{failed_target}` not found at index {failed_index}")]
    NotFound {
        /// Input index where the missing clip was first requested.
        failed_index: usize,
        /// Missing clip id string.
        failed_target: String,
    },

    /// Target clip or its parent track is locked.
    #[error(
        "clip.delete: {kind} `{id}` is locked for clip `{failed_target}` at index {failed_index}"
    )]
    Locked {
        /// Input index where the locked clip was first requested.
        failed_index: usize,
        /// Requested clip id string.
        failed_target: String,
        /// Locked entity kind.
        kind: &'static str,
        /// Locked entity id.
        id: String,
    },
}

#[derive(Debug, Clone)]
struct RequestedClip {
    input_index: usize,
    clip_id: ClipId,
    raw: String,
}

#[derive(Debug, Clone)]
struct ClipLocation {
    track_idx: usize,
    clip_idx: usize,
    track_locked: bool,
    track_id: String,
    clip_locked: bool,
    clip_id: ClipId,
}

#[derive(Debug, Clone)]
struct LocatedClip {
    request: RequestedClip,
    location: ClipLocation,
}

#[derive(Debug, Clone)]
struct LinkGroupMember {
    track_idx: usize,
    clip_idx: usize,
    clip_id: ClipId,
    locked: bool,
    in_removal_set: bool,
}

/// Build the RFC-6902 patch for `clip.delete`.
///
/// # Errors
/// Returns [`ClipDeleteError`] for batch-size, deferred ripple,
/// selector, missing clip, or locked-target failures.
#[allow(clippy::too_many_lines)]
pub fn compute_patch(
    prior: &Project,
    args: &ClipDeleteArgs,
) -> Result<(Value, Vec<Value>, ClipDeleteData), ClipDeleteError> {
    if args.clips.len() > CLIPS_MAX_BATCH {
        return Err(ClipDeleteError::SchemaViolation {
            field: CLIPS_FIELD,
            hint: SPLIT_BATCH_HINT,
            actual: Some(args.clips.len()),
            max: Some(CLIPS_MAX_BATCH),
        });
    }

    if args.ripple == Some(true) {
        return Err(ClipDeleteError::SchemaViolation {
            field: RIPPLE_FIELD,
            hint: RIPPLE_DEFERRED_HINT,
            actual: None,
            max: None,
        });
    }

    if args.ripple_scope.is_some() {
        return Err(ClipDeleteError::SchemaViolation {
            field: RIPPLE_SCOPE_FIELD,
            hint: RIPPLE_DEFERRED_HINT,
            actual: None,
            max: None,
        });
    }

    let requested = parse_and_dedupe(args)?;
    if requested.is_empty() {
        let data = empty_data();
        return Ok((json!([]), vec![envelope_warning(&data)], data));
    }

    let prior_by_id = clip_locations(prior);
    let mut found = Vec::new();
    let mut missing_clip_ids = Vec::new();

    for request in requested {
        if let Some(location) = prior_by_id.get(&request.clip_id) {
            found.push(LocatedClip {
                request,
                location: location.clone(),
            });
            continue;
        }

        if args.soft() {
            missing_clip_ids.push(request.clip_id);
            continue;
        }

        return Err(ClipDeleteError::NotFound {
            failed_index: request.input_index,
            failed_target: request.raw,
        });
    }

    for located in &found {
        if located.location.clip_locked {
            return Err(ClipDeleteError::Locked {
                failed_index: located.request.input_index,
                failed_target: located.request.raw.clone(),
                kind: "clip",
                id: located.location.clip_id.to_string(),
            });
        }
        if located.location.track_locked {
            return Err(ClipDeleteError::Locked {
                failed_index: located.request.input_index,
                failed_target: located.request.raw.clone(),
                kind: "track",
                id: located.location.track_id.clone(),
            });
        }
    }

    let removal_ids: HashSet<ClipId> = found
        .iter()
        .map(|located| located.location.clip_id)
        .collect();
    let link_group_clears = lone_survivor_link_group_clears(prior, &removal_ids);
    let cleared_link_group_clip_ids =
        sort_ids(link_group_clears.iter().map(|member| member.clip_id));

    let removed_clip_ids = sort_ids(found.iter().map(|located| located.location.clip_id));
    let missing_clip_ids = sort_ids(missing_clip_ids);

    let data = ClipDeleteData {
        removed_clip_ids,
        missing_clip_ids,
        cleared_link_group_clip_ids,
    };

    let mut ops = Vec::new();

    let mut sorted_clears = link_group_clears;
    sorted_clears.sort_by_key(|member| (member.track_idx, member.clip_idx));
    for member in &sorted_clears {
        ops.push(json!({
            "op": "replace",
            "path": format!("/tracks/{}/clips/{}/link_group", member.track_idx, member.clip_idx),
            "value": null,
        }));
    }

    let mut removals: Vec<(usize, usize)> = found
        .iter()
        .map(|located| (located.location.track_idx, located.location.clip_idx))
        .collect();
    removals.sort_by_key(|entry| Reverse((entry.1, entry.0)));
    for (track_idx, clip_idx) in &removals {
        ops.push(json!({
            "op": "remove",
            "path": format!("/tracks/{track_idx}/clips/{clip_idx}"),
        }));
    }

    let mut warnings = vec![envelope_warning(&data)];
    warnings.extend(locked_survivor_warnings(&sorted_clears));

    Ok((Value::Array(ops), warnings, data))
}

fn parse_and_dedupe(args: &ClipDeleteArgs) -> Result<Vec<RequestedClip>, ClipDeleteError> {
    let mut seen = HashSet::new();
    let mut requested = Vec::new();

    for (input_index, raw) in args.clips.iter().enumerate() {
        let clip_id = raw
            .parse::<ClipId>()
            .map_err(|err| ClipDeleteError::BadSelector {
                failed_index: input_index,
                failed_target: raw.clone(),
                detail: err.to_string(),
            })?;

        if seen.insert(clip_id) {
            requested.push(RequestedClip {
                input_index,
                clip_id,
                raw: raw.clone(),
            });
        }
    }

    Ok(requested)
}

fn clip_locations(prior: &Project) -> HashMap<ClipId, ClipLocation> {
    let mut by_id = HashMap::new();
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            by_id.insert(
                clip.id,
                ClipLocation {
                    track_idx,
                    clip_idx,
                    track_locked: track.locked,
                    track_id: track.id.to_string(),
                    clip_locked: clip.locked,
                    clip_id: clip.id,
                },
            );
        }
    }
    by_id
}

fn lone_survivor_link_group_clears(
    prior: &Project,
    removal_ids: &HashSet<ClipId>,
) -> Vec<LinkGroupMember> {
    let mut members_by_group: HashMap<LinkGroupId, Vec<LinkGroupMember>> = HashMap::new();
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            let Some(link_group) = clip.link_group else {
                continue;
            };
            members_by_group
                .entry(link_group)
                .or_default()
                .push(LinkGroupMember {
                    track_idx,
                    clip_idx,
                    clip_id: clip.id,
                    locked: clip.locked,
                    in_removal_set: removal_ids.contains(&clip.id),
                });
        }
    }

    let mut clears = Vec::new();
    for members in members_by_group.into_values() {
        let had_removal = members.iter().any(|member| member.in_removal_set);
        if !had_removal {
            continue;
        }
        let survivors: Vec<LinkGroupMember> = members
            .into_iter()
            .filter(|member| !member.in_removal_set)
            .collect();
        if survivors.len() == 1 {
            clears.extend(survivors);
        }
    }
    clears
}

fn empty_data() -> ClipDeleteData {
    ClipDeleteData {
        removed_clip_ids: Vec::new(),
        missing_clip_ids: Vec::new(),
        cleared_link_group_clip_ids: Vec::new(),
    }
}

fn envelope_warning(data: &ClipDeleteData) -> Value {
    json!({
        "code": W_CLIP_DELETE_ENVELOPE_CODE,
        "message": "clip.delete envelope",
        "details": {
            "removed_clip_ids": stringify_ids(&data.removed_clip_ids),
            "missing_clip_ids": stringify_ids(&data.missing_clip_ids),
            "cleared_link_group_clip_ids": stringify_ids(&data.cleared_link_group_clip_ids),
        }
    })
}

fn locked_survivor_warnings(sorted_clears: &[LinkGroupMember]) -> Vec<Value> {
    sorted_clears
        .iter()
        .filter(|member| member.locked)
        .map(|member| {
            json!({
                "code": W_LINK_GROUP_CLEARED_ON_LOCKED_CODE,
                "message": "link group cleared on locked survivor",
                "details": {
                    "clip_id": member.clip_id.to_string(),
                }
            })
        })
        .collect()
}

fn stringify_ids<T: ToString>(ids: &[T]) -> Vec<String> {
    ids.iter().map(ToString::to_string).collect()
}

fn sort_ids<T>(ids: impl IntoIterator<Item = T>) -> Vec<T>
where
    T: ToString,
{
    let mut ids: Vec<T> = ids.into_iter().collect();
    ids.sort_by_key(ToString::to_string);
    ids
}

/// Rebuild `ClipDeleteData` from recorded warnings.
///
/// # Errors
/// Returns [`ReconstructError`] if the internal envelope warning is
/// missing or malformed.
pub fn data_envelope_from_args_warnings(
    _args: &ClipDeleteArgs,
    warnings: &[Value],
) -> Result<ClipDeleteData, ReconstructError> {
    let details = envelope_details_from_warnings(warnings)?;
    Ok(ClipDeleteData {
        removed_clip_ids: required_id_list(details, "removed_clip_ids")?,
        missing_clip_ids: required_id_list(details, "missing_clip_ids")?,
        cleared_link_group_clip_ids: required_id_list(details, "cleared_link_group_clip_ids")?,
    })
}

fn envelope_details_from_warnings(warnings: &[Value]) -> Result<&Value, ReconstructError> {
    for warning in warnings {
        if warning.get("code").and_then(Value::as_str) != Some(W_CLIP_DELETE_ENVELOPE_CODE) {
            continue;
        }
        return warning
            .get("details")
            .ok_or(ReconstructError::MissingField {
                name: "warnings[].W_CLIP_DELETE_ENVELOPE.details",
            });
    }

    Err(ReconstructError::MissingField {
        name: "warnings[].W_CLIP_DELETE_ENVELOPE",
    })
}

fn required_id_list<T>(details: &Value, name: &'static str) -> Result<Vec<T>, ReconstructError>
where
    T: std::str::FromStr + ToString,
{
    let values = details
        .get(name)
        .ok_or(ReconstructError::MissingField { name })?
        .as_array()
        .ok_or(ReconstructError::TypeMismatch {
            name,
            expected: "array of UUIDv7 strings",
        })?;
    let ids = values
        .iter()
        .map(|value| {
            let raw = value.as_str().ok_or(ReconstructError::TypeMismatch {
                name,
                expected: "array of UUIDv7 strings",
            })?;
            raw.parse::<T>()
                .map_err(|_| ReconstructError::TypeMismatch {
                    name,
                    expected: "array of UUIDv7 strings",
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(sort_ids(ids))
}

impl From<ClipDeleteError> for VerbError {
    fn from(value: ClipDeleteError) -> Self {
        match value {
            ClipDeleteError::SchemaViolation { .. }
            | ClipDeleteError::BadSelector { .. }
            | ClipDeleteError::NotFound { .. }
            | ClipDeleteError::Locked { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `clip.delete`.
#[derive(Debug, Default)]
pub struct ClipDeleteVerb;

impl Verb for ClipDeleteVerb {
    fn verb(&self) -> &'static str {
        "clip.delete"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ClipDeleteArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("clip.delete: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("clip.delete: patch construction failed: {err}"))
            })?;
        let data = serde_json::to_value(&data)
            .map_err(|err| VerbError::Custom(format!("clip.delete: data failed: {err}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: ClipDeleteArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ClipDeleteArgs",
            })?;

        let envelope = data_envelope_from_args_warnings(&typed, warnings)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
