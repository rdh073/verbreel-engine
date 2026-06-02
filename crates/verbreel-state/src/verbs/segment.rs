//! `segment` subject-segmentation result model — green-screen-free matte.
//!
//! ## Data-only module — no verb, no engine surface.
//!
//! Like [`super::face_mouth`], this module defines **only** the canonical
//! result struct the `verbreel-ai` `segment` sidecar adapter decodes into.
//! There is no [`Verb`](crate::reconstructor::Verb) impl and it is **not**
//! registered in [`super::default_registry`], so it adds zero engine/dispatch
//! surface and the conformance fixtures stay byte-frozen.
//!
//! The struct lives in `verbreel-state` (not `verbreel-ai`) so the result
//! shape is owned by the state crate — the single source of truth a future
//! composition-root verb will reuse. The allowed `verbreel-ai →
//! verbreel-state` edge lets the adapter import this; the reverse edge is
//! never created (no state verb imports an adapter).
//!
//! ## Matte → `Clip.mask` kind `asset` wiring seam (issue #476)
//!
//! The sidecar produces a per-frame foreground alpha matte and freezes it as
//! a single content-addressed alpha asset under `assets/<aa>/<sha>.<ext>`,
//! imported as an `image`-kind asset. The returned [`SegmentMatteRef`] names
//! that asset by its [`AssetHash`]; a composition-root verb (deferred
//! follow-up, mirroring the #474 / #433 precedent) hands `matte_asset_hash`
//! to the existing `clip.set_mask` verb as
//! [`MaskKind::Asset`](crate::clip::MaskKind::Asset) `{asset_id, threshold?}`.
//! The deterministic compositor then applies that alpha mask with **no new
//! render surface** — reusing the path at `crates/verbreel-state/src/clip.rs`.
//! Keeping the result decode-only avoids changing any verb's pure
//! `compute_patch` output, so the engine surface and conformance fixtures
//! stay byte-frozen.
//!
//! ## Determinism seam (issue #476)
//!
//! ML inference is the explicit non-deterministic seam. The engine runs the
//! sidecar **once**, freezes the matte into the CAS-keyed alpha asset, and
//! references it by hash here — render is byte-deterministic against the
//! frozen matte, never per-render inference.
//!
//! ## Wire contract (issue #476)
//!
//! ```json
//! {"matte_asset_hash":"<sha256-hex>","width":1920,"height":1080,
//!  "frames":[{"t_tk":0},{"t_tk":8000}]}
//! ```
//!
//! Field declaration order is the serde wire order (`snake_case` keys match
//! the JSON contract verbatim). Frame order is preserved by `Vec` — a
//! `HashMap` would be wrong here (CLAUDE.md ordered-key rule).

use serde::{Deserialize, Serialize};
use verbreel_types::{AssetHash, Tick};

/// Reference to a frozen subject-segmentation alpha matte (issue #476).
///
/// The sidecar removes the background of a clip with no green screen and
/// freezes the per-frame foreground alpha into one content-addressed
/// `image`-kind asset; this struct names that asset by its CAS hash plus the
/// matte dimensions and per-frame project-time timing. It is a *reference*,
/// never the pixels — render resolves the matte from CAS, so the
/// non-deterministic ML pass runs once and the matte stays frozen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentMatteRef {
    /// Content-addressed hash of the frozen alpha-matte asset (imported as
    /// an `image`-kind asset, referenced via `Clip.mask` kind `asset`).
    pub matte_asset_hash: AssetHash,
    /// Matte width in pixels (source frame width).
    pub width: u32,
    /// Matte height in pixels (source frame height).
    pub height: u32,
    /// Per-frame project-time samples, in frame order.
    pub frames: Vec<SegmentMatteFrame>,
}

/// One matte-frame sample at a project-time tick.
///
/// `t_tk` is a [`Tick`] at the engine-wide 240,000 Hz rate
/// ([`TICK_RATE_HZ`](verbreel_types::TICK_RATE_HZ)); `Tick` is
/// `#[serde(transparent)]`, so it stays a bare JSON number on the wire — the
/// type only pins the rate expectation at the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentMatteFrame {
    /// Project-time tick of this matte frame (240,000 Hz).
    pub t_tk: Tick,
}

#[cfg(test)]
mod tests {
    use super::*;

    const MATTE: &str = "aa11bb22cc33dd44ee55ff660011223344556677889900112233445566778899";

    fn sample_ref() -> SegmentMatteRef {
        SegmentMatteRef {
            matte_asset_hash: AssetHash::new(MATTE).unwrap(),
            width: 1920,
            height: 1080,
            frames: vec![
                SegmentMatteFrame { t_tk: Tick::new(0) },
                SegmentMatteFrame {
                    t_tk: Tick::new(8000),
                },
            ],
        }
    }

    #[test]
    fn serde_round_trip_preserves_struct() {
        let matte = sample_ref();
        let json = serde_json::to_string(&matte).unwrap();
        let back: SegmentMatteRef = serde_json::from_str(&json).unwrap();
        assert_eq!(matte, back);
    }

    #[test]
    fn wire_key_order_matches_contract() {
        let matte = sample_ref();
        let json = serde_json::to_string(&matte).unwrap();
        // serde emits keys in declaration order; assert the byte layout the
        // sidecar contract (#476) pins so a field reorder is caught.
        assert_eq!(
            json,
            r#"{"matte_asset_hash":"aa11bb22cc33dd44ee55ff660011223344556677889900112233445566778899","width":1920,"height":1080,"frames":[{"t_tk":0},{"t_tk":8000}]}"#
        );
    }

    #[test]
    fn decodes_the_issue_wire_example() {
        let wire = r#"{"matte_asset_hash":"aa11bb22cc33dd44ee55ff660011223344556677889900112233445566778899","width":1920,"height":1080,"frames":[{"t_tk":0},{"t_tk":8000}]}"#;
        let matte: SegmentMatteRef = serde_json::from_str(wire).unwrap();
        assert_eq!(matte, sample_ref());
    }
}
