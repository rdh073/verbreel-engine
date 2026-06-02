//! `face_mouth` perception result model — auto-director perception floor.
//!
//! ## Data-only module — no verb, no engine surface.
//!
//! Unlike the verb modules in this directory, this module defines **only**
//! the canonical result structs the `verbreel-ai` `face_mouth` sidecar
//! adapter decodes into. There is no [`Verb`](crate::reconstructor::Verb)
//! impl and it is **not** registered in [`super::default_registry`], so it
//! adds zero engine/dispatch surface and the conformance fixtures stay
//! byte-frozen.
//!
//! The structs live in `verbreel-state` (not `verbreel-ai`) so the result
//! shape is owned by the state crate — the single source of truth that a
//! future composition-root verb will reuse. The allowed `verbreel-ai →
//! verbreel-state` edge lets the adapter import these; the reverse edge is
//! never created (no state verb imports an adapter).
//!
//! ## Wire contract (issue #474)
//!
//! ```json
//! {"faces":[
//!   {"speaker_id":"f0",
//!    "bbox_trace":[{"t_tk":0,"x":120,"y":80,"w":300,"h":300}],
//!    "mouth_open_trace":[{"t_tk":0,"mar":0.04}]}
//! ]}
//! ```
//!
//! Field declaration order is the serde wire order (`snake_case` keys match
//! the JSON contract verbatim). Frame order is preserved by `Vec` — a
//! `HashMap` would be wrong here (CLAUDE.md ordered-key rule).

use serde::{Deserialize, Serialize};
use verbreel_types::Tick;

/// Per-tracked-face perception traces returned by the `face_mouth` sidecar.
///
/// One artifact, two consumers: `bbox_trace` drives auto-reframe and
/// `mouth_open_trace` (Mouth Aspect Ratio) feeds downstream active-speaker
/// inference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FaceMouthTrace {
    /// One entry per stable tracked face identity.
    pub faces: Vec<FaceTrace>,
}

/// Time-series traces for a single tracked face identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FaceTrace {
    /// Stable identity assigned by the sidecar's `IoU` tracker (e.g. `"f0"`).
    pub speaker_id: String,
    /// Bounding-box samples over time, for auto-reframe.
    pub bbox_trace: Vec<FaceBBoxSample>,
    /// Mouth-Aspect-Ratio samples over time, for active-speaker inference.
    pub mouth_open_trace: Vec<MouthOpenSample>,
}

/// One bounding-box sample at a project-time tick.
///
/// Pixel coordinates are integers to match the wire example
/// (`x:120, y:80, w:300, h:300`); `t_tk` is a [`Tick`] at the engine-wide
/// 240,000 Hz rate ([`TICK_RATE_HZ`](verbreel_types::TICK_RATE_HZ)).
/// `Tick` is `#[serde(transparent)]`, so it stays a bare JSON number on the
/// wire — the type only pins the rate expectation at the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaceBBoxSample {
    /// Project-time tick of this sample (240,000 Hz).
    pub t_tk: Tick,
    /// Bounding-box left edge in pixels.
    pub x: i64,
    /// Bounding-box top edge in pixels.
    pub y: i64,
    /// Bounding-box width in pixels.
    pub w: i64,
    /// Bounding-box height in pixels.
    pub h: i64,
}

/// One Mouth-Aspect-Ratio sample at a project-time tick.
///
/// `t_tk` is a [`Tick`] at the engine-wide 240,000 Hz rate; it serializes as
/// a bare JSON number via `#[serde(transparent)]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MouthOpenSample {
    /// Project-time tick of this sample (240,000 Hz).
    pub t_tk: Tick,
    /// Mouth Aspect Ratio: `vertical_lip_gap / horizontal_lip_width`.
    pub mar: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_trace() -> FaceMouthTrace {
        FaceMouthTrace {
            faces: vec![FaceTrace {
                speaker_id: "f0".to_string(),
                bbox_trace: vec![FaceBBoxSample {
                    t_tk: Tick::new(0),
                    x: 120,
                    y: 80,
                    w: 300,
                    h: 300,
                }],
                mouth_open_trace: vec![MouthOpenSample {
                    t_tk: Tick::new(0),
                    mar: 0.04,
                }],
            }],
        }
    }

    #[test]
    fn serde_round_trip_preserves_struct() {
        let trace = sample_trace();
        let json = serde_json::to_string(&trace).unwrap();
        // Assert the value round-trips, not the bytes: this test guards
        // serde semantics (deserialize(serialize(x)) == x) independently of
        // float formatting. `wire_key_order_matches_contract` covers the
        // exact byte layout (serde_json's ryu output is deterministic).
        let back: FaceMouthTrace = serde_json::from_str(&json).unwrap();
        assert_eq!(trace, back);
    }

    #[test]
    fn wire_key_order_matches_contract() {
        let trace = sample_trace();
        let json = serde_json::to_string(&trace).unwrap();
        // serde emits keys in declaration order; assert the byte layout the
        // sidecar contract (#474) pins so a field reorder is caught.
        assert_eq!(
            json,
            r#"{"faces":[{"speaker_id":"f0","bbox_trace":[{"t_tk":0,"x":120,"y":80,"w":300,"h":300}],"mouth_open_trace":[{"t_tk":0,"mar":0.04}]}]}"#
        );
    }

    #[test]
    fn decodes_the_issue_wire_example() {
        let wire = r#"{"faces":[{"speaker_id":"f0","bbox_trace":[{"t_tk":0,"x":120,"y":80,"w":300,"h":300}],"mouth_open_trace":[{"t_tk":0,"mar":0.04}]}]}"#;
        let trace: FaceMouthTrace = serde_json::from_str(wire).unwrap();
        assert_eq!(trace, sample_trace());
    }
}
