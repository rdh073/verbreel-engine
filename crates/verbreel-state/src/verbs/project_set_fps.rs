//! `project.set_fps` (§2.11) — third production verb in the engine.
//!
//! ## Spec quote (`spec/commands/project.md` §2.11, verbatim)
//!
//! > Changes the project frame rate (`Project.fps_num` / `fps_den`).
//! > All clip times are stored as integer ticks at 240,000 Hz (§0.2),
//! > independent of project fps — so no clip times change. The new fps
//! > only affects (a) frame snapping by subsequent mutating verbs and
//! > (b) the frame rate of subsequent renders.
//! >
//! > Existing clip positions may no longer fall on frame boundaries
//! > under the new fps. The engine does **not** proactively snap them
//! > — agents that want re-snapping should iterate the affected clips
//! > and re-call `clip.move` without `--exact_time`, which will
//! > frame-snap and emit `W_TIME_SNAPPED` for each adjusted value.
//! > Keyframes and markers are NOT frame-snapped (per §0.2 — keyframes
//! > interpolate on a sub-frame timeline and markers are display-only);
//! > their `time_tk` values remain exactly as stored regardless of fps
//! > changes, and `off_frame_count.keyframes` /
//! > `off_frame_count.markers` are informational only — there is no
//! > recovery action and no verb that would snap them. Same applies to
//! > `off_frame_count.audio_clips`: audio-clip positions are sub-frame
//! > addressable by construction.
//! >
//! > **Args**: `project_id: string`, `fps_num: integer` (≥1),
//! > `fps_den?: integer` (≥1; partial-update — omitted keeps the
//! > project's current `fps_den`; supply `--fps_den 1` explicitly when
//! > moving from an NTSC rate like `30000/1001` to an integer rate
//! > like `60/1`), `list_off_frame_entities?: boolean` (default `false`).
//! >
//! > **Returns** (`data`): `{ project_id; fps_num; fps_den;
//! > off_frame_count: { video_image_text_clips; audio_clips;
//! > keyframes; markers }; off_frame_entities?: {
//! > video_image_text_clip_ids; audio_clip_ids; keyframe_ids;
//! > marker_ids } }`. The `off_frame_entities` block is omitted
//! > entirely when the flag is false OR when all four counts are zero.
//! >
//! > **Errors**: `E_SCHEMA_VIOLATION` (`fps_num` or `fps_den` < 1).
//!
//! ## NTSC-footgun-guard partial-update semantics
//!
//! `fps_den` is **partial-update**: an omitted denominator keeps the
//! project's current `Project.fps_den`. This shape exists because an
//! earlier draft of the spec defaulted `fps_den` to `1`
//! unconditionally, which silently converted a `30000/1001` NTSC
//! project to `60/1` when an agent ran `project.set_fps --fps_num 60`
//! to bump the integer frame rate. Under v1, `fps_den` is `1` only for
//! projects already at `den == 1`, or when the caller asks for it
//! explicitly. To switch *into* an NTSC rate, pass both args
//! (`fps_num: 30000, fps_den: 1001`); to switch *out of* one, pass
//! both (`fps_num: 60, fps_den: 1`).
//!
//! The CLI surfaces a footgun-guard prompt when the caller's intent is
//! ambiguous (e.g. bumping `fps_num` on an NTSC project without also
//! supplying `fps_den`). That guard is a CLI-layer concern; this verb
//! enforces only the partial-update semantic itself.
//!
//! ## On-frame predicate (`is_off_frame`)
//!
//! A timestamp `t_tk` (in ticks at `TICK_RATE_HZ = 240_000`) lands on
//! a frame boundary under fps `n / d` iff:
//!
//! ```text
//! (t_tk * n) mod (TICK_RATE_HZ * d) == 0
//! ```
//!
//! Derivation: one frame spans `TICK_RATE_HZ * d / n` ticks. A
//! timestamp is on-boundary iff it is a multiple of the per-frame tick
//! count. The equivalent rearrangement above avoids rational
//! arithmetic — both sides are integers, so the predicate is exact for
//! every (n, d, t).
//!
//! Worked example — NTSC 30000/1001 fps at t = 8008 ticks:
//!
//! ```text
//! per-frame ticks  = 240_000 * 1001 / 30_000 = 8008
//! (8008 * 30_000) mod (240_000 * 1001)
//!   = 240_240_000 mod 240_240_000
//!   = 0   →  ON FRAME
//! ```
//!
//! Implementation uses `u128` for the multiplication to be overflow-
//! safe on long projects. The `Tick` value-range is i64 (~1.19 trillion
//! years at 240 kHz); multiplying by the schema-permitted `fps_num` cap
//! (currently u32) stays well inside `u128::MAX`.
//!
//! ## Reconstructor purity (§0.8)
//!
//! `compute_patch` and [`data_envelope`] share a private
//! [`compute_off_frame`] helper that walks `(project, fps_num,
//! fps_den, list_entities)` → `(OffFrameCount,
//! Option<OffFrameEntities>)`. `compute_patch` invokes it with `(prior,
//! new_num, new_den, args.list_off_frame_entities)` to count what
//! WOULD be off-frame after the change. `data_envelope` invokes it with
//! `(post_state, post_state.fps_num, post_state.fps_den,
//! args.list_off_frame_entities)` to count what IS off-frame in the
//! applied state. Because the patch only changes `fps_num` /
//! `fps_den` (clips, keyframes, and markers are untouched), both walks
//! see the same time values and produce identical counts and id lists
//! — the §0.8 reconstructor-purity contract holds by construction.
//!
//! `list_off_frame_entities` rides in `args`, so the reconstructor sees
//! the original execution-time value at replay and emits or omits the
//! entities block identically.
//!
//! ## Bucketing (`video_image_text_clips` vs `audio_clips`)
//!
//! §2.11 distinguishes:
//!
//! - `video_image_text_clips` — **actionable**. Subsequent
//!   `clip.move` (without `--exact_time`) will frame-snap each one and
//!   emit `W_TIME_SNAPPED`. These are clips on tracks of
//!   [`TrackKind::Video`] (carries video clips OR image-asset clips)
//!   or [`TrackKind::Text`].
//! - `audio_clips` — informational. Audio-clip positions are sub-frame
//!   addressable by construction (§0.2); the engine's snap step is a
//!   no-op for them. These are clips on [`TrackKind::Audio`].
//!
//! Effect tracks ([`TrackKind::Effect`]) carry no clips (§0.13
//! `check_effect_track_empty`); they're skipped silently. The
//! `clip_is_display_kind` helper in `invariants.rs` is a narrower
//! predicate (`Text` track OR Image asset) used by §0.13 invariants;
//! it does NOT include video clips and so cannot be reused for this
//! verb's broader bucket.
//!
//! ## Out of scope (this slice)
//!
//! - **NTSC footgun runtime enforcement** — CLI-layer concern; the
//!   engine only preserves partial-update semantics.
//! - **`clip.move` re-snap workflow / `W_TIME_SNAPPED` emission** — the
//!   §2.11 follow-up workflow is operator-driven (the agent iterates
//!   `off_frame_entities.video_image_text_clip_ids` and re-calls
//!   `clip.move`), not engine-driven.
//! - **Keyframe / marker snapping** — explicitly out per §2.11
//!   ("informational only" — no recovery action exists).
//! - **`verbreel-args` schema-crate population** — serde derive on
//!   [`ProjectSetFpsArgs`] is sufficient for this slice.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::{ProjectId, TICK_RATE_HZ, Tick};

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};
use crate::track::TrackKind;

/// Minimum legal value for `fps_num` and `fps_den` per §2.11
/// ("`fps_num: integer` (≥1)", "`fps_den?: integer` (≥1)"). A `0`
/// numerator would make the on-frame predicate's denominator
/// degenerate; a `0` denominator would divide-by-zero the per-frame
/// tick computation.
pub const FPS_MIN: u32 = 1;

/// Args for `project.set_fps`. Mirrors the §2.11 args list.
///
/// `fps_den` and `list_off_frame_entities` use `#[serde(default,
/// skip_serializing_if = "Option::is_none")]` so omitted optionals
/// round-trip cleanly and never appear as `null` on the wire
/// (matches the schema's "omitted = unchanged" semantics for
/// `fps_den` and the "default false" semantics for
/// `list_off_frame_entities`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSetFpsArgs {
    /// Target project id. Strongly-typed [`ProjectId`] so a verb-args
    /// payload carrying a non-`UUIDv7` string fails serde deserialize
    /// rather than reaching [`compute_patch`].
    pub project_id: ProjectId,

    /// New frame-rate numerator. Required. Must be `>= FPS_MIN` (1).
    pub fps_num: u32,

    /// Optional frame-rate denominator. **Partial-update**: `None`
    /// keeps the project's current `Project.fps_den`. Must be
    /// `>= FPS_MIN` (1) when supplied. See the module-level
    /// NTSC-footgun-guard rationale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps_den: Option<u32>,

    /// When `Some(true)`, the response's `off_frame_entities` block
    /// enumerates the ids of every entity that landed off-frame
    /// under the new fps. Default `false` (or `None`) — counts only.
    /// The block is also omitted when all four counts are zero, even
    /// if this flag is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_off_frame_entities: Option<bool>,
}

/// Counts of entities whose `time_tk` does not land on a frame
/// boundary under the new fps. Reported by every `project.set_fps`
/// call. See §2.11 for the actionable / informational distinction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OffFrameCount {
    /// Clips on [`TrackKind::Video`] or [`TrackKind::Text`] tracks
    /// whose `track_position_tk` is off-frame. **Actionable** — a
    /// follow-up `clip.move` without `--exact_time` will frame-snap
    /// each one.
    pub video_image_text_clips: usize,
    /// Clips on [`TrackKind::Audio`] tracks whose `track_position_tk`
    /// is off-frame. Informational only — audio positions are sub-
    /// frame addressable by construction.
    pub audio_clips: usize,
    /// Keyframes (across all clips) whose `time_tk` is off-frame.
    /// Informational only — keyframes interpolate on a sub-frame
    /// timeline.
    pub keyframes: usize,
    /// Markers whose `time_tk` is off-frame. Informational only —
    /// markers are display annotations.
    pub markers: usize,
}

/// Per-entity-class id lists for the `off_frame_entities` block.
/// Ids are emitted as their canonical `.to_string()` form (lower-case
/// `UUIDv7`), matching the convention used elsewhere in the verb layer.
///
/// Emitted in the response iff `args.list_off_frame_entities ==
/// Some(true)` AND at least one of the four counts is non-zero
/// (per §2.11 — "omitted entirely when the flag is false or when all
/// four counts are zero, to keep typical responses small").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OffFrameEntities {
    /// Ids of every clip counted in
    /// [`OffFrameCount::video_image_text_clips`], in track-then-clip
    /// declaration order.
    pub video_image_text_clip_ids: Vec<String>,
    /// Ids of every clip counted in [`OffFrameCount::audio_clips`].
    pub audio_clip_ids: Vec<String>,
    /// Ids of every keyframe counted in [`OffFrameCount::keyframes`],
    /// in track-then-clip-then-keyframe declaration order.
    pub keyframe_ids: Vec<String>,
    /// Ids of every marker counted in [`OffFrameCount::markers`], in
    /// `Project.markers[]` declaration order.
    pub marker_ids: Vec<String>,
}

/// Envelope `data` shape returned by `project.set_fps`. Per §2.11:
/// the full post-state fps plus derived off-frame analytics.
///
/// First verb whose `data` envelope carries derived (post-mutation,
/// not args-echo) information: `off_frame_count` is computed by
/// walking the project graph rather than copied from `args`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSetFpsData {
    /// The target project id (echoed from `args.project_id`).
    pub project_id: ProjectId,
    /// The post-state `Project.fps_num` (the value the patch installed).
    pub fps_num: u32,
    /// The post-state `Project.fps_den` (unchanged from prior when
    /// `args.fps_den` was omitted; otherwise the supplied value).
    pub fps_den: u32,
    /// Derived counts of off-frame entities under the new fps.
    pub off_frame_count: OffFrameCount,
    /// Optional id lists. Emitted iff `args.list_off_frame_entities
    /// == Some(true)` AND at least one off-frame entity exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off_frame_entities: Option<OffFrameEntities>,
}

/// Verb-level errors surfaced by [`compute_patch`]. Both variants map
/// onto §2.11's `E_SCHEMA_VIOLATION` once wired into the kernel's
/// error-translation layer. Routed through [`VerbError::BadArgs`] per
/// the §0.7 arg-shape-failure taxonomy.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProjectSetFpsError {
    /// `args.fps_num < FPS_MIN`. Spec §2.11 requires `fps_num >= 1`.
    #[error("project.set_fps: fps_num {value} < {min}")]
    FpsNumOutOfRange {
        /// The offending numerator value.
        value: u32,
        /// [`FPS_MIN`].
        min: u32,
    },

    /// `args.fps_den < FPS_MIN` when supplied. Spec §2.11 requires
    /// `fps_den >= 1`. Omitted `fps_den` cannot reach this variant —
    /// the partial-update path uses the prior project's `fps_den`,
    /// which is itself already invariant-checked at deserialize time.
    #[error("project.set_fps: fps_den {value} < {min}")]
    FpsDenOutOfRange {
        /// The offending denominator value.
        value: u32,
        /// [`FPS_MIN`].
        min: u32,
    },
}

/// Pure on-frame predicate. Returns `true` iff `t_tk` does NOT land
/// on a frame boundary under `fps_num / fps_den`.
///
/// Formula: a timestamp is on-boundary iff
/// `(t_tk * fps_num) mod (TICK_RATE_HZ * fps_den) == 0`. See the
/// module-level rustdoc for the derivation and a worked NTSC example.
///
/// `u128` arithmetic — `Tick`'s `i64` range times `fps_num` (`u32`)
/// tops out at ~`i64::MAX` * `u32::MAX` ≈ 4 × 10^28, well inside
/// `u128::MAX` (≈ 3.4 × 10^38). A negative `t_tk` (which would already
/// fail the schema's `minimum: 0` rule) clamps to 0 so the predicate
/// stays defined rather than panicking on `as u128`.
#[must_use]
pub fn is_off_frame(t_tk: Tick, fps_num: u32, fps_den: u32) -> bool {
    // Tick is i64; schema enforces `minimum: 0` so the cast is
    // lossless in valid projects. A defensive clamp guarantees the
    // helper is total even on hand-edited / mid-patch invalid input.
    let t = u128::try_from(t_tk.get().max(0)).unwrap_or(0);
    let n = u128::from(fps_num);
    let d = u128::from(fps_den);
    let frame_ticks = u128::from(TICK_RATE_HZ) * d;
    // fps_den == 0 would make frame_ticks zero and the modulo panic.
    // compute_patch rejects `fps_den == 0` before calling this helper,
    // and prior.fps_den is invariant-checked at deserialize, so the
    // guard is unreachable in normal flow — but kept so direct callers
    // (tests, future verbs) cannot trigger UB.
    if frame_ticks == 0 {
        return true;
    }
    (t * n) % frame_ticks != 0
}

/// Walk the project graph once; bucket every off-frame entity by
/// class. Returns the four counts plus, when `list_entities` is true
/// AND at least one count is non-zero, the parallel id lists.
///
/// Iteration order is deterministic — `project.tracks` in declared
/// order, each track's `clips` in declared order, each clip's
/// `keyframes` in declared order, then `project.markers` in declared
/// order. Callers can rely on the order so id-list outputs are
/// reproducible across runs.
///
/// Shared by [`compute_patch`] (called with `(prior, new_num,
/// new_den, list_entities)` — "what would be off-frame after the
/// change") and [`data_envelope`] (called with `(post_state,
/// post_state.fps_num, post_state.fps_den, list_entities)` — "what
/// IS off-frame in the applied state"). Both walks produce identical
/// counts because the patch only changes `fps_num` / `fps_den`; the
/// clip / keyframe / marker time values are untouched.
fn compute_off_frame(
    project: &Project,
    fps_num: u32,
    fps_den: u32,
    list_entities: bool,
) -> (OffFrameCount, Option<OffFrameEntities>) {
    let mut counts = OffFrameCount {
        video_image_text_clips: 0,
        audio_clips: 0,
        keyframes: 0,
        markers: 0,
    };
    let mut entities = if list_entities {
        Some(OffFrameEntities {
            video_image_text_clip_ids: Vec::new(),
            audio_clip_ids: Vec::new(),
            keyframe_ids: Vec::new(),
            marker_ids: Vec::new(),
        })
    } else {
        None
    };

    for track in &project.tracks {
        for clip in &track.clips {
            if is_off_frame(clip.track_position_tk, fps_num, fps_den) {
                match track.kind {
                    TrackKind::Video | TrackKind::Text => {
                        counts.video_image_text_clips += 1;
                        if let Some(e) = entities.as_mut() {
                            e.video_image_text_clip_ids.push(clip.id.to_string());
                        }
                    }
                    TrackKind::Audio => {
                        counts.audio_clips += 1;
                        if let Some(e) = entities.as_mut() {
                            e.audio_clip_ids.push(clip.id.to_string());
                        }
                    }
                    TrackKind::Effect => {
                        // §0.13 `check_effect_track_empty` makes this
                        // unreachable on a well-formed project; skip
                        // silently rather than panic.
                    }
                }
            }
            for kf in &clip.keyframes {
                if is_off_frame(kf.time_tk, fps_num, fps_den) {
                    counts.keyframes += 1;
                    if let Some(e) = entities.as_mut() {
                        e.keyframe_ids.push(kf.id.to_string());
                    }
                }
            }
        }
    }
    for marker in &project.markers {
        if is_off_frame(marker.time_tk, fps_num, fps_den) {
            counts.markers += 1;
            if let Some(e) = entities.as_mut() {
                e.marker_ids.push(marker.id.to_string());
            }
        }
    }

    let any =
        counts.video_image_text_clips + counts.audio_clips + counts.keyframes + counts.markers > 0;
    let entities = entities.filter(|_| any);
    (counts, entities)
}

/// Compute the RFC 6902 patch and derived off-frame analytics for a
/// `project.set_fps` call.
///
/// Pure function — no I/O, no clock, no RNG. Validates the args,
/// derives the effective `(new_num, new_den)` (honoring the
/// partial-update on `fps_den`), walks the project graph to count
/// off-frame entities under the new fps, and returns:
///
/// - the patch as a [`serde_json::Value`] (an RFC 6902 op array with
///   either one `replace` op on `/fps_num` (when `args.fps_den` is
///   `None`) or two `replace` ops on `/fps_num` and `/fps_den` (when
///   supplied)),
/// - the off-frame counts (returned separately so the caller doesn't
///   re-walk to recover them), and
/// - the optional off-frame entities block (per the
///   `list_off_frame_entities`-flag-AND-non-zero-counts semantics).
///
/// # Errors
///
/// Returns [`ProjectSetFpsError`]:
///
/// - [`ProjectSetFpsError::FpsNumOutOfRange`] when `args.fps_num <
///   FPS_MIN`.
/// - [`ProjectSetFpsError::FpsDenOutOfRange`] when `args.fps_den ==
///   Some(d)` with `d < FPS_MIN`.
pub fn compute_patch(
    prior: &Project,
    args: &ProjectSetFpsArgs,
) -> Result<(Value, OffFrameCount, Option<OffFrameEntities>), ProjectSetFpsError> {
    if args.fps_num < FPS_MIN {
        return Err(ProjectSetFpsError::FpsNumOutOfRange {
            value: args.fps_num,
            min: FPS_MIN,
        });
    }
    let new_den = match args.fps_den {
        Some(d) => {
            if d < FPS_MIN {
                return Err(ProjectSetFpsError::FpsDenOutOfRange {
                    value: d,
                    min: FPS_MIN,
                });
            }
            d
        }
        None => prior.fps_den,
    };
    let new_num = args.fps_num;

    let list_entities = args.list_off_frame_entities.unwrap_or(false);
    let (counts, entities) = compute_off_frame(prior, new_num, new_den, list_entities);

    let patch = if args.fps_den.is_some() {
        json!([
            { "op": "replace", "path": "/fps_num", "value": new_num },
            { "op": "replace", "path": "/fps_den", "value": new_den },
        ])
    } else {
        json!([
            { "op": "replace", "path": "/fps_num", "value": new_num },
        ])
    };

    Ok((patch, counts, entities))
}

/// Build the verb's envelope `data` from `(args, post_state)`. Pure —
/// this is the function the reconstructor exercises during replay.
///
/// Per §0.8 reconstructor purity, every field on
/// [`ProjectSetFpsData`] is derivable from the recorded inputs:
///
/// - `project_id`: cloned from `args.project_id`.
/// - `fps_num` / `fps_den`: read from `post_state` (the values the
///   patch installed). Reading `args.fps_num` would also work for
///   `fps_num` (always required) but not for `fps_den` (the
///   partial-update path leaves `args.fps_den == None`), so
///   `post_state` is the single source of truth for both.
/// - `off_frame_count` / `off_frame_entities`: re-derived by walking
///   `post_state` with `post_state.fps_num` / `post_state.fps_den`.
///   Because the patch only changes the fps fields (clips, keyframes,
///   and markers are untouched), this walk produces identical counts
///   and id lists to the `compute_patch` walk over `prior` with the new
///   fps.
#[must_use]
pub fn data_envelope(args: &ProjectSetFpsArgs, post_state: &Project) -> ProjectSetFpsData {
    let list_entities = args.list_off_frame_entities.unwrap_or(false);
    let (counts, entities) = compute_off_frame(
        post_state,
        post_state.fps_num,
        post_state.fps_den,
        list_entities,
    );
    ProjectSetFpsData {
        project_id: args.project_id,
        fps_num: post_state.fps_num,
        fps_den: post_state.fps_den,
        off_frame_count: counts,
        off_frame_entities: entities,
    }
}

/// Funnel [`ProjectSetFpsError`] into the verb-layer [`VerbError`]
/// taxonomy. Both variants are argument-shape failures
/// (`E_SCHEMA_VIOLATION` per §0.7 / §2.11), so both map to
/// [`VerbError::BadArgs`]. None of these are §0.13 engine invariants
/// — fps-bounds are arg-shape rules.
impl From<ProjectSetFpsError> for VerbError {
    fn from(value: ProjectSetFpsError) -> Self {
        VerbError::BadArgs {
            detail: value.to_string(),
        }
    }
}

/// The §0.8 verb for `project.set_fps`. Registered in a
/// [`crate::VerbRegistry`] so the §0.8 startup gate
/// ([`crate::validate_reconstructors`]) can exercise its `reconstruct`
/// path against a recorded fixture, and so
/// [`crate::lifecycle::ProjectStore::mutate_via_verb`] can route
/// forward calls through its `compute_patch` path.
///
/// Pure on both legs of the trait — no I/O, no clock, no RNG, no
/// patch / warnings inspection during reconstruct. The forward leg
/// (`compute_patch`) deserialises `args` into [`ProjectSetFpsArgs`],
/// calls the freestanding [`compute_patch`] helper, synthesises a
/// post-state by cloning `prior` and overwriting its fps fields, then
/// builds the data envelope via [`data_envelope`] — going through the
/// reconstructor's view so the forward + replay paths are byte-
/// identical by construction.
#[derive(Debug, Default)]
pub struct ProjectSetFpsVerb;

impl Verb for ProjectSetFpsVerb {
    fn verb(&self) -> &'static str {
        "project.set_fps"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: ProjectSetFpsArgs =
            serde_json::from_value(args.clone()).map_err(|e| VerbError::BadArgs {
                detail: format!("project.set_fps: args deserialize failed: {e}"),
            })?;

        let (patch_value, _counts, _entities) = compute_patch(prior, &typed)?;

        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|e| {
            VerbError::Custom(format!("project.set_fps: patch construction failed: {e}"))
        })?;

        // Synthesise the post-state from `(prior, args)` — the same
        // state the kernel will produce by applying `patch`. The
        // `data_envelope` then reads `post_state.fps_num` /
        // `post_state.fps_den` and re-walks for counts, so the
        // forward-path data field is byte-identical to what the
        // reconstructor would build from the recorded 5-tuple.
        let mut post_state = prior.clone();
        post_state.fps_num = typed.fps_num;
        if let Some(d) = typed.fps_den {
            post_state.fps_den = d;
        }
        let envelope = data_envelope(&typed, &post_state);
        let data = serde_json::to_value(&envelope).map_err(|e| {
            VerbError::Custom(format!(
                "project.set_fps: data envelope serialize failed: {e}"
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
        let typed: ProjectSetFpsArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ProjectSetFpsArgs",
            })?;
        let envelope = data_envelope(&typed, post_state);
        serde_json::to_value(&envelope).map_err(|e| ReconstructError::Custom(e.to_string()))
    }
}
