//! Tests for [`Project::apply`] — MVP pure RFC 6902 patch application.
//!
//! These tests lock the **MVP boundary**: §0.13 invariants are
//! deliberately NOT enforced at this slice, and the
//! `apply_does_not_enforce_invariants` test asserts that gap
//! explicitly. The follow-up fade-clamp slice will flip that test
//! to assert failure once the new `ApplyError::InvariantViolation`
//! variant lands.

use json_patch::Patch;
use serde_json::Value;
use verbreel_state::{ApplyError, Project};

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const THREE_TRACK_FIXTURE: &str = include_str!("fixtures/project_with_keyframes.json");

fn load_empty() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn load_three_track() -> Project {
    serde_json::from_str(THREE_TRACK_FIXTURE).expect("three-track fixture → Project")
}

fn parse_patch(s: &str) -> Patch {
    serde_json::from_str(s).unwrap_or_else(|e| panic!("patch literal {s:?} must parse: {e}"))
}

#[test]
fn apply_empty_patch_returns_same_project() {
    let p = load_empty();
    let patch = Patch(vec![]);
    let out = p.apply(&patch).expect("empty patch must succeed");
    assert_eq!(
        p, out,
        "empty patch must be identity over the typed Project"
    );
}

#[test]
fn apply_add_op_appends_marker() {
    let p = load_empty();
    assert!(p.markers.is_empty(), "fixture starts with no markers");

    let marker_json = r#"{
        "id": "01890000-0000-7000-8000-000000000aa1",
        "time_tk": 240000,
        "label": "intro"
    }"#;
    let patch_text = format!(r#"[{{"op":"add","path":"/markers/-","value":{marker_json}}}]"#);
    let patch = parse_patch(&patch_text);

    let out = p.apply(&patch).expect("add /markers/- must succeed");
    assert_eq!(out.markers.len(), 1, "marker list grew by one");
    assert_eq!(out.markers[0].label, "intro");
    // Default color from Marker::serde default — schema default.
    assert_eq!(out.markers[0].color, "#ffaa00ff");
}

#[test]
fn apply_replace_op_changes_name() {
    let p = load_empty();
    let original_name = p.name.clone();
    let patch = parse_patch(r#"[{"op":"replace","path":"/name","value":"after-replace"}]"#);

    let out = p.apply(&patch).expect("replace /name must succeed");
    assert_eq!(out.name, "after-replace");
    assert_ne!(out.name, original_name);
}

#[test]
fn apply_remove_op_drops_track() {
    // Uses the 3-track fixture (video + audio + text).
    let p = load_three_track();
    assert_eq!(p.tracks.len(), 3, "three-track fixture has 3 tracks");

    let patch = parse_patch(r#"[{"op":"remove","path":"/tracks/2"}]"#);
    let out = p.apply(&patch).expect("remove /tracks/2 must succeed");
    assert_eq!(out.tracks.len(), 2, "third track removed");

    // The remaining tracks are the video + audio (the text track was
    // index 2 in the fixture).
    let kinds: Vec<_> = out.tracks.iter().map(|t| t.kind).collect();
    assert!(
        !kinds
            .iter()
            .any(|k| matches!(k, verbreel_state::TrackKind::Text)),
        "the text track (was /tracks/2) is gone"
    );
}

#[test]
fn apply_test_op_succeeds_on_match() {
    let p = load_empty();
    let patch = parse_patch(r#"[{"op":"test","path":"/name","value":"test"}]"#);
    let out = p
        .apply(&patch)
        .expect("test op with matching value succeeds");
    assert_eq!(out, p, "test op is non-mutating");
}

#[test]
fn apply_test_op_fails_on_mismatch() {
    let p = load_empty();
    let patch = parse_patch(r#"[{"op":"test","path":"/name","value":"wrong-name"}]"#);
    let err = p
        .apply(&patch)
        .expect_err("test op with non-matching value must fail");
    assert!(
        matches!(err, ApplyError::PatchFailed { .. }),
        "test-op mismatch must surface as PatchFailed, got {err:?}"
    );
}

#[test]
fn apply_returns_new_project_not_mutated_input() {
    let p = load_empty();
    let snapshot = p.clone();

    let patch = parse_patch(r#"[{"op":"replace","path":"/name","value":"mutated"}]"#);
    let out = p.apply(&patch).expect("replace must succeed");

    // Input is unchanged.
    assert_eq!(p, snapshot, "&self must not be mutated by apply()");
    assert_ne!(p.name, out.name, "output diverges from input");
}

#[test]
fn apply_invalid_path_returns_patch_failed() {
    let p = load_empty();
    let patch = parse_patch(r#"[{"op":"replace","path":"/nonexistent/path","value":"x"}]"#);
    let err = p
        .apply(&patch)
        .expect_err("patch against nonexistent path must fail");
    assert!(
        matches!(err, ApplyError::PatchFailed { .. }),
        "invalid pointer must surface as PatchFailed, got {err:?}"
    );
}

#[test]
fn apply_type_violation_returns_type_violation() {
    // /canvas/width is a u32 in the typed Project. Writing a string
    // applies cleanly at the JSON-Patch layer, but the result
    // doesn't deserialize back into Project → TypeViolation.
    let p = load_empty();
    let patch = parse_patch(r#"[{"op":"replace","path":"/canvas/width","value":"not-a-number"}]"#);
    let err = p
        .apply(&patch)
        .expect_err("string-into-u32 patch must surface as TypeViolation");
    assert!(
        matches!(err, ApplyError::TypeViolation(_)),
        "string-into-u32 must surface as TypeViolation, got {err:?}"
    );
}

#[test]
fn apply_rejects_fade_clamp_violation() {
    // Was: MVP-boundary lock (`apply_does_not_enforce_invariants`).
    // Now: fade-clamp enforced. See PR #34 (§0.13 fade-clamp slice).
    //
    // §0.13 invariant: `fade_in_tk + fade_out_tk <= timeline_duration`
    // of the clip. The three-track fixture's video clip has
    // `source_in_tk=0`, `source_out_tk=2_400_000` (timeline duration
    // 2_400_000 ticks). Patching `fade_in_tk` to a value that exceeds
    // the clip's full timeline duration must now be rejected.
    use verbreel_state::InvariantViolation;
    let p = load_three_track();
    let patch = parse_patch(
        r#"[{"op":"replace","path":"/tracks/0/clips/0/fade_in_tk","value":9999999999}]"#,
    );
    let err = p
        .apply(&patch)
        .expect_err("fade > timeline must surface as InvariantViolation");
    match err {
        ApplyError::InvariantViolation(InvariantViolation::FadeClamp {
            fade_in_tk,
            fade_out_tk,
            timeline_duration_tk,
            ..
        }) => {
            assert_eq!(fade_in_tk.get(), 9_999_999_999);
            assert_eq!(fade_out_tk.get(), 0);
            assert_eq!(timeline_duration_tk.get(), 2_400_000);
        }
        other => panic!("expected FadeClamp variant, got {other:?}"),
    }
}

#[test]
fn apply_chain_preserves_history() {
    // Patch A: replace name. Patch B: append a marker. The
    // concatenation [A, B] should yield the same Project as
    // applying A then applying B to A's result.
    let p = load_empty();
    let a_text = r#"[{"op":"replace","path":"/name","value":"after-A"}]"#;
    let b_text = r#"[{"op":"add","path":"/markers/-","value":{
        "id":"01890000-0000-7000-8000-000000000aa2",
        "time_tk":120000,
        "label":"chained"
    }}]"#;
    let a = parse_patch(a_text);
    let b = parse_patch(b_text);

    let after_a = p.apply(&a).expect("A succeeds");
    let after_b_after_a = after_a.apply(&b).expect("B after A succeeds");

    let combined_text = r#"[
        {"op":"replace","path":"/name","value":"after-A"},
        {"op":"add","path":"/markers/-","value":{
            "id":"01890000-0000-7000-8000-000000000aa2",
            "time_tk":120000,
            "label":"chained"
        }}
    ]"#;
    let combined = parse_patch(combined_text);
    let after_combined = p.apply(&combined).expect("combined [A, B] succeeds");

    assert_eq!(
        after_b_after_a, after_combined,
        "stepwise == concatenated under apply()"
    );
}

#[test]
fn apply_ops_helper_equivalent_to_apply_patch() {
    // Smoke test for the apply_ops wrapper — same outcome as
    // apply(&Patch(ops)).
    let p = load_empty();
    let patch_text = r#"[{"op":"replace","path":"/name","value":"via-ops"}]"#;
    let patch = parse_patch(patch_text);

    let via_apply = p.apply(&patch).expect("apply(&Patch) reference impl");
    let via_apply_ops = p
        .apply_ops(&patch.0)
        .expect("apply_ops(&[PatchOperation]) wrapper");

    assert_eq!(via_apply, via_apply_ops, "wrapper must match apply()");
}

#[test]
fn type_violation_carries_serde_error_information() {
    // Sanity check that the TypeViolation variant carries enough
    // error info to be useful in logs — not just a unit variant.
    let p = load_empty();
    let patch = parse_patch(r#"[{"op":"replace","path":"/tick_rate_hz","value":48000}]"#);
    // Project requires tick_rate_hz == 240000 (spec §0.2); changing
    // it triggers the custom serde validator → TypeViolation with
    // a meaningful message.
    let err = p
        .apply(&patch)
        .expect_err("tick_rate_hz!=240000 must trip the typed validator");
    match err {
        ApplyError::TypeViolation(serde_err) => {
            let msg = serde_err.to_string();
            assert!(
                !msg.is_empty(),
                "TypeViolation error message must be non-empty for debuggability"
            );
        }
        other => panic!("expected TypeViolation, got {other:?}"),
    }
}

#[test]
fn apply_to_json_value_round_trip_is_stable() {
    // The whole point of apply() is that Project ↔ Value ↔ Project
    // is type-stable. A "noop" patch (test against current name
    // succeeds) round-trips identically through the apply() path.
    let p = load_empty();
    let patch_text = format!(r#"[{{"op":"test","path":"/name","value":"{}"}}]"#, p.name);
    let patch = parse_patch(&patch_text);
    let out = p.apply(&patch).expect("test op succeeds");

    // The two Projects must be value-equal AND their serde-Value
    // representations must be byte-equal. The latter is the §0.5.2
    // canonical-form premise (no fields lost or reordered through
    // the round trip).
    let v_in = serde_json::to_value(&p).expect("p → Value");
    let v_out = serde_json::to_value(&out).expect("out → Value");
    assert_eq!(v_in, v_out, "round-trip through apply() preserves Value");
    assert!(
        v_in.is_object(),
        "sanity: serialized Project is a JSON object"
    );
    let _ = Value::Null; // touch import; quiet unused warnings on lean builds
}
