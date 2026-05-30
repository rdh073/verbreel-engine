//! IR-to-render input adapter with deterministic frame windowing.
//!
//! `verbreel-render` consumes the neutral [`verbreel_ir`] graph types
//! ([`Graph`] / [`Evaluator`]) — never `verbreel-state`'s `Project` (dep-rule:
//! `render -> ir + codec-native`). This module lowers a keyed graph into a
//! [`RenderPlan`]: the ordered set of compositing layers the GPU stage walks,
//! plus the deterministic frame window (tick -> frame-index mapping) the
//! encode stage clocks against.
//!
//! ## Why the windowing lives here, not in the GPU stage
//!
//! The engine tick base is 240,000 Hz (§0.2). A render targets an output frame
//! rate (e.g. 30 fps), so the adapter is the single place that maps an engine
//! tick to a non-negative frame index with a pinned rounding rule. Pinning it
//! here keeps the GPU stage tick-agnostic (it only ever sees frame indices and
//! pixel buffers) and keeps the rounding rule in one auditable spot for the
//! determinism contract.

use verbreel_ir::{AssetHash, CacheStatus, Evaluator, Graph, NodeKind, TICK_RATE_HZ};

/// One compositing layer the GPU stage draws, in bottom-to-top order.
///
/// A layer corresponds to one track's composite output. `source_asset` is the
/// content-addressed media the track's (single, v1-floor) source reads, if any;
/// the GPU stage decodes it to a yuv420p frame and composites it at
/// `alpha_q16`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderLayer {
    /// Content-addressed source media for this layer, if the track has a
    /// media source. `None` is a generated/empty layer (composited as
    /// transparent at the v1 floor).
    pub source_asset: Option<AssetHash>,
    /// Source-over alpha, fixed-point 0..=65535 mapping 0.0..=1.0. The v1
    /// floor composites opaque video, so this is `65535` unless the caller
    /// overrides it; the field exists so effect-driven opacity has a home
    /// without an API break.
    pub alpha_q16: u16,
    /// The composite node's content-addressed cache hash, taken from the
    /// [`Evaluator`]. This is the address render keys a cached composited
    /// layer by — an upstream edit changes this hash (the evaluator folds
    /// parents' hashes in), so the GPU stage knows to recomposite.
    pub cache_hash: [u8; 32],
}

/// A deterministic frame window: the half-open tick range `[start, end)` an
/// output frame covers, plus its zero-based frame index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameWindow {
    /// Zero-based output frame index.
    pub index: u64,
    /// Inclusive start tick of the window (engine ticks, 240 kHz base).
    pub start_tick: u64,
    /// Exclusive end tick of the window.
    pub end_tick: u64,
}

/// The lowered render plan for one composition graph at one output tick.
///
/// Built purely from a keyed [`Evaluator`] — same graph in, same plan out, so
/// the plan itself never introduces nondeterminism.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderPlan {
    /// Compositing layers, bottom-to-top (track z-order). The GPU stage walks
    /// this slice in order.
    pub layers: Vec<RenderLayer>,
    /// The engine tick this plan composites (the graph's tick).
    pub tick: u64,
}

impl RenderPlan {
    /// Lower a keyed graph into a render plan.
    ///
    /// Walks the evaluator's topological steps, collecting one [`RenderLayer`]
    /// per [`NodeKind::Composite`] node in build order (which the builder pins
    /// to track z-order, bottom-to-top). Each composite's first parent chain is
    /// followed back to the [`NodeKind::Source`] node so the layer carries the
    /// media asset; effect nodes between them do not change which asset is read.
    ///
    /// The `evaluator` supplies each composite layer's content-addressed
    /// [`cache_hash`](RenderLayer::cache_hash): keying the layer to its
    /// upstream-folded hash is what lets the GPU stage skip recompositing when
    /// nothing upstream changed.
    #[must_use]
    pub fn from_evaluator(graph: &Graph, evaluator: &Evaluator<'_>) -> Self {
        let mut layers = Vec::new();

        for node in graph.nodes_in_order() {
            let NodeKind::Composite = node.kind else {
                continue;
            };
            // A composite's parents are its clips' chain-tails, in z-order.
            // The v1 floor renders one source per track: take the first clip
            // chain and resolve its source asset.
            let source_asset = node
                .parents
                .first()
                .and_then(|chain_tail| resolve_source_asset(graph, *chain_tail));
            // The evaluator keyed every node in `new`, so a composite that came
            // from the same graph always has a cache hash; default to zeros only
            // for a hand-built graph whose node set and evaluator disagree.
            let cache_hash = evaluator.cache_hash(node.id).unwrap_or([0u8; 32]);
            layers.push(RenderLayer {
                source_asset,
                alpha_q16: u16::MAX,
                cache_hash,
            });
        }

        Self {
            layers,
            tick: graph.tick(),
        }
    }

    /// Number of compositing layers.
    #[must_use]
    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    /// Whether the plan would skip a layer the evaluator reports as a cache
    /// hit. Pure helper exposed so the GPU stage and tests share one rule.
    #[must_use]
    pub fn is_layer_live(status: CacheStatus) -> bool {
        matches!(status, CacheStatus::Miss)
    }
}

/// Follow a clip chain tail (the source itself, or the last effect) back to its
/// [`NodeKind::Source`] node and return the asset it reads.
///
/// Each effect node has exactly one parent (the builder's invariant), so the
/// walk is a straight line with no branching; it terminates at the source.
fn resolve_source_asset(graph: &Graph, mut node_id: verbreel_ir::IrNodeId) -> Option<AssetHash> {
    loop {
        let node = graph.node(node_id)?;
        match &node.kind {
            NodeKind::Source { asset } => return asset.clone(),
            // Effect nodes have exactly one parent; walk up to the source.
            NodeKind::Effect => node_id = *node.parents.first()?,
            // A composite or output node cannot be a clip-chain tail; the
            // graph is malformed if we reach one here.
            NodeKind::Composite | NodeKind::Output => return None,
        }
    }
}

/// Map an engine tick to its zero-based output frame index at `fps_num/fps_den`.
///
/// Pinned rounding: integer floor of `tick * fps_num / (TICK_RATE_HZ * fps_den)`.
/// Floor (not round-to-nearest) is the deterministic choice because it makes
/// the frame window half-open `[start, end)` partition the tick line with no
/// gaps or overlaps — every tick belongs to exactly one frame.
///
/// # Errors
///
/// Returns [`crate::RenderError::InvalidInput`] when `fps_num` or `fps_den` is
/// zero (a zero rate has no frame mapping).
pub fn tick_to_frame_index(
    tick: u64,
    fps_num: u32,
    fps_den: u32,
) -> Result<u64, crate::RenderError> {
    if fps_num == 0 || fps_den == 0 {
        return Err(crate::RenderError::InvalidInput {
            detail: format!("frame rate {fps_num}/{fps_den} has a zero term"),
        });
    }
    // u128 to avoid overflow on the tick * fps_num product.
    let num = u128::from(tick) * u128::from(fps_num);
    let den = u128::from(TICK_RATE_HZ) * u128::from(fps_den);
    Ok(u64::try_from(num / den).unwrap_or(u64::MAX))
}

/// Build the deterministic [`FrameWindow`] for output frame `index` at
/// `fps_num/fps_den`.
///
/// The window is `[index * ticks_per_frame, (index + 1) * ticks_per_frame)`
/// where `ticks_per_frame = TICK_RATE_HZ * fps_den / fps_num`, computed in
/// rationals so non-integer frame durations (e.g. NTSC 30000/1001) don't drift:
/// the boundary tick of frame `n` is the floor of `n * TICK_RATE_HZ * fps_den
/// / fps_num`, so consecutive windows abut exactly.
///
/// # Errors
///
/// Returns [`crate::RenderError::InvalidInput`] when `fps_num` or `fps_den` is
/// zero.
pub fn frame_window(
    index: u64,
    fps_num: u32,
    fps_den: u32,
) -> Result<FrameWindow, crate::RenderError> {
    if fps_num == 0 || fps_den == 0 {
        return Err(crate::RenderError::InvalidInput {
            detail: format!("frame rate {fps_num}/{fps_den} has a zero term"),
        });
    }
    let boundary = |n: u128| -> u64 {
        let num = n * u128::from(TICK_RATE_HZ) * u128::from(fps_den);
        u64::try_from(num / u128::from(fps_num)).unwrap_or(u64::MAX)
    };
    let start = boundary(u128::from(index));
    let end = boundary(u128::from(index) + 1);
    Ok(FrameWindow {
        index,
        start_tick: start,
        end_tick: end,
    })
}
