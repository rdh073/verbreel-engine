//! Builder — lowers a [`CompositionInput`] into a [`Graph`].
//!
//! The lowering is a pure structural transform: it mints no ids and computes
//! no hashes, so the same input always yields the same graph (the determinism
//! test target). The edge wiring encodes the composition semantics:
//!
//! - each clip becomes a [`NodeKind::Source`] node;
//! - the clip's effects chain in order — effect *i*'s parent is effect *i-1*,
//!   and the first effect's parent is the source (so an effect node has
//!   exactly one parent);
//! - the last node in a clip's chain (its source if it has no effects) is a
//!   parent of the track's [`NodeKind::Composite`] node, in clip z-order;
//! - every track composite is a parent of the single [`NodeKind::Output`]
//!   node, in track z-order (bottom-to-top).
//!
//! Build order = z-order: for each track bottom-to-top, each clip
//! earliest-first emits source-then-effects, then the track composite; the
//! output node is emitted last. That vector becomes the graph's topological
//! order — every node precedes its children because a child is always emitted
//! after the parents it consumes.

use crate::composition::{ClipInput, CompositionInput, TrackInput};
use crate::graph::{Graph, GraphNode, NodeKind};
use crate::node_id::IrNodeId;

/// Lower a composition into a deterministically-ordered DAG.
///
/// See the module docs for the edge-wiring and ordering contract. The result
/// is ready for [`crate::evaluator::Evaluator`] to key and walk.
#[must_use]
pub fn build(input: &CompositionInput) -> Graph {
    let mut nodes: Vec<GraphNode> = Vec::new();
    let mut composite_ids: Vec<IrNodeId> = Vec::with_capacity(input.tracks.len());

    for track in &input.tracks {
        lower_track(track, &mut nodes);
        composite_ids.push(track.composite_node_id);
    }

    nodes.push(GraphNode {
        id: input.output.node_id,
        kind: NodeKind::Output,
        args_hash: input.output.args_hash,
        parents: composite_ids,
    });

    Graph::from_ordered(nodes, input.output.node_id, input.tick)
}

/// Emit one track's clip subgraphs followed by its composite node.
fn lower_track(track: &TrackInput, nodes: &mut Vec<GraphNode>) {
    let mut clip_outputs: Vec<IrNodeId> = Vec::with_capacity(track.clips.len());

    for clip in &track.clips {
        let clip_output = lower_clip(clip, nodes);
        clip_outputs.push(clip_output);
    }

    nodes.push(GraphNode {
        id: track.composite_node_id,
        kind: NodeKind::Composite,
        args_hash: track.composite_args_hash,
        parents: clip_outputs,
    });
}

/// Emit one clip's source node and effect chain. Returns the id of the chain's
/// last node (the source itself when the clip has no effects), which the
/// caller wires into the track composite.
fn lower_clip(clip: &ClipInput, nodes: &mut Vec<GraphNode>) -> IrNodeId {
    nodes.push(GraphNode {
        id: clip.source_node_id,
        kind: NodeKind::Source {
            asset: clip.asset.clone(),
        },
        args_hash: clip.args_hash,
        parents: Vec::new(),
    });

    let mut prev = clip.source_node_id;
    for effect in &clip.effects {
        nodes.push(GraphNode {
            id: effect.node_id,
            kind: NodeKind::Effect,
            args_hash: effect.args_hash,
            parents: vec![prev],
        });
        prev = effect.node_id;
    }

    prev
}
