//! verbreel-ir — Composition IR, tick-rate math, `cache_hash` derivation.
//!
//! Turns a neutral composition snapshot into a deterministic, content-addressed
//! render graph that `verbreel-render` walks node-by-node.
//!
//! - [`IrNodeId`] — strict `UUIDv7` newtype identifying a composition-graph
//!   node (§0.3 IDs).
//! - [`CacheKey`] — four-field struct + pure-function [`CacheKey::derive`]
//!   that returns a deterministic SHA-256 over a pinned byte layout.
//! - [`CompositionInput`] — the types-level boundary the builder lowers from
//!   (the dep-rule keeps `verbreel-state`'s `Project`/`Track`/`Clip` off-limits,
//!   so the caller lowers a state snapshot into this neutral struct first).
//! - [`build`] — lowers a [`CompositionInput`] into a [`Graph`] with
//!   deterministic node ordering matching track/clip z-order.
//! - [`Evaluator`] — keys every node (folding parents' cache hashes in so an
//!   upstream edit busts downstream cache) and exposes the evaluation-order
//!   iterator render walks to skip cached nodes.
//!
//! Research 01 line 114 calls out `cache_hash` as a pure function of
//! `(node_id, args_hash, upstream_cache_hashes, tick)`. The graph executor
//! establishes the invariant that a node's identity folds in its inputs'
//! identities, so cache validity propagates down the DAG without any node
//! re-litigating the hashing contract.
//!
//! Tick-rate math intentionally NOT redeclared here — it lives in
//! `verbreel_types::tick`.

#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(clippy::pedantic)]

pub mod builder;
pub mod cache_key;
pub mod composition;
pub mod evaluator;
pub mod graph;
pub mod node_id;

pub use builder::build;
pub use cache_key::CacheKey;
pub use composition::{ClipInput, CompositionInput, EffectInput, OutputInput, TrackInput};
pub use evaluator::{CacheStatus, EvalStep, Evaluator};
pub use graph::{Graph, GraphNode, NodeKind};
pub use node_id::IrNodeId;

// Re-export the workspace `Tick` newtype so downstream crates whose
// dep graph allows `verbreel-ir` but not direct `verbreel-types`
// (`verbreel-render`, future composition consumers) can spell typed
// tick parameters without widening their crate edge.
//
// `TICK_RATE_HZ` rides along for the same reason: `verbreel-render`'s
// tick->frame windowing math needs the engine tick base (§0.2), and the
// canonical definition lives in `verbreel_types::tick`. Re-exporting it
// keeps one source of truth instead of a redeclared private constant.
pub use verbreel_types::{TICK_RATE_HZ, Tick};

// `AssetHash` is already part of this crate's public graph surface
// (`NodeKind::Source { asset: Option<AssetHash> }`). Re-export it so the
// same downstream crates can name the asset a source node reads without a
// direct `verbreel-types` edge.
pub use verbreel_types::AssetHash;
