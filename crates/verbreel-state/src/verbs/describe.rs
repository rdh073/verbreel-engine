//! `describe` (§1.3) — sixty-second production verb in the engine.
//!
//! ## Spec quote (`spec/commands/meta.md` §1.3, condensed)
//!
//! > Returns the full state of any entity by ID or selector. Read-only.
//! >
//! > **Args**: `project_id?: string`, `target: string`
//! > **Returns** (`data`): `{ kind: "project"|"track"|"clip"|"effect"|"keyframe"|"asset"|"marker";
//! > entity: object }`
//! > **Errors**: `E_NOT_FOUND`, `E_NO_MATCH`, `E_BAD_SELECTOR`, `E_ARGS_INCOMPATIBLE`.
//!
//! ## Read-only verb
//!
//! `describe` does not mutate project state; the patch is always `[]`,
//! no warnings are returned, and `data` carries `{kind, entity}` for
//! the resolved entity.
//!
//! ## Selector shape (this slice)
//!
//! This slice requires a qualified `<prefix>:<UUIDv7>` selector. The
//! seven supported prefixes are `project`, `track`, `clip`, `effect`,
//! `keyframe`, `asset`, `marker` — one branch per entity kind in the
//! `Project` graph.
//!
//! Bare-UUID resolution (a top-level UUID matching an open project, no
//! prefix) requires open-projects-index access at the lifecycle layer
//! and is deferred. Structural selectors (e.g. `clip:track-1:0`) are
//! likewise deferred; the [`DescribeError::NoMatch`] variant is kept on
//! the error enum for the future structural form and is currently
//! unreachable. CLI prefix-expansion `E_AMBIGUOUS_ID` is surfaced by
//! the CLI before the engine sees the call — out of verb-layer scope.
//!
//! ## Project-id rules
//!
//! `project_id` is REQUIRED at the verb layer. The lifecycle / CLI
//! layer infers it from `~/.verbreel/active-project` (§0.12) for the
//! CLI surface before invoking the verb. When `target` is
//! `project:<UUID>` and the UUID disagrees with `args.project_id`, the
//! verb returns [`DescribeError::ArgsIncompatible`] carrying both ids
//! per spec §1.3's decision table.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{AssetId, ClipId, EffectId, KeyframeId, MarkerId, ProjectId, TrackId};

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// The seven entity kinds resolvable by `describe`. Serialized as
/// lowercase strings (`"project"`, `"track"`, etc.) per spec §1.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DescribeKind {
    /// The `Project` root.
    Project,
    /// A `Track` on `Project.tracks[]`.
    Track,
    /// A `Clip` on `Track.clips[]`.
    Clip,
    /// An `Effect` — either clip-attached (`Clip.effects[]`) or
    /// track-level (`Track.effects[]`); both typed.
    Effect,
    /// A `Keyframe` on `Clip.keyframes[]`.
    Keyframe,
    /// An `Asset` on `Project.assets[]`.
    Asset,
    /// A `Marker` on `Project.markers[]`.
    Marker,
}

/// Arguments for `describe`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescribeArgs {
    /// Target project id. Required at the verb layer; the lifecycle /
    /// CLI layer is responsible for inferring it from active-project
    /// state before invoking the verb.
    pub project_id: ProjectId,

    /// Qualified selector `<prefix>:<UUIDv7>` where `<prefix>` is one of
    /// `project`, `track`, `clip`, `effect`, `keyframe`, `asset`,
    /// `marker`.
    pub target: String,
}

/// Envelope `data` returned by `describe`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DescribeData {
    /// Matched entity kind.
    pub kind: DescribeKind,

    /// Matched entity serialized as a JSON object. The shape mirrors
    /// the entity's `serde::Serialize` projection — `Project` /
    /// `Track` / `Clip` / `Asset` / `Marker` / `Effect` / `Keyframe`.
    pub entity: Value,
}

/// Verb-level failures for `describe`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DescribeError {
    /// `args.target` is empty, missing its `:` separator, has an empty
    /// prefix or empty body, uses an unknown prefix, or has a body that
    /// fails to parse as `UUIDv7`. Maps to `E_BAD_SELECTOR`.
    #[error("describe: `target` selector parse failed: {detail}")]
    BadSelector {
        /// Parse failure detail.
        detail: String,
    },

    /// Selector resolved to a known prefix but the entity id is not
    /// present in the project. Maps to `E_NOT_FOUND`.
    #[error("describe: {kind} `{target_id}` not found")]
    NotFound {
        /// Matched entity kind (`"track"`, `"clip"`, etc.).
        kind: &'static str,
        /// Missing entity id string.
        target_id: String,
    },

    /// Reserved for future structural selectors (e.g. `clip:track-1:0`)
    /// that match zero rows. Currently unreachable — every supported
    /// selector in this slice is a bare `<prefix>:<uuid>` form that
    /// routes through [`DescribeError::NotFound`] instead.
    #[error("describe: selector `{selector}` matched no {kind}")]
    NoMatch {
        /// Matched entity kind.
        kind: &'static str,
        /// Original selector string.
        selector: String,
    },

    /// `args.project_id` disagrees with the project id encoded in
    /// `project:<UUID>` (spec §1.3 decision table — `details` carries
    /// both ids). Maps to `E_ARGS_INCOMPATIBLE`.
    #[error(
        "describe: target project id `{target_project_id}` does not match supplied \
         project_id `{supplied_project_id}`"
    )]
    ArgsIncompatible {
        /// Project id encoded in the `project:` selector.
        target_project_id: String,
        /// Project id supplied via `args.project_id`.
        supplied_project_id: String,
    },
}

/// Parsed `(prefix, body)` form of a qualified selector.
#[derive(Debug, Clone)]
enum ParsedTarget {
    Project(ProjectId),
    Track(TrackId),
    Clip(ClipId),
    Effect(EffectId),
    Keyframe(KeyframeId),
    Asset(AssetId),
    Marker(MarkerId),
}

fn parse_target(raw: &str) -> Result<ParsedTarget, DescribeError> {
    if raw.is_empty() {
        return Err(DescribeError::BadSelector {
            detail: "selector is empty".to_string(),
        });
    }

    let (prefix, body) = raw
        .split_once(':')
        .ok_or_else(|| DescribeError::BadSelector {
            detail: format!(
                "selector `{raw}` is unqualified (missing `<prefix>:` — \
                 expected one of project/track/clip/effect/keyframe/asset/marker)"
            ),
        })?;

    if prefix.is_empty() {
        return Err(DescribeError::BadSelector {
            detail: "selector prefix is empty".to_string(),
        });
    }
    if body.is_empty() {
        return Err(DescribeError::BadSelector {
            detail: format!("selector body is empty after `{prefix}:`"),
        });
    }

    match prefix {
        "project" => body
            .parse::<ProjectId>()
            .map(ParsedTarget::Project)
            .map_err(|err| DescribeError::BadSelector {
                detail: format!("project body parse failed: {err}"),
            }),
        "track" => body
            .parse::<TrackId>()
            .map(ParsedTarget::Track)
            .map_err(|err| DescribeError::BadSelector {
                detail: format!("track body parse failed: {err}"),
            }),
        "clip" => body
            .parse::<ClipId>()
            .map(ParsedTarget::Clip)
            .map_err(|err| DescribeError::BadSelector {
                detail: format!("clip body parse failed: {err}"),
            }),
        "effect" => body
            .parse::<EffectId>()
            .map(ParsedTarget::Effect)
            .map_err(|err| DescribeError::BadSelector {
                detail: format!("effect body parse failed: {err}"),
            }),
        "keyframe" => body
            .parse::<KeyframeId>()
            .map(ParsedTarget::Keyframe)
            .map_err(|err| DescribeError::BadSelector {
                detail: format!("keyframe body parse failed: {err}"),
            }),
        "asset" => body
            .parse::<AssetId>()
            .map(ParsedTarget::Asset)
            .map_err(|err| DescribeError::BadSelector {
                detail: format!("asset body parse failed: {err}"),
            }),
        "marker" => body
            .parse::<MarkerId>()
            .map(ParsedTarget::Marker)
            .map_err(|err| DescribeError::BadSelector {
                detail: format!("marker body parse failed: {err}"),
            }),
        other => Err(DescribeError::BadSelector {
            detail: format!(
                "unknown selector prefix `{other}` (expected one of \
                 project/track/clip/effect/keyframe/asset/marker)"
            ),
        }),
    }
}

#[allow(clippy::too_many_lines)]
fn lookup(
    prior: &Project,
    parsed: &ParsedTarget,
    supplied_project_id: ProjectId,
) -> Result<DescribeData, DescribeError> {
    match parsed {
        ParsedTarget::Project(target_id) => {
            if *target_id != prior.id {
                return Err(DescribeError::ArgsIncompatible {
                    target_project_id: target_id.to_string(),
                    supplied_project_id: supplied_project_id.to_string(),
                });
            }
            let entity = serde_json::to_value(prior).map_err(|err| DescribeError::BadSelector {
                detail: format!("project serialize failed: {err}"),
            })?;
            Ok(DescribeData {
                kind: DescribeKind::Project,
                entity,
            })
        }
        ParsedTarget::Track(track_id) => {
            let track = prior
                .tracks
                .iter()
                .find(|t| t.id == *track_id)
                .ok_or_else(|| DescribeError::NotFound {
                    kind: "track",
                    target_id: track_id.to_string(),
                })?;
            let entity = serde_json::to_value(track).map_err(|err| DescribeError::BadSelector {
                detail: format!("track serialize failed: {err}"),
            })?;
            Ok(DescribeData {
                kind: DescribeKind::Track,
                entity,
            })
        }
        ParsedTarget::Clip(clip_id) => {
            let clip = prior
                .tracks
                .iter()
                .flat_map(|t| t.clips.iter())
                .find(|c| c.id == *clip_id)
                .ok_or_else(|| DescribeError::NotFound {
                    kind: "clip",
                    target_id: clip_id.to_string(),
                })?;
            let entity = serde_json::to_value(clip).map_err(|err| DescribeError::BadSelector {
                detail: format!("clip serialize failed: {err}"),
            })?;
            Ok(DescribeData {
                kind: DescribeKind::Clip,
                entity,
            })
        }
        ParsedTarget::Effect(effect_id) => {
            // Clip-attached and track-attached effects both live in
            // `Vec<Effect>` (typed, matched on `Effect.id`). First match
            // wins; clip scan first.
            for track in &prior.tracks {
                for clip in &track.clips {
                    for effect in &clip.effects {
                        if effect.id == *effect_id {
                            let entity = serde_json::to_value(effect).map_err(|err| {
                                DescribeError::BadSelector {
                                    detail: format!("effect serialize failed: {err}"),
                                }
                            })?;
                            return Ok(DescribeData {
                                kind: DescribeKind::Effect,
                                entity,
                            });
                        }
                    }
                }
            }
            for track in &prior.tracks {
                for effect in &track.effects {
                    if effect.id == *effect_id {
                        let entity = serde_json::to_value(effect).map_err(|err| {
                            DescribeError::BadSelector {
                                detail: format!("effect serialize failed: {err}"),
                            }
                        })?;
                        return Ok(DescribeData {
                            kind: DescribeKind::Effect,
                            entity,
                        });
                    }
                }
            }
            Err(DescribeError::NotFound {
                kind: "effect",
                target_id: effect_id.to_string(),
            })
        }
        ParsedTarget::Keyframe(keyframe_id) => {
            let kf = prior
                .tracks
                .iter()
                .flat_map(|t| t.clips.iter())
                .flat_map(|c| c.keyframes.iter())
                .find(|k| k.id == *keyframe_id)
                .ok_or_else(|| DescribeError::NotFound {
                    kind: "keyframe",
                    target_id: keyframe_id.to_string(),
                })?;
            let entity = serde_json::to_value(kf).map_err(|err| DescribeError::BadSelector {
                detail: format!("keyframe serialize failed: {err}"),
            })?;
            Ok(DescribeData {
                kind: DescribeKind::Keyframe,
                entity,
            })
        }
        ParsedTarget::Asset(asset_id) => {
            let asset = prior
                .assets
                .iter()
                .find(|a| a.id() == asset_id)
                .ok_or_else(|| DescribeError::NotFound {
                    kind: "asset",
                    target_id: asset_id.to_string(),
                })?;
            let entity = serde_json::to_value(asset).map_err(|err| DescribeError::BadSelector {
                detail: format!("asset serialize failed: {err}"),
            })?;
            Ok(DescribeData {
                kind: DescribeKind::Asset,
                entity,
            })
        }
        ParsedTarget::Marker(marker_id) => {
            let marker = prior
                .markers
                .iter()
                .find(|m| m.id == *marker_id)
                .ok_or_else(|| DescribeError::NotFound {
                    kind: "marker",
                    target_id: marker_id.to_string(),
                })?;
            let entity =
                serde_json::to_value(marker).map_err(|err| DescribeError::BadSelector {
                    detail: format!("marker serialize failed: {err}"),
                })?;
            Ok(DescribeData {
                kind: DescribeKind::Marker,
                entity,
            })
        }
    }
}

/// Build the (empty) RFC-6902 patch and data envelope for `describe`.
///
/// The patch is always `[]` and the warnings vec is always empty — this
/// is a read-only verb.
///
/// # Errors
///
/// Returns [`DescribeError`] for selector parse failures, unknown
/// target ids, or project-id disagreement with a `project:` selector.
pub fn compute_patch(
    prior: &Project,
    args: &DescribeArgs,
) -> Result<(Value, Vec<Value>, DescribeData), DescribeError> {
    let parsed = parse_target(&args.target)?;
    let data = lookup(prior, &parsed, args.project_id)?;
    Ok((json!([]), Vec::new(), data))
}

/// Rebuild the data envelope from `(args, post_state)`.
///
/// For a read-only verb the post-state equals the pre-state, so the
/// same lookup drops out of the post-state graph.
///
/// # Errors
///
/// Returns [`ReconstructError`] when args do not deserialize, the
/// selector is malformed, or the post-state walk fails to find the
/// target.
pub fn data_envelope_from_post_state(
    args: &DescribeArgs,
    post_state: &Project,
) -> Result<DescribeData, ReconstructError> {
    let parsed = parse_target(&args.target).map_err(|_| ReconstructError::TypeMismatch {
        name: "args.target",
        expected: "qualified `<prefix>:<UUIDv7>` selector",
    })?;
    lookup(post_state, &parsed, args.project_id).map_err(|err| match err {
        DescribeError::NotFound { kind, target_id } => ReconstructError::PostStateMissing {
            detail: format!("describe: {kind} {target_id} not found in post_state"),
        },
        other => ReconstructError::Custom(other.to_string()),
    })
}

impl From<DescribeError> for VerbError {
    fn from(value: DescribeError) -> Self {
        match value {
            DescribeError::BadSelector { .. }
            | DescribeError::NotFound { .. }
            | DescribeError::NoMatch { .. }
            | DescribeError::ArgsIncompatible { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `describe`.
#[derive(Debug, Default)]
pub struct DescribeVerb;

impl Verb for DescribeVerb {
    fn verb(&self) -> &'static str {
        "describe"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: DescribeArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("describe: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!("describe: patch construction failed: {err}"))
        })?;
        let data = serde_json::to_value(&data)
            .map_err(|err| VerbError::Custom(format!("describe: data envelope failed: {err}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: DescribeArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "DescribeArgs",
            })?;
        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
