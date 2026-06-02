//! `gen_image` text-to-image generation result model (issue #482).
//!
//! ## Data-only module — no verb, no engine surface.
//!
//! Like [`super::segment`] and [`super::face_mouth`], this module defines
//! **only** the canonical result struct the `verbreel-ai` `gen_image` sidecar
//! adapter decodes into. There is no [`Verb`](crate::reconstructor::Verb) impl
//! and it is **not** registered in [`super::default_registry`], so it adds zero
//! engine/dispatch surface and the conformance fixtures stay byte-frozen.
//!
//! The struct lives in `verbreel-state` (not `verbreel-ai`) so the result
//! shape is owned by the state crate — the single source of truth a future
//! composition-root verb will reuse. The allowed `verbreel-ai →
//! verbreel-state` edge lets the adapter import this; the reverse edge is
//! never created (no state verb imports an adapter).
//!
//! ## Generated image → content-addressed `asset.import` seam (issue #482)
//!
//! Unlike `segment` (whose sidecar freezes a matte into CAS itself and returns
//! only a hash), a freshly generated image is **not yet in CAS**. The sidecar
//! writes the produced image to a temp file and returns its [`image_path`]
//! here; a composition-root verb (deferred follow-up, mirroring the #474 /
//! #476 precedent) hands that path to the existing
//! [`asset_import`](super::asset_import) path, which hashes the bytes and
//! freezes them under `assets/<aa>/<sha>.<ext>` as an `image`-kind asset. The
//! CAS write stays in `verbreel-state`; this struct carries only the handle.
//! Returning a path (not base64 bytes) keeps the one-line stdout protocol
//! small and lets the existing magic-byte import probe own format detection.
//!
//! ## Determinism seam (issue #482)
//!
//! Generative diffusion is the explicit non-deterministic seam. The engine
//! runs the sidecar **once** with an explicit [`seed`] (same prompt + params +
//! seed → reproducible, per the engine's existing `seed` rule for
//! particle/glitch effects), freezes the produced image into the CAS-keyed
//! asset, and references it by hash thereafter — render is byte-deterministic
//! against the frozen asset, never per-render inference. [`seed`] is echoed
//! back so the caller can verify the sidecar honored the requested seed.
//!
//! ## Wire contract (issue #482)
//!
//! ```json
//! {"image_path":"/tmp/vr-gen/abcd.png","width":1024,"height":1024,"seed":42}
//! ```
//!
//! Field declaration order is the serde wire order (`snake_case` keys match
//! the JSON contract verbatim).
//!
//! [`image_path`]: GenImageData::image_path
//! [`seed`]: GenImageData::seed

use serde::{Deserialize, Serialize};

/// Result of a single text-to-image generation pass (issue #482).
///
/// Names the temp-file the sidecar wrote the produced image to, the image
/// dimensions, and the seed the sidecar generated with. It is a *handle*,
/// never the pixels on the wire — a composition-root verb imports
/// [`image_path`](Self::image_path) through the content-addressed
/// [`asset_import`](super::asset_import) path, so the non-deterministic
/// generation runs once and the frozen asset drives deterministic render.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenImageData {
    /// Filesystem path the sidecar wrote the generated image to. The
    /// composition root hands this to the content-addressed `asset.import`
    /// path, which hashes the bytes and freezes them under
    /// `assets/<aa>/<sha>.<ext>`. It is a transient handle, not a CAS path.
    pub image_path: String,
    /// Generated image width in pixels.
    pub width: u32,
    /// Generated image height in pixels.
    pub height: u32,
    /// Seed the sidecar generated with. Echoed from the request so the caller
    /// can confirm the honored seed (same prompt + params + seed →
    /// reproducible image bytes → identical CAS hash).
    pub seed: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> GenImageData {
        GenImageData {
            image_path: "/tmp/vr-gen/abcd.png".to_string(),
            width: 1024,
            height: 1024,
            seed: 42,
        }
    }

    #[test]
    fn serde_round_trip_preserves_struct() {
        let data = sample();
        let json = serde_json::to_string(&data).unwrap();
        let back: GenImageData = serde_json::from_str(&json).unwrap();
        assert_eq!(data, back);
    }

    #[test]
    fn wire_key_order_matches_contract() {
        let data = sample();
        let json = serde_json::to_string(&data).unwrap();
        // serde emits keys in declaration order; assert the byte layout the
        // sidecar contract (#482) pins so a field reorder is caught.
        assert_eq!(
            json,
            r#"{"image_path":"/tmp/vr-gen/abcd.png","width":1024,"height":1024,"seed":42}"#
        );
    }

    #[test]
    fn decodes_the_issue_wire_example() {
        let wire = r#"{"image_path":"/tmp/vr-gen/abcd.png","width":1024,"height":1024,"seed":42}"#;
        let data: GenImageData = serde_json::from_str(wire).unwrap();
        assert_eq!(data, sample());
    }
}
