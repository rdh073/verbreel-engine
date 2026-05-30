//! Regression tests for the composition graph executor (#421).
//!
//! Three targets from the issue:
//! - determinism: same input → identical graph + identical node `CacheKey`s.
//! - cache locality: editing one clip changes only the affected subgraph's
//!   keys; unrelated nodes keep their keys.
//! - stable topological order matching track/clip z-order.

use std::collections::HashSet;

use verbreel_ir::{
    CacheStatus, ClipInput, CompositionInput, EffectInput, Evaluator, NodeKind, OutputInput,
    TrackInput, build,
};
use verbreel_types::AssetHash;
use verbreel_types::id::UuidV7;

/// A stable v7 UUID built from a single discriminating byte. Deterministic so
/// fixtures are reproducible across runs (no `now()` time dependence).
fn node_id(tag: u8) -> verbreel_ir::IrNodeId {
    // Version nibble pinned to 7, variant bits to 0b10 — a valid RFC 9562 v7.
    let bytes: [u8; 16] = [
        tag, 0x11, 0x22, 0x33, 0x44, 0x55, 0x76, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xff,
    ];
    // Fix version (byte 6 high nibble = 7) and variant (byte 8 high bits = 10).
    let mut b = bytes;
    b[6] = 0x70 | (b[6] & 0x0f);
    b[8] = 0x80 | (b[8] & 0x3f);
    let uuid = uuid::Uuid::from_bytes(b);
    verbreel_ir::IrNodeId::from_uuid_v7(UuidV7::from_uuid(uuid).expect("valid v7"))
}

fn args(seed: u8) -> [u8; 32] {
    [seed; 32]
}

/// Two tracks: track 0 has clip A (source 10, effect 11) and clip B (source
/// 12); track 1 has clip C (source 13). Output is node 99.
fn fixture() -> CompositionInput {
    CompositionInput {
        tracks: vec![
            TrackInput {
                composite_node_id: node_id(20),
                composite_args_hash: args(120),
                clips: vec![
                    ClipInput {
                        source_node_id: node_id(10),
                        args_hash: args(10),
                        asset: Some(AssetHash::new("a".repeat(64)).unwrap()),
                        effects: vec![EffectInput {
                            node_id: node_id(11),
                            args_hash: args(11),
                        }],
                    },
                    ClipInput {
                        source_node_id: node_id(12),
                        args_hash: args(12),
                        asset: None,
                        effects: vec![],
                    },
                ],
            },
            TrackInput {
                composite_node_id: node_id(21),
                composite_args_hash: args(121),
                clips: vec![ClipInput {
                    source_node_id: node_id(13),
                    args_hash: args(13),
                    asset: None,
                    effects: vec![],
                }],
            },
        ],
        output: OutputInput {
            node_id: node_id(99),
            args_hash: args(99),
        },
        tick: 240_000,
    }
}

#[test]
fn determinism_same_input_identical_graph_and_keys() {
    let input = fixture();

    let g1 = build(&input);
    let g2 = build(&input);
    assert_eq!(g1, g2, "same input must yield an identical graph");

    let e1 = Evaluator::new(&g1);
    let e2 = Evaluator::new(&g2);

    for id in g1.topological_order() {
        assert_eq!(
            e1.key(*id),
            e2.key(*id),
            "node {id} must have an identical CacheKey across runs"
        );
        assert_eq!(
            e1.cache_hash(*id),
            e2.cache_hash(*id),
            "node {id} must have an identical derived cache hash across runs"
        );
    }
}

#[test]
fn cache_locality_editing_one_clip_only_busts_its_subgraph() {
    let base = fixture();
    let base_graph = build(&base);
    let base_eval = Evaluator::new(&base_graph);

    // Edit clip B's args (source node 12) on track 0 — change nothing else.
    let mut edited = fixture();
    edited.tracks[0].clips[1].args_hash = args(200);
    let edited_graph = build(&edited);
    let edited_eval = Evaluator::new(&edited_graph);

    // Affected: clip B's source (12), its track composite (20, B feeds it),
    // and the output (99, fed by composite 20). Everything else unchanged.
    let affected = [node_id(12), node_id(20), node_id(99)];
    let unaffected = [
        node_id(10), // clip A source — different clip, same track
        node_id(11), // clip A effect
        node_id(13), // track 1 clip C source — different track
        node_id(21), // track 1 composite
    ];

    for id in affected {
        assert_ne!(
            base_eval.cache_hash(id),
            edited_eval.cache_hash(id),
            "node {id} is downstream of the edited clip and must change"
        );
    }
    for id in unaffected {
        assert_eq!(
            base_eval.cache_hash(id),
            edited_eval.cache_hash(id),
            "node {id} is unrelated to the edit and must keep its key"
        );
    }
}

#[test]
fn topological_order_is_stable_and_matches_z_order() {
    let input = fixture();
    let g = build(&input);

    // Expected z-order: track 0 (bottom) first — clip A (source 10, effect 11),
    // clip B (source 12), composite 20 — then track 1 — clip C (source 13),
    // composite 21 — then the output (99) last.
    let expected = [
        node_id(10),
        node_id(11),
        node_id(12),
        node_id(20),
        node_id(13),
        node_id(21),
        node_id(99),
    ];
    assert_eq!(g.topological_order(), expected);

    // Every node appears after all its declared parents.
    let order = g.topological_order();
    let pos = |id| order.iter().position(|x| *x == id).unwrap();
    for node in g.nodes_in_order() {
        for parent in &node.parents {
            assert!(
                pos(*parent) < pos(node.id),
                "parent {parent} must precede child {} in topological order",
                node.id
            );
        }
    }

    // Stable across rebuilds.
    assert_eq!(build(&input).topological_order(), g.topological_order());
}

#[test]
fn node_kinds_and_edges_reflect_composition_structure() {
    let g = build(&fixture());

    assert!(matches!(
        g.node(node_id(10)).unwrap().kind,
        NodeKind::Source { asset: Some(_) }
    ));
    assert!(matches!(
        g.node(node_id(12)).unwrap().kind,
        NodeKind::Source { asset: None }
    ));
    assert_eq!(g.node(node_id(11)).unwrap().kind, NodeKind::Effect);
    assert_eq!(g.node(node_id(20)).unwrap().kind, NodeKind::Composite);
    assert_eq!(g.node(node_id(99)).unwrap().kind, NodeKind::Output);
    assert_eq!(g.output(), node_id(99));

    // Effect 11's only parent is source 10 (the chain head).
    assert_eq!(g.node(node_id(11)).unwrap().parents, vec![node_id(10)]);
    // Track-0 composite is fed by clip A's chain tail (effect 11) then clip B
    // (source 12), in clip z-order.
    assert_eq!(
        g.node(node_id(20)).unwrap().parents,
        vec![node_id(11), node_id(12)]
    );
    // Output is fed by both track composites in track z-order.
    assert_eq!(
        g.node(node_id(99)).unwrap().parents,
        vec![node_id(20), node_id(21)]
    );
}

#[test]
fn plan_marks_held_hashes_hit_and_rest_miss() {
    let g = build(&fixture());
    let eval = Evaluator::new(&g);

    // Hold only the two leaf source nodes that have no parents and no effects.
    let mut held: HashSet<[u8; 32]> = HashSet::new();
    held.insert(eval.cache_hash(node_id(12)).unwrap());
    held.insert(eval.cache_hash(node_id(13)).unwrap());

    let mut hits = 0;
    let mut misses = 0;
    for step in eval.plan(&held) {
        match step.status {
            CacheStatus::Hit => {
                hits += 1;
                assert!(step.node_id == node_id(12) || step.node_id == node_id(13));
            }
            CacheStatus::Miss => misses += 1,
        }
    }
    assert_eq!(hits, 2);
    assert_eq!(misses, g.len() - 2);
}
