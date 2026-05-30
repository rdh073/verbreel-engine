//! Composition graph node model + the DAG container.
//!
//! A [`Graph`] is a set of [`GraphNode`]s plus directed edges from each node
//! to its immediate parents (the nodes whose output it consumes). It is the
//! structural product of [`crate::builder::build`] and the input to
//! [`crate::evaluator::Evaluator`]; it carries no pixels and runs no work —
//! it only describes and orders.
//!
//! Edge direction convention: `parents` lists the nodes a node *depends on*.
//! A source node has no parents; the output node's parents are the per-track
//! composite nodes. Topological order (parents before children) is therefore
//! the order render must evaluate nodes in.

use std::collections::BTreeMap;

use verbreel_types::AssetHash;

use crate::composition::ArgsHash;
use crate::node_id::IrNodeId;

/// What a node computes. The render side dispatches on this; `verbreel-ir`
/// only records it so the graph is self-describing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    /// Reads a clip's media. Leaf of the graph — no parents.
    Source {
        /// Content-addressed asset read, if any.
        asset: Option<AssetHash>,
    },
    /// Applies one effect to its single parent (transform / effect node).
    Effect,
    /// Composites the clips of one track into a single track layer.
    Composite,
    /// The single sink: composites all track layers into the final frame.
    Output,
}

/// A node in the composition graph.
///
/// `args_hash` is the pre-hashed args supplied at the boundary (never computed
/// here); `parents` are the immediate dependencies whose cache keys fold into
/// this node's key during evaluation, making upstream edits bust downstream
/// cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNode {
    /// Stable identity of this node.
    pub id: IrNodeId,
    /// What this node computes.
    pub kind: NodeKind,
    /// SHA-256 of this node's canonicalized args, supplied pre-hashed.
    pub args_hash: ArgsHash,
    /// Immediate parents, in dependency order. Order is load-bearing: it is
    /// the order parent cache hashes fold into this node's [`crate::CacheKey`].
    pub parents: Vec<IrNodeId>,
}

/// A composition DAG: nodes keyed by id, plus the deterministic build order.
///
/// Nodes live in a [`BTreeMap`] keyed by [`IrNodeId`] so lookup is `O(log n)`
/// and iteration over the map is itself deterministic (by node id). The
/// separate `order` vector records the *build* order — track/clip z-order with
/// the output node last — which [`Self::topological_order`] returns. The two
/// orderings are distinct on purpose: id-order is for stable lookup, build
/// order is the z-order render walks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Graph {
    nodes: BTreeMap<IrNodeId, GraphNode>,
    order: Vec<IrNodeId>,
    output: IrNodeId,
    tick: u64,
}

impl Graph {
    /// Construct from already-ordered nodes. Internal to the builder; callers
    /// outside the crate build graphs via [`crate::builder::build`].
    pub(crate) fn from_ordered(nodes: Vec<GraphNode>, output: IrNodeId, tick: u64) -> Self {
        let order = nodes.iter().map(|n| n.id).collect();
        let nodes = nodes.into_iter().map(|n| (n.id, n)).collect();
        Self {
            nodes,
            order,
            output,
            tick,
        }
    }

    /// Look up a node by id.
    #[must_use]
    pub fn node(&self, id: IrNodeId) -> Option<&GraphNode> {
        self.nodes.get(&id)
    }

    /// Number of nodes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the graph has no nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Identity of the single output (sink) node.
    #[must_use]
    pub fn output(&self) -> IrNodeId {
        self.output
    }

    /// Engine tick this graph describes (§0.2 `TICK_RATE_HZ`).
    #[must_use]
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Deterministic topological order: every node appears after all its
    /// parents, and the order matches track/clip z-order with the output node
    /// last. This is the order render evaluates nodes in.
    #[must_use]
    pub fn topological_order(&self) -> &[IrNodeId] {
        &self.order
    }

    /// Iterator over nodes in [`topological_order`](Self::topological_order).
    ///
    /// # Panics
    ///
    /// Never in practice: `order` is built from the same node set in
    /// [`from_ordered`](Self::from_ordered), so every id in `order` is a key in
    /// `nodes`. A panic here would mean the two fell out of sync, which the
    /// constructor makes impossible.
    pub fn nodes_in_order(&self) -> impl Iterator<Item = &GraphNode> {
        self.order
            .iter()
            .map(|id| self.nodes.get(id).expect("order id missing from node map"))
    }
}
