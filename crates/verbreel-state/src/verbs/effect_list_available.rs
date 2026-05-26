//! `effect.list_available` (§6.5) — twenty-seventh production verb in the engine.
//!
//! ## Spec quote (`spec/commands/effect.md` §6.5, verbatim)
//!
//! > CLI: `verbreel effect list_available [--category <video|audio|transition|text>]`
//! > MCP: `effect.list_available`
//! > Args: `category?: "video"|"audio"|"transition"|"text"`
//! > Returns (`data`): `{ effects: { kind: string; category: string; summary: string; params_schema_id: string }[] }`
//! > Bundled effect kinds (v1.0): blur, lut, chroma_key, crop,
//! > color_correct, time_stretch, eq, compressor, reverb, denoise,
//! > burned_caption, transition.crossfade, transition.wipe,
//! > transition.dip_to_black.
//!
//! ## Bundle metadata, not project state.
//!
//! `effect.list_available` is read-only and does not read or mutate
//! project state; it only exposes curated effect metadata from this
//! engine build.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Effect category filter for `effect.list_available`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EffectCategory {
    /// Video effects.
    Video,
    /// Audio effects.
    Audio,
    /// Transition effects.
    Transition,
    /// Text effects.
    Text,
}

/// Arguments for `effect.list_available`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectListAvailableArgs {
    /// Target project id.
    pub project_id: ProjectId,

    /// Optional category filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<EffectCategory>,
}

/// Single effect metadata entry in the returned list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailableEffect {
    /// Effect kind identifier.
    pub kind: String,
    /// Effect category.
    pub category: String,
    /// Short effect summary.
    pub summary: String,
    /// Parameter schema identifier.
    pub params_schema_id: String,
}

/// Envelope returned by `effect.list_available`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectListAvailableData {
    /// Available (and optionally filtered) effect entries.
    pub effects: Vec<AvailableEffect>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// Verb-level error type for `effect.list_available`.
pub enum EffectListAvailableError {
    /// No verb-level runtime errors.
    #[error("effect.list_available: unreachable (no error variants)")]
    Unreachable,
}

/// Static bundled effects for v1.0.
///
/// ## Returns
///
/// Entries are sorted lexicographically by `kind`.
pub const V1_EFFECT_KINDS: &[&str] = &[
    "blur",
    "burned_caption",
    "chroma_key",
    "color_correct",
    "compressor",
    "crop",
    "denoise",
    "eq",
    "lut",
    "reverb",
    "time_stretch",
    "transition.crossfade",
    "transition.dip_to_black",
    "transition.wipe",
];

const BUNDLED_EFFECTS: &[(&str, &str, &str, &str)] = &[
    (
        "blur",
        "video",
        "Uniform Gaussian blur.",
        "EffectParams.blur@1",
    ),
    (
        "chroma_key",
        "video",
        "Single-color chroma key (v1.0).",
        "EffectParams.chroma_key@1",
    ),
    (
        "color_correct",
        "video",
        "Brightness / contrast / saturation / temperature / tint adjustment.",
        "EffectParams.color_correct@1",
    ),
    (
        "compressor",
        "audio",
        "Dynamic-range compression.",
        "EffectParams.compressor@1",
    ),
    ("crop", "video", "Rectangular crop.", "EffectParams.crop@1"),
    (
        "denoise",
        "audio",
        "Audio noise reduction (managed).",
        "EffectParams.denoise@1",
    ),
    ("eq", "audio", "Parametric EQ.", "EffectParams.eq@1"),
    (
        "lut",
        "video",
        "3D LUT color grading.",
        "EffectParams.lut@1",
    ),
    (
        "reverb",
        "audio",
        "Convolution / algorithmic reverb.",
        "EffectParams.reverb@1",
    ),
    (
        "time_stretch",
        "audio",
        "Pitch-preserving time stretch (managed, owned by clip.set_speed).",
        "EffectParams.time_stretch@1",
    ),
    (
        "burned_caption",
        "text",
        "Burned-in caption overlay (managed by caption.bake).",
        "EffectParams.burned_caption@1",
    ),
    (
        "transition.crossfade",
        "transition",
        "Crossfade between adjacent clips.",
        "EffectParams.transition.crossfade@1",
    ),
    (
        "transition.dip_to_black",
        "transition",
        "Dip-to-black transition.",
        "EffectParams.transition.dip_to_black@1",
    ),
    (
        "transition.wipe",
        "transition",
        "Wipe transition (directional).",
        "EffectParams.transition.wipe@1",
    ),
];

/// Return v1.0 bundled effects sorted alphabetically by kind.
///
/// # Errors
///
/// This helper currently cannot fail.
#[must_use]
pub fn bundled_effects() -> Vec<AvailableEffect> {
    let mut effects: Vec<AvailableEffect> = BUNDLED_EFFECTS
        .iter()
        .map(|(kind, category, summary, schema_id)| AvailableEffect {
            kind: (*kind).to_string(),
            category: (*category).to_string(),
            summary: (*summary).to_string(),
            params_schema_id: (*schema_id).to_string(),
        })
        .collect();
    effects.sort_by(|a, b| a.kind.cmp(&b.kind));
    effects
}

/// Build the RFC 6902 patch for `effect.list_available`.
///
/// # Errors
///
/// No runtime errors are produced by this verb; the returned `Result` exists
/// for parity with the broader compute-patch API.
pub fn compute_patch(
    _prior: &Project,
    args: &EffectListAvailableArgs,
) -> Result<(Value, Vec<Value>, EffectListAvailableData), EffectListAvailableError> {
    let category_filter: Option<&str> = args.category.map(|c| match c {
        EffectCategory::Video => "video",
        EffectCategory::Audio => "audio",
        EffectCategory::Transition => "transition",
        EffectCategory::Text => "text",
    });

    let effects: Vec<AvailableEffect> = bundled_effects()
        .into_iter()
        .filter(|e| match category_filter {
            Some(c) => e.category == c,
            None => true,
        })
        .collect();

    Ok((json!([]), Vec::new(), EffectListAvailableData { effects }))
}

/// Build the data envelope from `(args, post_state)`.
///
/// # Errors
///
/// Reuses [`compute_patch`], so this can only return reconstruction errors
/// introduced while rebuilding the deterministic envelope.
pub fn data_envelope_from_args(
    args: &EffectListAvailableArgs,
    post_state: &Project,
) -> Result<EffectListAvailableData, ReconstructError> {
    let (_, _, data) =
        compute_patch(post_state, args).map_err(|e| ReconstructError::Custom(e.to_string()))?;
    Ok(data)
}

impl From<EffectListAvailableError> for VerbError {
    fn from(value: EffectListAvailableError) -> Self {
        match value {
            EffectListAvailableError::Unreachable => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `effect.list_available`.
#[derive(Debug, Default)]
pub struct EffectListAvailableVerb;

impl Verb for EffectListAvailableVerb {
    fn verb(&self) -> &'static str {
        "effect.list_available"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: EffectListAvailableArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("effect.list_available: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|err| {
            VerbError::Custom(format!(
                "effect.list_available: patch construction failed: {err}"
            ))
        })?;

        let data = serde_json::to_value(&data).map_err(|err| {
            VerbError::Custom(format!(
                "effect.list_available: data envelope failed: {err}"
            ))
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
        let typed: EffectListAvailableArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "EffectListAvailableArgs",
            })?;

        let envelope = data_envelope_from_args(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
