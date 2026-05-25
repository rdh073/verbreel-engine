//! `keyframe.remove` (§8.2) — thirty-fifth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/keyframe.md` §8.2, summarized)
//!
//! `keyframe.remove` removes a batch of keyframes by bare `UUIDv7` id.
//! `soft=false` rejects the whole batch when any id is missing;
//! `soft=true` removes existing ids and reports missing ids in the
//! data envelope.
//!
//! Malformed keyframe ids intentionally return `E_BAD_SELECTOR`-style
//! [`KeyframeRemoveError::BadSelector`] errors with `failed_index`.
//! §8.2 does not list `E_BAD_SELECTOR`, but §A treats malformed
//! selectors uniformly and `keyframe.set` already rejects non-bare-UUID
//! keyframe selectors on that path.
//!
//! RFC-6902 remove operations are emitted in descending
//! `(track, clip, keyframe)` index order so deleting one keyframe cannot
//! shift the later paths in the same atomic patch.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{KeyframeId, ProjectId};

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Maximum allowed batch length for `keyframes` (§0.8).
pub const KEYFRAMES_MAX_BATCH: usize = 10_000;

/// Warning code emitted when `keyframe.remove` produces an empty patch
/// or skips a missing keyframe in soft mode.
pub const W_NOOP_CODE: &str = "W_NOOP";

const KEYFRAMES_FIELD: &str = "keyframes";
const SPLIT_BATCH_HINT: &str = "split the batch into smaller calls";

/// Arguments for `keyframe.remove`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyframeRemoveArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Keyframe ids to remove, as bare `UUIDv7` strings.
    pub keyframes: Vec<String>,

    /// `true` downgrades missing ids from error to the data envelope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft: Option<bool>,
}

impl KeyframeRemoveArgs {
    fn soft(&self) -> bool {
        self.soft.unwrap_or(false)
    }
}

/// Envelope returned by `keyframe.remove`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyframeRemoveData {
    /// IDs successfully removed, sorted by UUID string.
    pub removed_keyframe_ids: Vec<KeyframeId>,

    /// Missing IDs skipped under `soft=true`, sorted by UUID string.
    pub missing_keyframe_ids: Vec<KeyframeId>,
}

/// Verb-level validation failures for `keyframe.remove`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum KeyframeRemoveError {
    /// `args.keyframes.len() > KEYFRAMES_MAX_BATCH`.
    #[error("keyframe.remove: too many keyframes ({actual}) > max ({max})")]
    SchemaViolation {
        /// Field that violated the schema-level cap.
        field: &'static str,
        /// Actionable split hint.
        hint: &'static str,
        /// Actual number of keyframe ids supplied.
        actual: usize,
        /// Hard maximum.
        max: usize,
    },

    /// Keyframe id failed bare-UUID parsing.
    #[error(
        "keyframe.remove: keyframe `{failed_target}` selector failed at index {failed_index}: {detail}"
    )]
    BadSelector {
        /// Input index where parsing failed.
        failed_index: usize,
        /// Raw offending string.
        failed_target: String,
        /// Parse failure detail.
        detail: String,
    },

    /// Missing keyframe in strict mode.
    #[error("keyframe.remove: keyframe `{failed_target}` not found at index {failed_index}")]
    NotFound {
        /// Input index where the missing keyframe was first requested.
        failed_index: usize,
        /// Missing keyframe id string.
        failed_target: String,
    },

    /// Parent clip or parent track is locked.
    #[error(
        "keyframe.remove: {kind} `{id}` is locked for keyframe `{failed_target}` at index {failed_index}"
    )]
    Locked {
        /// Input index where the locked keyframe was first requested.
        failed_index: usize,
        /// Requested keyframe id string.
        failed_target: String,
        /// Locked entity kind.
        kind: &'static str,
        /// Locked entity id.
        id: String,
    },
}

#[derive(Debug, Clone)]
struct RequestedKeyframe {
    input_index: usize,
    keyframe_id: KeyframeId,
    raw: String,
}

#[derive(Debug, Clone)]
struct LocatedKeyframe {
    request: RequestedKeyframe,
    track_idx: usize,
    clip_idx: usize,
    keyframe_idx: usize,
    track_locked: bool,
    track_id: String,
    clip_locked: bool,
    clip_id: String,
}

/// Build the RFC-6902 patch for `keyframe.remove`.
///
/// # Errors
/// Returns [`KeyframeRemoveError`] for batch-size, selector, missing
/// keyframe, or locked-parent failures.
pub fn compute_patch(
    prior: &Project,
    args: &KeyframeRemoveArgs,
) -> Result<(Value, Vec<Value>, KeyframeRemoveData), KeyframeRemoveError> {
    if args.keyframes.len() > KEYFRAMES_MAX_BATCH {
        return Err(KeyframeRemoveError::SchemaViolation {
            field: KEYFRAMES_FIELD,
            hint: SPLIT_BATCH_HINT,
            actual: args.keyframes.len(),
            max: KEYFRAMES_MAX_BATCH,
        });
    }

    let requested = parse_and_dedupe(args)?;
    if requested.is_empty() {
        return Ok((json!([]), vec![no_op_warning()], empty_data()));
    }

    let prior_by_id = keyframe_locations(prior);
    let mut found = Vec::new();
    let mut missing_keyframe_ids = Vec::new();
    let mut warnings = Vec::new();

    for request in requested {
        if let Some(location) = prior_by_id.get(&request.keyframe_id) {
            found.push(LocatedKeyframe {
                request,
                track_idx: location.track_idx,
                clip_idx: location.clip_idx,
                keyframe_idx: location.keyframe_idx,
                track_locked: location.track_locked,
                track_id: location.track_id.clone(),
                clip_locked: location.clip_locked,
                clip_id: location.clip_id.clone(),
            });
            continue;
        }

        if args.soft() {
            missing_keyframe_ids.push(request.keyframe_id);
            warnings.push(missing_warning(&request));
            continue;
        }

        return Err(KeyframeRemoveError::NotFound {
            failed_index: request.input_index,
            failed_target: request.raw,
        });
    }

    for located in &found {
        if located.track_locked {
            return Err(KeyframeRemoveError::Locked {
                failed_index: located.request.input_index,
                failed_target: located.request.raw.clone(),
                kind: "track",
                id: located.track_id.clone(),
            });
        }
        if located.clip_locked {
            return Err(KeyframeRemoveError::Locked {
                failed_index: located.request.input_index,
                failed_target: located.request.raw.clone(),
                kind: "clip",
                id: located.clip_id.clone(),
            });
        }
    }

    let mut removals: Vec<(usize, usize, usize)> = found
        .iter()
        .map(|located| (located.track_idx, located.clip_idx, located.keyframe_idx))
        .collect();
    removals.sort_by(|left, right| right.cmp(left));

    let patch = Value::Array(
        removals
            .iter()
            .map(|(track_idx, clip_idx, keyframe_idx)| {
                json!({
                    "op": "remove",
                    "path": format!(
                        "/tracks/{track_idx}/clips/{clip_idx}/keyframes/{keyframe_idx}"
                    ),
                })
            })
            .collect(),
    );

    if removals.is_empty() && warnings.is_empty() {
        warnings.push(no_op_warning());
    }

    let removed_keyframe_ids = sort_ids(found.iter().map(|located| located.request.keyframe_id));
    let missing_keyframe_ids = sort_ids(missing_keyframe_ids);

    Ok((
        patch,
        warnings,
        KeyframeRemoveData {
            removed_keyframe_ids,
            missing_keyframe_ids,
        },
    ))
}

fn parse_and_dedupe(
    args: &KeyframeRemoveArgs,
) -> Result<Vec<RequestedKeyframe>, KeyframeRemoveError> {
    let mut seen = HashSet::new();
    let mut requested = Vec::new();

    for (input_index, raw) in args.keyframes.iter().enumerate() {
        let keyframe_id =
            raw.parse::<KeyframeId>()
                .map_err(|err| KeyframeRemoveError::BadSelector {
                    failed_index: input_index,
                    failed_target: raw.clone(),
                    detail: err.to_string(),
                })?;

        if seen.insert(keyframe_id) {
            requested.push(RequestedKeyframe {
                input_index,
                keyframe_id,
                raw: raw.clone(),
            });
        }
    }

    Ok(requested)
}

#[derive(Debug, Clone)]
struct KeyframeLocation {
    track_idx: usize,
    clip_idx: usize,
    keyframe_idx: usize,
    track_locked: bool,
    track_id: String,
    clip_locked: bool,
    clip_id: String,
}

fn keyframe_locations(prior: &Project) -> HashMap<KeyframeId, KeyframeLocation> {
    let mut by_id = HashMap::new();
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            for (keyframe_idx, keyframe) in clip.keyframes.iter().enumerate() {
                by_id.insert(
                    keyframe.id,
                    KeyframeLocation {
                        track_idx,
                        clip_idx,
                        keyframe_idx,
                        track_locked: track.locked,
                        track_id: track.id.to_string(),
                        clip_locked: clip.locked,
                        clip_id: clip.id.to_string(),
                    },
                );
            }
        }
    }
    by_id
}

fn sort_ids(ids: impl IntoIterator<Item = KeyframeId>) -> Vec<KeyframeId> {
    let mut ids: Vec<KeyframeId> = ids.into_iter().collect();
    ids.sort_by_key(ToString::to_string);
    ids
}

fn empty_data() -> KeyframeRemoveData {
    KeyframeRemoveData {
        removed_keyframe_ids: Vec::new(),
        missing_keyframe_ids: Vec::new(),
    }
}

fn no_op_warning() -> Value {
    json!({
        "code": W_NOOP_CODE,
        "message": "keyframe.remove no-op",
        "details": {
            "verb": "keyframe.remove",
        }
    })
}

fn missing_warning(request: &RequestedKeyframe) -> Value {
    json!({
        "code": W_NOOP_CODE,
        "message": "keyframe not found (soft skip)",
        "details": {
            "keyframe_id": request.raw,
            "input_index": request.input_index,
        }
    })
}

/// Rebuild `KeyframeRemoveData` from recorded args and warnings.
///
/// # Errors
/// Returns [`ReconstructError`] if recorded args or warning details are
/// malformed.
pub fn data_envelope_from_args_warnings(
    args: &KeyframeRemoveArgs,
    warnings: &[Value],
) -> Result<KeyframeRemoveData, ReconstructError> {
    let requested = parse_and_dedupe_for_reconstruct(args)?;
    let missing_keyframe_ids = missing_ids_from_warnings(warnings)?;
    let missing_set: HashSet<KeyframeId> = missing_keyframe_ids.iter().copied().collect();
    let removed_keyframe_ids = requested
        .into_iter()
        .filter(|keyframe_id| !missing_set.contains(keyframe_id));

    Ok(KeyframeRemoveData {
        removed_keyframe_ids: sort_ids(removed_keyframe_ids),
        missing_keyframe_ids: sort_ids(missing_keyframe_ids),
    })
}

fn parse_and_dedupe_for_reconstruct(
    args: &KeyframeRemoveArgs,
) -> Result<Vec<KeyframeId>, ReconstructError> {
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    for raw in &args.keyframes {
        let id = raw
            .parse::<KeyframeId>()
            .map_err(|_| ReconstructError::TypeMismatch {
                name: "args.keyframes[]",
                expected: "UUIDv7 KeyframeId string",
            })?;
        if seen.insert(id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

fn missing_ids_from_warnings(warnings: &[Value]) -> Result<Vec<KeyframeId>, ReconstructError> {
    let mut ids = Vec::new();
    for warning in warnings {
        if warning.get("code").and_then(Value::as_str) != Some(W_NOOP_CODE) {
            continue;
        }
        let Some(details) = warning.get("details") else {
            continue;
        };
        let Some(keyframe_id) = details.get("keyframe_id") else {
            continue;
        };
        let raw = keyframe_id.as_str().ok_or(ReconstructError::TypeMismatch {
            name: "warnings[].details.keyframe_id",
            expected: "string",
        })?;
        ids.push(
            raw.parse::<KeyframeId>()
                .map_err(|_| ReconstructError::TypeMismatch {
                    name: "warnings[].details.keyframe_id",
                    expected: "UUIDv7 KeyframeId string",
                })?,
        );
    }
    Ok(ids)
}

impl From<KeyframeRemoveError> for VerbError {
    fn from(value: KeyframeRemoveError) -> Self {
        match value {
            KeyframeRemoveError::SchemaViolation { .. }
            | KeyframeRemoveError::BadSelector { .. }
            | KeyframeRemoveError::NotFound { .. }
            | KeyframeRemoveError::Locked { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `keyframe.remove`.
#[derive(Debug, Default)]
pub struct KeyframeRemoveVerb;

impl Verb for KeyframeRemoveVerb {
    fn verb(&self) -> &'static str {
        "keyframe.remove"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: KeyframeRemoveArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("keyframe.remove: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("keyframe.remove: patch construction failed: {err}"))
            })?;
        let data = serde_json::to_value(&data)
            .map_err(|err| VerbError::Custom(format!("keyframe.remove: data failed: {err}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        warnings: &[Value],
        _post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: KeyframeRemoveArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "KeyframeRemoveArgs",
            })?;

        let envelope = data_envelope_from_args_warnings(&typed, warnings)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
