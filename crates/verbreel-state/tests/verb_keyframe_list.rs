//! Tests for `keyframe.list` (§8.4) — twenty-ninth production verb.

use std::sync::Arc;

use serde_json::{Value, json};
use verbreel_state::verbs::keyframe_list::{compute_patch, data_envelope_from_post_state};
use verbreel_state::{
    Keyframe, KeyframeListArgs, KeyframeListData, KeyframeListError, KeyframeListVerb,
    MutateOutcome, Project, Verb, VerbError, VerbRegistry, default_fixtures, default_registry,
    validate_reconstructors,
};
use verbreel_types::Tick;

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";
const TRACK_ID: &str = "01900000-0000-7000-8000-0000000aa999";
const CLIP_ID: &str = "01900000-0000-7000-8000-0000000bb999";
const MISSING_CLIP: &str = "01900000-0000-7000-8000-0000000cc999";
const K1: &str = "01900000-0000-7000-8000-0000000d1001";
const K2: &str = "01900000-0000-7000-8000-0000000d1002";
const K3: &str = "01900000-0000-7000-8000-0000000d1003";
const K4: &str = "01900000-0000-7000-8000-0000000d1004";
const K5: &str = "01900000-0000-7000-8000-0000000d1005";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    FIXTURE_PROJECT_ID
        .parse()
        .expect("fixture project id parses")
}

fn keyframe_value(id: &str, property: &str, time_tk: i64, value: f64) -> Value {
    json!({
        "id": id,
        "property": property,
        "time_tk": time_tk,
        "value": value,
    })
}

fn keyframe(id: &str, property: &str, time_tk: i64, value: f64) -> Keyframe {
    serde_json::from_value(keyframe_value(id, property, time_tk, value))
        .expect("keyframe fixture parses")
}

fn project_with_keyframes(keyframes: &[Value]) -> Project {
    let mut prior = empty_project();
    let track = serde_json::from_value(json!({
        "id": TRACK_ID,
        "kind": "text",
        "name": "Text 1",
        "clips": [{
            "id": CLIP_ID,
            "name": "Clip 1",
            "asset_id": "00000000-0000-0000-0000-000000000000",
            "track_position_tk": 0,
            "source_in_tk": 0,
            "source_out_tk": 480_000,
            "text": {
                "content": "Hello",
                "font_family": "Arial",
                "font_size_px": 24,
            },
            "keyframes": keyframes.to_vec(),
        }],
    }))
    .expect("text track fixture parses");

    prior.tracks.push(track);
    prior.duration_tk = Tick::new(480_000);
    prior
}

fn args(property: Option<&str>) -> KeyframeListArgs {
    KeyframeListArgs {
        project_id: fixture_project_id(),
        clip: CLIP_ID.to_string(),
        property: property.map(str::to_string),
    }
}

#[test]
fn compute_patch_empty_keyframes_returns_empty_list() {
    let prior = project_with_keyframes(&[]);
    let (patch, warnings, data) = compute_patch(&prior, &args(None)).expect("happy path");

    assert_eq!(patch, json!([]));
    assert!(warnings.is_empty());
    assert!(data.keyframes.is_empty());
}

#[test]
fn compute_patch_one_keyframe_returns_singleton() {
    let prior = project_with_keyframes(&[keyframe_value(K1, "opacity", 0, 1.0)]);
    let (patch, warnings, data) = compute_patch(&prior, &args(None)).expect("happy path");

    assert_eq!(patch, json!([]));
    assert!(warnings.is_empty());
    assert_eq!(data.keyframes, vec![keyframe(K1, "opacity", 0, 1.0)]);
}

#[test]
fn compute_patch_sorting_on_property_then_time_tk() {
    let prior = project_with_keyframes(&[
        keyframe_value(K1, "transform.x", 250, 0.5),
        keyframe_value(K2, "opacity", 500, 0.8),
        keyframe_value(K3, "opacity", 100, 0.2),
    ]);

    let (_patch, _warnings, data) = compute_patch(&prior, &args(None)).expect("happy path");
    assert_eq!(
        data.keyframes,
        vec![
            keyframe(K3, "opacity", 100, 0.2),
            keyframe(K2, "opacity", 500, 0.8),
            keyframe(K1, "transform.x", 250, 0.5),
        ]
    );
}

#[test]
fn compute_patch_filter_by_property() {
    let prior = project_with_keyframes(&[
        keyframe_value(K1, "opacity", 100, 1.0),
        keyframe_value(K2, "transform.x", 200, 10.0),
        keyframe_value(K3, "opacity", 500, 0.0),
    ]);
    let args = args(Some("opacity"));

    let (_patch, _warnings, data) = compute_patch(&prior, &args).expect("happy path");
    assert_eq!(
        data.keyframes,
        vec![
            keyframe(K1, "opacity", 100, 1.0),
            keyframe(K3, "opacity", 500, 0.0)
        ]
    );
}

#[test]
fn compute_patch_filter_mismatch_property_returns_empty() {
    let prior = project_with_keyframes(&[keyframe_value(K1, "opacity", 100, 1.0)]);
    let args = args(Some("transform.y"));

    let (_patch, _warnings, data) = compute_patch(&prior, &args).expect("happy path");
    assert!(data.keyframes.is_empty());
}

#[test]
fn compute_patch_sorts_within_property_by_time_tk() {
    let prior = project_with_keyframes(&[
        keyframe_value(K1, "volume", 500, 1.0),
        keyframe_value(K2, "volume", 100, 0.2),
        keyframe_value(K3, "volume", 250, 0.5),
    ]);

    let (_patch, _warnings, data) =
        compute_patch(&prior, &args(Some("volume"))).expect("happy path");
    assert_eq!(
        data.keyframes,
        vec![
            keyframe(K2, "volume", 100, 0.2),
            keyframe(K3, "volume", 250, 0.5),
            keyframe(K1, "volume", 500, 1.0),
        ]
    );
}

#[test]
fn compute_patch_sorts_across_properties() {
    let prior = project_with_keyframes(&[
        keyframe_value(K1, "transform.x", 0, 10.0),
        keyframe_value(K2, "opacity", 0, 1.0),
        keyframe_value(K3, "opacity", 250, 0.5),
    ]);

    let (_patch, _warnings, data) = compute_patch(&prior, &args(None)).expect("happy path");
    assert_eq!(
        data.keyframes,
        vec![
            keyframe(K2, "opacity", 0, 1.0),
            keyframe(K3, "opacity", 250, 0.5),
            keyframe(K1, "transform.x", 0, 10.0),
        ]
    );
}

#[test]
fn compute_patch_bad_selector_maps_to_bad_selector() {
    let prior = project_with_keyframes(&[]);
    let err = compute_patch(
        &prior,
        &KeyframeListArgs {
            project_id: fixture_project_id(),
            clip: "not-a-uuid".to_string(),
            property: None,
        },
    )
    .expect_err("bad selector must reject");

    match err {
        KeyframeListError::BadSelector { detail } => {
            assert!(detail.contains("UUID"), "{detail}");
        }
        other => panic!("expected BadSelector, got {other:?}"),
    }
}

#[test]
fn compute_patch_missing_clip_maps_to_clip_not_found() {
    let prior = project_with_keyframes(&[]);
    let err = compute_patch(
        &prior,
        &KeyframeListArgs {
            project_id: fixture_project_id(),
            clip: MISSING_CLIP.to_string(),
            property: None,
        },
    )
    .expect_err("missing clip must reject");

    match err {
        KeyframeListError::ClipNotFound { clip_id } => assert_eq!(clip_id, MISSING_CLIP),
        other => panic!("expected ClipNotFound, got {other:?}"),
    }
}

#[test]
fn compute_patch_patch_is_always_empty() {
    let prior = project_with_keyframes(&[
        keyframe_value(K1, "opacity", 500, 1.0),
        keyframe_value(K2, "transform.x", 250, 0.5),
    ]);
    let (patch, _warnings, _data) = compute_patch(&prior, &args(None)).expect("happy path");
    assert_eq!(patch, json!([]));
    let patch2 = compute_patch(&prior, &args(Some("opacity"))).expect("happy path");
    assert_eq!(patch2.0, json!([]));
}

#[test]
fn compute_patch_warnings_always_empty() {
    let prior = project_with_keyframes(&[
        keyframe_value(K1, "opacity", 500, 1.0),
        keyframe_value(K2, "transform.x", 250, 0.5),
    ]);
    let (_patch, warnings, _data) = compute_patch(&prior, &args(None)).expect("happy path");
    assert!(warnings.is_empty());
    let (_patch, warnings, _data) =
        compute_patch(&prior, &args(Some("opacity"))).expect("happy path");
    assert!(warnings.is_empty());
}

#[test]
fn compute_patch_verb_error_mapping() {
    let prior = project_with_keyframes(&[]);
    let verb = KeyframeListVerb;

    let bad_selector = serde_json::to_value(KeyframeListArgs {
        project_id: fixture_project_id(),
        clip: "not-a-uuid".to_string(),
        property: None,
    })
    .expect("bad selector args serialize");
    let err = verb
        .compute_patch(&prior, &bad_selector)
        .expect_err("bad selector maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));

    let clip_not_found = serde_json::to_value(KeyframeListArgs {
        project_id: fixture_project_id(),
        clip: MISSING_CLIP.to_string(),
        property: None,
    })
    .expect("missing clip args serialize");
    let err = verb
        .compute_patch(&prior, &clip_not_found)
        .expect_err("missing clip maps");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn reconstruct_round_trip_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "keyframe.list")
        .expect("default_fixtures includes keyframe.list");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(KeyframeListVerb))
        .expect("register keyframe.list verb");

    let report = validate_reconstructors(&registry, &[fixture])
        .expect("keyframe.list reconstructor should pass");
    assert_eq!(report.verbs_checked, vec!["keyframe.list"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn data_envelope_from_post_state_is_sorted() {
    let post_state = project_with_keyframes(&[
        keyframe_value(K4, "transform.x", 250, 2.0),
        keyframe_value(K5, "opacity", 500, 0.7),
        keyframe_value(K1, "opacity", 100, 1.0),
    ]);
    let args = args(None);

    let data = data_envelope_from_post_state(&args, &post_state)
        .expect("reconstruction envelope should work");
    let reconstructed = serde_json::to_value(data).expect("data serializes");
    let expected = serde_json::to_value(KeyframeListData {
        keyframes: vec![
            keyframe(K1, "opacity", 100, 1.0),
            keyframe(K5, "opacity", 500, 0.7),
            keyframe(K4, "transform.x", 250, 2.0),
        ],
    })
    .expect("expected data serializes");
    assert_eq!(reconstructed, expected);
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let prior = project_with_keyframes(&[
        keyframe_value(K1, "opacity", 1_000, 1.0),
        keyframe_value(K2, "transform.x", 500, 4.0),
        keyframe_value(K3, "opacity", 250, 0.3),
    ]);

    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        prior,
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let args = json!({
        "project_id": FIXTURE_PROJECT_ID,
        "clip": CLIP_ID,
    });
    let outcome = store
        .mutate_via_verb("keyframe.list", args, None)
        .expect("keyframe.list should route");

    let MutateOutcome::NoOp { data, warnings, .. } = outcome else {
        panic!("happy path must return NoOp");
    };

    assert!(warnings.is_empty());
    let data: KeyframeListData =
        serde_json::from_value(data).expect("keyframe.list data deserializes");
    assert_eq!(data.keyframes.len(), 3);
    assert_eq!(data.keyframes[0].property.as_str(), "opacity");
    assert_eq!(data.keyframes[1].property.as_str(), "opacity");
    assert_eq!(data.keyframes[2].property.as_str(), "transform.x");
}
