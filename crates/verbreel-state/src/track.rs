//! [`Track`] and [`TrackKind`]. Mirrors `$defs/Track` from
//! `spec/project-schema.json`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use verbreel_types::TrackId;

use crate::clip::Clip;

/// The kind of a track. Spec `$defs/Track::kind` enum.
///
/// - `Video` — holds video clips composited onto the canvas.
/// - `Audio` — holds audio clips mixed into the master bus.
/// - `Text` — holds text overlay clips (lower-thirds, captions).
/// - `Effect` — adjustment-layer track; carries only track-level
///   `effects[]`; the `clips` array MUST be empty per §0.13.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackKind {
    /// Video track. Holds video clips and (optionally) track-level effects.
    Video,
    /// Audio track. Holds audio clips and (optionally) track-bus effects.
    Audio,
    /// Text track. Holds text overlay clips and (optionally) typography effects.
    Text,
    /// Effect track. Holds ONLY track-level effects; `clips` MUST be `[]`.
    Effect,
}

/// A track in the project graph.
///
/// Schema reference: `spec/project-schema.json` `$defs/Track`.
///
/// Required: `id`, `kind`, `name`, `clips`. Optional fields take serde
/// defaults that match the schema defaults (`false` / `1.0` / `0.0`
/// / `[]`).
//
// `clippy::struct_excessive_bools` would prefer ≤ 3 bool fields in a
// struct; Track has 4 (`muted`, `solo`, `locked`, `hidden`) directly
// mandated by `spec/project-schema.json` `$defs/Track`. Refactoring
// into a state-machine enum would either drop information the schema
// requires or invent a synthetic field shape that fails round-trip.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    /// Track ID — strongly-typed [`TrackId`] (`UuidV7`).
    pub id: TrackId,

    /// Track kind. See [`TrackKind`].
    pub kind: TrackKind,

    /// Display name. Spec: 1..=128 chars (range-enforcement is currently
    /// only at the schema layer; the typed validator lands with §0.13).
    pub name: String,

    /// Clips on this track. See [`Clip`] for the per-clip shape.
    #[serde(default)]
    pub clips: Vec<Clip>,

    /// Whether the track is muted. Default `false`.
    #[serde(default)]
    pub muted: bool,

    /// Whether the track is solo'd. Default `false`.
    #[serde(default)]
    pub solo: bool,

    /// Whether the track is locked (mutating verbs return `E_LOCKED`).
    /// Default `false`.
    #[serde(default)]
    pub locked: bool,

    /// Whether the track is hidden in the UI. Default `false`.
    #[serde(default)]
    pub hidden: bool,

    /// Linear gain (audio tracks only). Range 0..=4. Default `1.0`.
    #[serde(default = "default_volume")]
    pub volume: f64,

    /// Stereo pan (audio tracks only). Range -1..=1. Default `0.0`.
    #[serde(default)]
    pub pan: f64,

    /// Track-level effects (adjustment layers). Default `[]`.
    ///
    // TODO(#19 follow-up): replace `Vec<serde_json::Value>` with
    // `Vec<Effect>` once `Effect` is typed.
    #[serde(default)]
    pub effects: Vec<Value>,
}

fn default_volume() -> f64 {
    1.0
}
