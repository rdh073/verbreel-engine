//! `track.add` (§4.1) — ninth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/track.md` §4.1, verbatim)
//!
//! > Appends a new track of the given kind.
//!
//! **CLI**: `verbreel track add [--project <id>] --kind <kind> [--name <str>] [--index <i>]`
//! **MCP**: `track.add`
//! **Args**: `project_id: string`, `kind: enum`, `name?: string`, `index?: integer`
//! **Returns** (`data`): `{ track_id: string; kind: string; index: integer }`
//! **Errors**: `E_TRACK_BAD_INDEX`, `E_TRACK_NAME_CONFLICT`
//!
//! ## §4.1 auto-namer (byte-exact)
//!
//! Auto-name is the title-case kind plus the next free integer suffix found
//! in existing names that match the byte-exact regex
//!
//! `^<Kind> (0|[1-9][0-9]*)$`
//!
//! where `<Kind>` is `Video` / `Audio` / `Text` / `Effect`. No leading
//! zeros (`02` / `01`), no alternate numerals (`Video ١`), no extra words
//! (`Video Final 2`) participate in the scan. The next name is always
//! `max_observed + 1`; gaps are not filled.
//!
//! ## Kind-relative index
//!
//! `index` is relative to the contiguous kind block in `Project.tracks[]`:
//!
//! - `index: 0` inserts at the head of that block
//! - `index: count` inserts at the block tail
//!
//! A new kind with no existing block maps to insertion at the end of
//! `tracks[]`.
//!
//! ## Contiguity preservation
//!
//! `Project::apply()` enforces §0.13 track-contiguity, so invalid
//! kind-relative mapping (inserting into another kind's region) must be
//! avoided during patch construction.
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::OnceLock;
use thiserror::Error;
use verbreel_types::{ProjectId, TrackId};

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::{Track, TrackKind};

/// Maximum `Track.name` length per schema (`maxLength: 128`).
pub const TRACK_NAME_MAX: usize = 128;

/// Maximum `Track.name` length for `track.add` explicit names.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackAddArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Track kind to insert.
    pub kind: TrackKind,

    /// Optional track name. Omitted names are auto-generated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Optional kind-relative insertion index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
}

/// Envelope data emitted by `track.add`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackAddData {
    /// Newly inserted track id.
    pub track_id: TrackId,

    /// Inserted track kind.
    pub kind: TrackKind,

    /// Final kind-relative index of the inserted track in the post-state.
    pub index: usize,
}

/// Validation failures for `track.add`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TrackAddError {
    /// Empty explicit `name`.
    #[error("track.add: `name` must be non-empty")]
    NameEmpty,

    /// Explicit `name` longer than [`TRACK_NAME_MAX`].
    #[error("track.add: name has {actual} chars, exceeds maximum {max}")]
    NameTooLong {
        /// Measured chars in the incoming name.
        actual: usize,

        /// Maximum allowed chars.
        max: usize,
    },

    /// Duplicate name within the target kind.
    #[error("track.add: track name `{name}` already exists for kind {kind:?}")]
    NameConflict {
        /// Name that collides in-kind.
        name: String,

        /// Track kind in which collision occurred.
        kind: TrackKind,
    },

    /// Kind-relative insertion index out of bounds.
    #[error("track.add: index {requested} exceeds max allowed {max_allowed} for kind {kind:?}")]
    BadIndex {
        /// Requested kind-relative index.
        requested: usize,

        /// Maximum allowed index (`kind_count`).
        max_allowed: usize,

        /// Target kind for the failing index check.
        kind: TrackKind,
    },
}

fn kind_label(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Video => "Video",
        TrackKind::Audio => "Audio",
        TrackKind::Text => "Text",
        TrackKind::Effect => "Effect",
    }
}

fn auto_name_regex_for_kind(kind: TrackKind) -> &'static Regex {
    match kind {
        TrackKind::Video => {
            static VIDEO_NAME_RE: OnceLock<Regex> = OnceLock::new();
            VIDEO_NAME_RE
                .get_or_init(|| Regex::new(r"^Video (0|[1-9][0-9]*)$").expect("valid regex"))
        }
        TrackKind::Audio => {
            static AUDIO_NAME_RE: OnceLock<Regex> = OnceLock::new();
            AUDIO_NAME_RE
                .get_or_init(|| Regex::new(r"^Audio (0|[1-9][0-9]*)$").expect("valid regex"))
        }
        TrackKind::Text => {
            static TEXT_NAME_RE: OnceLock<Regex> = OnceLock::new();
            TEXT_NAME_RE.get_or_init(|| Regex::new(r"^Text (0|[1-9][0-9]*)$").expect("valid regex"))
        }
        TrackKind::Effect => {
            static EFFECT_NAME_RE: OnceLock<Regex> = OnceLock::new();
            EFFECT_NAME_RE
                .get_or_init(|| Regex::new(r"^Effect (0|[1-9][0-9]*)$").expect("valid regex"))
        }
    }
}

fn auto_name_for_kind(prior: &Project, kind: TrackKind) -> String {
    let mut max_seen = 0usize;
    let re = auto_name_regex_for_kind(kind);

    for track in &prior.tracks {
        if track.kind != kind {
            continue;
        }

        let Some(captures) = re.captures(&track.name) else {
            continue;
        };
        let parsed = captures
            .get(1)
            .and_then(|group| group.as_str().parse::<usize>().ok())
            .unwrap_or(0);
        if parsed > max_seen {
            max_seen = parsed;
        }
    }

    format!("{} {}", kind_label(kind), max_seen + 1)
}

fn find_kind_block_range(prior: &Project, kind: TrackKind) -> Option<(usize, usize)> {
    let start = prior.tracks.iter().position(|track| track.kind == kind)?;
    let mut count = 0usize;

    for track in &prior.tracks[start..] {
        if track.kind != kind {
            break;
        }
        count += 1;
    }

    Some((start, count))
}

fn resolve_global_insertion_idx(
    prior: &Project,
    kind: TrackKind,
    kind_relative_idx: usize,
) -> usize {
    match find_kind_block_range(prior, kind) {
        Some((start, _count)) => start + kind_relative_idx,
        None => prior.tracks.len(),
    }
}

/// Build the RFC 6902 patch and warnings for `track.add`.
///
/// # Errors
/// - [`TrackAddError::NameEmpty`] for empty explicit `name`.
/// - [`TrackAddError::NameTooLong`] for explicit `name` beyond
///   [`TRACK_NAME_MAX`].
/// - [`TrackAddError::NameConflict`] when an in-kind track already uses
///   the same explicit name.
/// - [`TrackAddError::BadIndex`] when requested `index > kind_count`.
pub fn compute_patch(
    prior: &Project,
    args: &TrackAddArgs,
) -> Result<(Value, Vec<Value>), TrackAddError> {
    let resolved_name = match args.name.clone() {
        Some(name) => name,
        None => auto_name_for_kind(prior, args.kind),
    };

    if resolved_name.is_empty() {
        return Err(TrackAddError::NameEmpty);
    }

    let resolved_len = resolved_name.chars().count();
    if resolved_len > TRACK_NAME_MAX {
        return Err(TrackAddError::NameTooLong {
            actual: resolved_len,
            max: TRACK_NAME_MAX,
        });
    }

    if prior
        .tracks
        .iter()
        .any(|track| track.kind == args.kind && track.name == resolved_name)
    {
        return Err(TrackAddError::NameConflict {
            name: resolved_name.clone(),
            kind: args.kind,
        });
    }

    let kind_count = find_kind_block_range(prior, args.kind).map_or(0, |(_, count)| count);
    let requested_idx = args.index.unwrap_or(kind_count);
    if requested_idx > kind_count {
        return Err(TrackAddError::BadIndex {
            requested: requested_idx,
            max_allowed: kind_count,
            kind: args.kind,
        });
    }

    let global_idx = resolve_global_insertion_idx(prior, args.kind, requested_idx);
    let new_track_id = TrackId::now();

    let track = Track {
        id: new_track_id,
        kind: args.kind,
        name: resolved_name,
        clips: Vec::new(),
        muted: false,
        solo: false,
        locked: false,
        hidden: false,
        volume: 1.0,
        pan: 0.0,
        effects: Vec::new(),
    };

    let patch = json!([{
        "op": "add",
        "path": format!("/tracks/{global_idx}"),
        "value": track,
    }]);

    Ok((patch, Vec::new()))
}

/// Build the envelope `data` value from the patch and post-state.
///
/// # Errors
/// - If the patch is not a single `add` op on `"/tracks/<index>"`.
/// - If `id` is missing from the patch value or cannot parse as `TrackId`.
/// - If the inserted track cannot be found in `post_state`.
/// - If the track's kind block cannot be located in `post_state`.
pub fn data_envelope_from_post_state(
    patch: &Value,
    post_state: &Project,
) -> Result<TrackAddData, ReconstructError> {
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

    let op_name = op
        .get("op")
        .and_then(Value::as_str)
        .ok_or(ReconstructError::MissingField {
            name: "patch[0].op",
        })?;
    if op_name != "add" {
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
    if !op_path.starts_with("/tracks/") {
        return Err(ReconstructError::TypeMismatch {
            name: "patch[0].path",
            expected: "/tracks/<idx>",
        });
    }

    let value =
        op.get("value")
            .and_then(Value::as_object)
            .ok_or(ReconstructError::MissingField {
                name: "patch[0].value",
            })?;

    let track_id_str =
        value
            .get("id")
            .and_then(Value::as_str)
            .ok_or(ReconstructError::MissingField {
                name: "patch[0].value.id",
            })?;
    let track_id = track_id_str
        .parse::<TrackId>()
        .map_err(|err| ReconstructError::Custom(err.to_string()))?;

    let (global_idx, found_kind) = post_state
        .tracks
        .iter()
        .enumerate()
        .find(|(_, track)| track.id == track_id)
        .map(|(idx, track)| (idx, track.kind))
        .ok_or(ReconstructError::Custom(format!(
            "track_add: track id {track_id} not found in post_state.tracks"
        )))?;

    let (kind_start, _) =
        find_kind_block_range(post_state, found_kind).ok_or(ReconstructError::Custom(format!(
            "track_add: kind block for {found_kind:?} missing in post_state"
        )))?;

    Ok(TrackAddData {
        track_id,
        kind: found_kind,
        index: global_idx - kind_start,
    })
}

impl From<TrackAddError> for VerbError {
    fn from(value: TrackAddError) -> Self {
        match value {
            TrackAddError::NameEmpty
            | TrackAddError::NameTooLong { .. }
            | TrackAddError::NameConflict { .. }
            | TrackAddError::BadIndex { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// `track.add` verb registration entry.
#[derive(Debug, Default)]
pub struct TrackAddVerb;

impl Verb for TrackAddVerb {
    fn verb(&self) -> &'static str {
        "track.add"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: TrackAddArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("track.add: args deserialize failed: {err}"),
            })?;

        let (patch_value, _warnings) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch =
            serde_json::from_value(patch_value.clone()).map_err(|err| {
                VerbError::Custom(format!("track.add: patch construction failed: {err}"))
            })?;

        let post_state = prior
            .apply(&patch)
            .map_err(|err| VerbError::InvariantViolation {
                detail: format!("track.add: post-state validation failed: {err}"),
            })?;

        let envelope = data_envelope_from_post_state(&patch_value, &post_state).map_err(|err| {
            VerbError::Custom(format!(
                "track.add: data envelope reconstruction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&envelope)
            .map_err(|err| VerbError::Custom(format!("track.add: data serialize failed: {err}")))?;

        Ok((patch, data, Vec::new()))
    }

    fn reconstruct(
        &self,
        _args: &Value,
        patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let envelope = data_envelope_from_post_state(patch, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
