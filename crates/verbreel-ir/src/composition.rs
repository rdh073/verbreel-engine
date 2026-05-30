//! `CompositionInput` — the types-level boundary the graph builder lowers from.
//!
//! The composition types `Project`/`Track`/`Clip`/`Effect` live in
//! `verbreel-state`, which `verbreel-ir` MUST NOT depend on (dep-rule: `ir`
//! imports `verbreel-types` only). So the builder cannot read a state project
//! directly. Instead the caller (render side) lowers a state snapshot into the
//! neutral, types-level structs in this module, and the builder lowers *those*
//! into a DAG.
//!
//! Two invariants make the downstream graph deterministic and dep-rule clean:
//!
//! 1. **Every element carries its own stable [`IrNodeId`].** The builder mints
//!    nothing — node identity comes from the (already `UUIDv7`) ids the caller
//!    supplies, so the same composition produces the same node ids across runs.
//!    Minting via `IrNodeId::now()` here would make the graph time-dependent
//!    and break the determinism contract.
//! 2. **Args are pre-hashed.** Each element supplies `args_hash: [u8; 32]`,
//!    the SHA-256 of its canonicalized args, computed upstream by the caller
//!    via `verbreel_canon::jcs`. `verbreel-ir` never canonicalizes and never
//!    calls `serde_json::to_string` for identity — it folds the supplied hash
//!    straight into [`crate::CacheKey`].
//!
//! Spec references:
//! - §0.13 — content-addressed identity.
//! - Research 01 line 114 — `cache_hash` is a pure function of
//!   `(node_id, args_hash, upstream_cache_hashes, tick)`.

use verbreel_types::AssetHash;

use crate::node_id::IrNodeId;

/// SHA-256 of an element's canonicalized args, supplied pre-hashed by the
/// caller (computed upstream via `verbreel_canon::jcs`).
pub type ArgsHash = [u8; 32];

/// A neutral, types-level snapshot of a composition for the graph builder.
///
/// Ordered: `tracks[0]` is the bottommost layer, `tracks[last]` the topmost.
/// Within a track, `clips[0]` is the earliest in z-order. The builder
/// preserves this order so the output's topological walk matches track/clip
/// z-order (the stable-order test target).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositionInput {
    /// Tracks in bottom-to-top compositing order.
    pub tracks: Vec<TrackInput>,
    /// Identity + args of the single output node all tracks composite into.
    pub output: OutputInput,
    /// Engine tick this snapshot describes (§0.2 `TICK_RATE_HZ`).
    pub tick: u64,
}

/// One track: an ordered list of clips plus the composite node they feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackInput {
    /// Identity of the per-track composite node clips on this track feed.
    pub composite_node_id: IrNodeId,
    /// SHA-256 of the track-composite node's canonicalized args.
    pub composite_args_hash: ArgsHash,
    /// Clips on this track, earliest-first (z-order within the track).
    pub clips: Vec<ClipInput>,
}

/// One clip: a source node, the asset it reads, and its ordered effect chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipInput {
    /// Identity of this clip's source node.
    pub source_node_id: IrNodeId,
    /// SHA-256 of the source node's canonicalized args (in/out points, etc.).
    pub args_hash: ArgsHash,
    /// Content-addressed asset this clip's source reads, if any.
    ///
    /// Folded into the source node's args identity so swapping the underlying
    /// media busts the cache even when every other arg is unchanged.
    pub asset: Option<AssetHash>,
    /// Effects applied to this clip, in application order (first applied first).
    pub effects: Vec<EffectInput>,
}

/// One effect in a clip's chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectInput {
    /// Identity of this effect's transform node.
    pub node_id: IrNodeId,
    /// SHA-256 of the effect's canonicalized args (kind, params, keyframes).
    pub args_hash: ArgsHash,
}

/// Identity + args of the composition's single output node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputInput {
    /// Identity of the output node every track composite feeds.
    pub node_id: IrNodeId,
    /// SHA-256 of the output node's canonicalized args (resolution, etc.).
    pub args_hash: ArgsHash,
}
