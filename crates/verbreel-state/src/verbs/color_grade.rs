//! Shared minting helper for the two color-grade verbs `clip.auto_color`
//! and `clip.color_match` (issue #479).
//!
//! Both verbs derive a [`crate::color_dsp::GradeParams`] from frame
//! statistics and mint the **same** existing `color_correct` effect kind
//! on the target clip. The minting, params-map construction, target
//! lookup, and RFC-6902 patch shape are identical between them, so they
//! live here (DRY) rather than being re-implemented per verb. The math
//! that differs (auto white-balance vs Reinhard transfer) stays in
//! [`crate::color_dsp`].
//!
//! No new effect kind is introduced: `color_correct` is already in
//! `V1_EFFECT_KINDS`, so `effect.list_available` is unchanged.

use serde_json::{Map, Value, json};
use verbreel_types::{ClipId, EffectId};

use crate::color_dsp::GradeParams;
use crate::effect::{Effect, EffectKind};
use crate::project::Project;

/// The effect kind both color-grade verbs emit. Must stay one of the
/// already-registered `V1_EFFECT_KINDS`.
pub const GRADE_KIND: &str = "color_correct";

/// A located clip plus the indices needed to address it in an RFC-6902
/// path, and the lock state of the clip and its parent track.
#[derive(Debug, Clone)]
pub struct LocatedGradeTarget {
    /// Index of the parent track in `project.tracks`.
    pub track_idx: usize,
    /// Index of the clip in `track.clips`.
    pub clip_idx: usize,
    /// Whether the parent track is locked.
    pub track_locked: bool,
    /// Parent track id string.
    pub track_id: String,
    /// Whether the clip is locked.
    pub clip_locked: bool,
    /// Clip id string.
    pub clip_id: String,
    /// Snapshot of the clip's current effect list (to append to).
    pub effects: Vec<Effect>,
}

/// Locate the target clip for a color-grade verb.
#[must_use]
pub fn locate_grade_target(prior: &Project, clip_id: ClipId) -> Option<LocatedGradeTarget> {
    for (track_idx, track) in prior.tracks.iter().enumerate() {
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            if clip.id == clip_id {
                return Some(LocatedGradeTarget {
                    track_idx,
                    clip_idx,
                    track_locked: track.locked,
                    track_id: track.id.to_string(),
                    clip_locked: clip.locked,
                    clip_id: clip.id.to_string(),
                    effects: clip.effects.clone(),
                });
            }
        }
    }
    None
}

/// Build the ordered params map for a grade. The key order is fixed
/// (`brightness`, `contrast`, `saturation`, `temperature`, `tint`) so the
/// canonical-JSON hash is stable; values are already rounded by
/// [`GradeParams::rounded`].
#[must_use]
pub fn grade_params_map(grade: GradeParams) -> Map<String, Value> {
    let mut params = Map::new();
    params.insert("brightness".to_string(), number(grade.brightness));
    params.insert("contrast".to_string(), number(grade.contrast));
    params.insert("saturation".to_string(), number(grade.saturation));
    params.insert("temperature".to_string(), number(grade.temperature));
    params.insert("tint".to_string(), number(grade.tint));
    params
}

/// Serialize a finite `f64` as a JSON number.
///
/// # Panics
///
/// Panics if `value` is non-finite (NaN/±inf). The grade math never
/// produces a non-finite value — every divisor is guarded
/// (`channel_gain`, `reinhard_channel`, `ratio_or_one`, the levels
/// `span > EPSILON` branch) and `GradeParams::rounded` leaves finite
/// inputs finite — so this is unreachable in practice. We panic rather
/// than zero-fill: baking a silent `0.0` into a persisted grade is
/// exactly the fail-quiet behavior these verbs reject. A break in the
/// finiteness invariant must surface loudly at write time.
fn number(value: f64) -> Value {
    Value::Number(
        serde_json::Number::from_f64(value)
            .expect("grade params are guaranteed finite by the guarded grade math"),
    )
}

/// Build the RFC-6902 patch that appends a freshly minted `color_correct`
/// effect carrying `params` to the located clip.
///
/// # Panics
///
/// Never panics in practice: the only fallible step is
/// `EffectKind::new(GRADE_KIND)`, and [`GRADE_KIND`] is a non-empty
/// constant, so the `minLength: 1` check always succeeds.
#[must_use]
pub fn build_grade_patch(
    located: &LocatedGradeTarget,
    effect_id: EffectId,
    params: &Map<String, Value>,
) -> Value {
    let effect = Effect {
        id: effect_id,
        kind: EffectKind::new(GRADE_KIND.to_string())
            .expect("GRADE_KIND is a non-empty constant kind"),
        enabled: true,
        params: params.clone(),
        window: None,
    };

    let mut effects = located.effects.clone();
    effects.push(effect);

    json!([{
        "op": "replace",
        "path": format!("/tracks/{}/clips/{}/effects", located.track_idx, located.clip_idx),
        "value": effects,
    }])
}
