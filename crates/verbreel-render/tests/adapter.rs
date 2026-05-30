//! IR-to-render adapter tests: deterministic frame windowing + plan lowering.
//! Pure — no GPU, runs feature-off in CI.

use verbreel_ir::{
    AssetHash, ClipInput, CompositionInput, EffectInput, Evaluator, OutputInput, TrackInput, build,
};
use verbreel_render::{RenderPlan, frame_window, tick_to_frame_index};

fn node() -> verbreel_ir::IrNodeId {
    verbreel_ir::IrNodeId::now()
}

/// A valid 64-char lowercase-hex `AssetHash` from a single repeated nibble.
fn asset(nibble: char) -> AssetHash {
    AssetHash::new(std::iter::repeat_n(nibble, 64).collect::<String>()).unwrap()
}

// --- frame windowing -----------------------------------------------------

#[test]
fn tick_zero_is_frame_zero() {
    assert_eq!(tick_to_frame_index(0, 30, 1).unwrap(), 0);
}

#[test]
fn one_frame_duration_at_30fps_is_8000_ticks() {
    // 240000 / 30 = 8000 ticks per frame.
    assert_eq!(tick_to_frame_index(7999, 30, 1).unwrap(), 0);
    assert_eq!(tick_to_frame_index(8000, 30, 1).unwrap(), 1);
    assert_eq!(tick_to_frame_index(16000, 30, 1).unwrap(), 2);
}

#[test]
fn windows_abut_with_no_gap_or_overlap() {
    // Each frame window's end must equal the next window's start.
    let mut prev_end = 0u64;
    for index in 0..10 {
        let w = frame_window(index, 30, 1).unwrap();
        assert_eq!(w.index, index);
        assert_eq!(
            w.start_tick,
            prev_end,
            "frame {index} must start where {} ended",
            index - 1
        );
        assert!(w.end_tick > w.start_tick, "window must be non-empty");
        prev_end = w.end_tick;
    }
}

#[test]
fn ntsc_2997_rate_does_not_drift() {
    // 30000/1001 fps. The 30th frame boundary must stay exact in rationals:
    // floor(30 * 240000 * 1001 / 30000) = floor(240240) = 240240.
    let w = frame_window(30, 30000, 1001).unwrap();
    assert_eq!(w.start_tick, 240_240);
}

#[test]
fn zero_rate_is_rejected() {
    assert!(tick_to_frame_index(0, 0, 1).is_err());
    assert!(tick_to_frame_index(0, 30, 0).is_err());
    assert!(frame_window(0, 0, 1).is_err());
    assert!(frame_window(0, 30, 0).is_err());
}

#[test]
fn windowing_is_overflow_safe_at_max_tick() {
    // u128 intermediate must not overflow on a huge tick.
    let f = tick_to_frame_index(u64::MAX, 60, 1).unwrap();
    assert!(f > 0);
}

// --- plan lowering -------------------------------------------------------

/// Build a two-track composition (bottom track has a source asset, top track
/// has a source + one effect) and lower it.
fn two_track_input() -> CompositionInput {
    let asset_a = asset('a');
    let asset_b = asset('b');
    CompositionInput {
        tracks: vec![
            TrackInput {
                composite_node_id: node(),
                composite_args_hash: [0u8; 32],
                clips: vec![ClipInput {
                    source_node_id: node(),
                    args_hash: [10u8; 32],
                    asset: Some(asset_a),
                    effects: vec![],
                }],
            },
            TrackInput {
                composite_node_id: node(),
                composite_args_hash: [0u8; 32],
                clips: vec![ClipInput {
                    source_node_id: node(),
                    args_hash: [20u8; 32],
                    asset: Some(asset_b),
                    effects: vec![EffectInput {
                        node_id: node(),
                        args_hash: [21u8; 32],
                    }],
                }],
            },
        ],
        output: OutputInput {
            node_id: node(),
            args_hash: [99u8; 32],
        },
        tick: 8000,
    }
}

#[test]
fn plan_has_one_layer_per_track_in_z_order() {
    let input = two_track_input();
    let graph = build(&input);
    let eval = Evaluator::new(&graph);
    let plan = RenderPlan::from_evaluator(&graph, &eval);

    assert_eq!(plan.layer_count(), 2, "one layer per track");
    // Bottom track (index 0) carries asset_a; effects on the top track do not
    // change which asset its layer resolves to.
    assert_eq!(plan.layers[0].source_asset, Some(asset('a')));
    assert_eq!(plan.layers[1].source_asset, Some(asset('b')));
    assert_eq!(plan.tick, 8000);
}

#[test]
fn plan_layer_carries_evaluator_cache_hash() {
    let input = two_track_input();
    let graph = build(&input);
    let eval = Evaluator::new(&graph);
    let plan = RenderPlan::from_evaluator(&graph, &eval);

    // Each composite layer's cache hash must match the evaluator's derived
    // hash for that composite node (the content address render caches by).
    for (i, track) in input.tracks.iter().enumerate() {
        let expected = eval.cache_hash(track.composite_node_id).unwrap();
        assert_eq!(plan.layers[i].cache_hash, expected);
    }
}

#[test]
fn plan_is_deterministic_for_same_graph() {
    let input = two_track_input();
    let graph = build(&input);
    let eval = Evaluator::new(&graph);
    let a = RenderPlan::from_evaluator(&graph, &eval);
    let b = RenderPlan::from_evaluator(&graph, &eval);
    assert_eq!(a, b, "same graph must yield the same plan");
}

#[test]
fn is_layer_live_only_on_cache_miss() {
    use verbreel_ir::CacheStatus;
    assert!(RenderPlan::is_layer_live(CacheStatus::Miss));
    assert!(!RenderPlan::is_layer_live(CacheStatus::Hit));
}
