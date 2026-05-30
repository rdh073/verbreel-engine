//! Cache-aware evaluator — keys every node and decides skip vs. recompute.
//!
//! The evaluator walks the graph in [`Graph::topological_order`] and computes
//! each node's [`CacheKey`], folding the *derived* cache hashes of its parents
//! (in dependency order) into `upstream_hashes`. Because a node's key depends
//! on its parents' keys, editing one upstream node changes its derived hash,
//! which changes every descendant's key — an upstream edit busts downstream
//! cache while unrelated subgraphs keep their keys (the cache-locality target).
//!
//! `verbreel-ir` owns the keying and the skip/recompute decision; it does NOT
//! own the pixel cache. Render supplies the set of cache hashes it already
//! holds (an in-memory [`HashSet`] of prior `derive()` results); the evaluator
//! reports, per node in evaluation order, whether render can reuse the cached
//! result or must recompute. No persistent or distributed store (out of scope).

use std::collections::{HashMap, HashSet};

use crate::cache_key::CacheKey;
use crate::graph::Graph;
use crate::node_id::IrNodeId;

/// Whether render must recompute a node or may reuse a cached result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheStatus {
    /// The node's cache hash is already held — render reuses it.
    Hit,
    /// The node's cache hash is absent — render must recompute.
    Miss,
}

/// One node's evaluation step: its identity, key, derived hash, and status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalStep {
    /// Node being evaluated.
    pub node_id: IrNodeId,
    /// The cache key computed for this node (parents already folded in).
    pub key: CacheKey,
    /// `key.derive()`, the content address of this node's result.
    pub cache_hash: [u8; 32],
    /// Whether render hits or misses the supplied cache.
    pub status: CacheStatus,
}

/// Keys a graph and produces an evaluation-order plan.
///
/// Construction is pure and deterministic: same graph in, same keys out. The
/// caller then drives [`Self::plan`] (or [`Self::steps`]) against the set of
/// cache hashes it currently holds.
#[derive(Debug, Clone)]
pub struct Evaluator<'g> {
    graph: &'g Graph,
    keys: HashMap<IrNodeId, CacheKey>,
    cache_hashes: HashMap<IrNodeId, [u8; 32]>,
}

impl<'g> Evaluator<'g> {
    /// Key every node in the graph, folding parents' derived hashes into each
    /// node's `upstream_hashes` in dependency order.
    ///
    /// Single pass in topological order: a parent is always keyed before its
    /// children, so its derived hash is available when a child folds it in.
    ///
    /// # Panics
    ///
    /// Panics if a node lists a parent that does not precede it in the graph's
    /// topological order. [`crate::build`] always produces a valid topological
    /// order, so this cannot fire for builder-produced graphs; it would only
    /// trip a hand-constructed graph whose `parents` violate the ordering
    /// invariant — surfacing the broken graph loudly rather than silently
    /// hashing a zeroed parent.
    #[must_use]
    pub fn new(graph: &'g Graph) -> Self {
        let mut keys: HashMap<IrNodeId, CacheKey> = HashMap::with_capacity(graph.len());
        let mut cache_hashes: HashMap<IrNodeId, [u8; 32]> = HashMap::with_capacity(graph.len());

        for node in graph.nodes_in_order() {
            let upstream_hashes = node
                .parents
                .iter()
                .map(|p| {
                    *cache_hashes
                        .get(p)
                        .expect("parent keyed before child in topological order")
                })
                .collect();

            let key = CacheKey {
                node_id: node.id,
                args_hash: node.args_hash,
                upstream_hashes,
                tick: graph.tick(),
            };
            let hash = key.derive();

            keys.insert(node.id, key);
            cache_hashes.insert(node.id, hash);
        }

        Self {
            graph,
            keys,
            cache_hashes,
        }
    }

    /// The cache key computed for a node, if it is in the graph.
    #[must_use]
    pub fn key(&self, id: IrNodeId) -> Option<&CacheKey> {
        self.keys.get(&id)
    }

    /// The derived cache hash (`key.derive()`) for a node, if it is in the
    /// graph. This is the content address render addresses cached results by.
    #[must_use]
    pub fn cache_hash(&self, id: IrNodeId) -> Option<[u8; 32]> {
        self.cache_hashes.get(&id).copied()
    }

    /// Evaluation steps in topological order, with no cache (every node a
    /// [`CacheStatus::Miss`]). Convenience for render walks that maintain
    /// their own cache externally.
    ///
    /// # Panics
    ///
    /// Never in practice: every id in the graph's topological order was keyed
    /// in [`new`](Self::new), so the lookups always succeed.
    pub fn steps(&self) -> impl Iterator<Item = EvalStep> + '_ {
        self.graph.topological_order().iter().map(move |id| {
            let key = self.keys.get(id).expect("graph node missing a key").clone();
            EvalStep {
                node_id: *id,
                key,
                cache_hash: self.cache_hashes[id],
                status: CacheStatus::Miss,
            }
        })
    }

    /// Evaluation steps in topological order, marking each node [`CacheStatus::Hit`]
    /// when its derived cache hash is in `held` and [`CacheStatus::Miss`]
    /// otherwise. This is the iterator render walks to skip cached nodes.
    ///
    /// # Panics
    ///
    /// Never in practice: every id in the graph's topological order was keyed
    /// in [`new`](Self::new), so the lookups always succeed.
    pub fn plan<'a>(&'a self, held: &'a HashSet<[u8; 32]>) -> impl Iterator<Item = EvalStep> + 'a {
        self.graph.topological_order().iter().map(move |id| {
            let key = self.keys.get(id).expect("graph node missing a key").clone();
            let cache_hash = self.cache_hashes[id];
            let status = if held.contains(&cache_hash) {
                CacheStatus::Hit
            } else {
                CacheStatus::Miss
            };
            EvalStep {
                node_id: *id,
                key,
                cache_hash,
                status,
            }
        })
    }
}
