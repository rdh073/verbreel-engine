//! Tests for `help` (§1.1) — sixty-fourth production verb (meta arc).

use std::sync::Arc;

use serde_json::json;
use verbreel_state::verbs::help::{compute_patch, data_envelope_from_post_state};
use verbreel_state::{
    HelpArgs, HelpData, HelpError, HelpVerb, MutateOutcome, Project, Verb, VerbDoc, VerbError,
    VerbRegistry, default_fixtures, default_registry, validate_reconstructors,
};

#[cfg(feature = "native")]
use verbreel_state::ProjectStore;

const EMPTY_FIXTURE: &str = include_str!("fixtures/empty_project_create.json");
const FIXTURE_PROJECT_ID: &str = "0190b8d3-15e3-7000-bd00-000000000001";

fn empty_project() -> Project {
    serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture → Project")
}

fn fixture_project_id() -> verbreel_types::ProjectId {
    empty_project().id
}

fn args_no_topic() -> HelpArgs {
    HelpArgs {
        project_id: fixture_project_id(),
        topic: None,
    }
}

fn args_with_topic(topic: &str) -> HelpArgs {
    HelpArgs {
        project_id: fixture_project_id(),
        topic: Some(topic.to_string()),
    }
}

// ------- arg deserialization -------

#[test]
fn args_deserialize_ok() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID, "topic": "clip" });
    let typed: HelpArgs = serde_json::from_value(raw).expect("ok");
    assert_eq!(typed.project_id.to_string(), FIXTURE_PROJECT_ID);
    assert_eq!(typed.topic.as_deref(), Some("clip"));
}

#[test]
fn args_missing_topic_defaults_to_none() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID });
    let typed: HelpArgs = serde_json::from_value(raw).expect("topic is optional");
    assert!(typed.topic.is_none());
}

#[test]
fn args_topic_json_null_is_none() {
    let raw = json!({ "project_id": FIXTURE_PROJECT_ID, "topic": null });
    let typed: HelpArgs = serde_json::from_value(raw).expect("null topic → None");
    assert!(typed.topic.is_none());
}

#[test]
fn args_missing_project_id_fails_through_verb() {
    let prior = empty_project();
    let verb = HelpVerb;
    let err = verb
        .compute_patch(&prior, &json!({}))
        .expect_err("missing project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

#[test]
fn args_wrong_type_fails_through_verb() {
    let prior = empty_project();
    let verb = HelpVerb;
    let err = verb
        .compute_patch(&prior, &json!({ "project_id": 12345 }))
        .expect_err("non-string project_id should map to BadArgs");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

// ------- no-topic branch -------

#[test]
fn no_topic_returns_nouns_only() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args_no_topic()).expect("happy path");
    assert!(data.nouns.is_some());
    assert!(data.verbs.is_none());
    assert!(data.verb.is_none());
}

#[test]
fn no_topic_nouns_sorted_asc() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args_no_topic()).expect("happy path");
    let nouns = data.nouns.expect("nouns populated");
    let mut sorted = nouns.clone();
    sorted.sort();
    assert_eq!(nouns, sorted);
}

#[test]
fn no_topic_nouns_contain_expected_entries() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args_no_topic()).expect("happy path");
    let nouns = data.nouns.expect("nouns populated");
    for expected in [
        "clip",
        "track",
        "asset",
        "describe",
        "help",
        "list_capabilities",
    ] {
        assert!(
            nouns.iter().any(|n| n == expected),
            "expected `{expected}` in nouns list, got {nouns:?}"
        );
    }
}

// ------- noun-prefix branch -------

#[test]
fn single_noun_topic_returns_verbs() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args_with_topic("clip")).expect("happy path");
    assert!(data.nouns.is_none());
    assert!(data.verb.is_none());
    let verbs = data.verbs.expect("verbs populated");
    assert!(!verbs.is_empty(), "clip.* should yield ≥1 verb");
    for v in &verbs {
        assert!(
            v.name == "clip" || v.name.starts_with("clip."),
            "unexpected non-clip verb `{}` in clip topic result",
            v.name
        );
    }
}

#[test]
fn verbs_sorted_asc_by_name() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args_with_topic("clip")).expect("happy path");
    let verbs = data.verbs.expect("verbs populated");
    let names: Vec<String> = verbs.iter().map(|v| v.name.clone()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
}

#[test]
fn verb_doc_shape_has_exactly_three_fields() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args_with_topic("clip")).expect("happy path");
    let first = data
        .verbs
        .as_ref()
        .and_then(|v| v.first())
        .expect("at least one clip verb");
    let value = serde_json::to_value(first).expect("VerbDoc serializes");
    let obj = value.as_object().expect("VerbDoc is a JSON object");
    assert_eq!(
        obj.keys().count(),
        3,
        "got {:?}",
        obj.keys().collect::<Vec<_>>()
    );
    assert!(obj.contains_key("name"));
    assert!(obj.contains_key("summary"));
    assert!(obj.contains_key("args_schema_id"));
}

// ------- full-verb branch -------

#[test]
fn full_verb_topic_returns_verb_only() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args_with_topic("clip.add")).expect("happy path");
    assert!(data.nouns.is_none());
    assert!(data.verbs.is_none());
    let verb = data.verb.expect("verb populated");
    assert_eq!(verb.name, "clip.add");
    assert_eq!(verb.summary, "");
    assert_eq!(verb.args_schema_id, "");
}

// ------- bare-name verb handling -------

#[test]
fn help_finds_itself() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args_with_topic("help")).expect("happy path");
    let verbs = data.verbs.expect("verbs populated");
    assert_eq!(verbs.len(), 1, "help is a single bare-name verb");
    assert_eq!(verbs[0].name, "help");
}

#[test]
fn bare_name_verb_describe() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args_with_topic("describe")).expect("happy path");
    let verbs = data.verbs.expect("verbs populated");
    assert_eq!(verbs.len(), 1);
    assert_eq!(verbs[0].name, "describe");
}

#[test]
fn bare_name_verb_list_capabilities() {
    let prior = empty_project();
    let (_, _, data) =
        compute_patch(&prior, &args_with_topic("list_capabilities")).expect("happy path");
    let verbs = data.verbs.expect("verbs populated");
    assert_eq!(verbs.len(), 1);
    assert_eq!(verbs[0].name, "list_capabilities");
}

// ------- error branches -------

#[test]
fn unknown_noun_returns_unknown_topic() {
    let prior = empty_project();
    let err =
        compute_patch(&prior, &args_with_topic("bogus")).expect_err("bogus noun → UnknownTopic");
    assert!(matches!(err, HelpError::UnknownTopic { ref topic } if topic == "bogus"));
}

#[test]
fn unknown_full_verb_returns_unknown_topic() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args_with_topic("clip.bogus"))
        .expect_err("clip.bogus → UnknownTopic");
    assert!(matches!(err, HelpError::UnknownTopic { ref topic } if topic == "clip.bogus"));
}

#[test]
fn empty_topic_string_returns_unknown_topic() {
    let prior = empty_project();
    let err = compute_patch(&prior, &args_with_topic("")).expect_err("empty topic → UnknownTopic");
    assert!(matches!(err, HelpError::UnknownTopic { ref topic } if topic.is_empty()));
}

#[test]
fn unknown_topic_maps_to_bad_args_through_verb() {
    let prior = empty_project();
    let verb = HelpVerb;
    let err = verb
        .compute_patch(
            &prior,
            &json!({ "project_id": FIXTURE_PROJECT_ID, "topic": "bogus" }),
        )
        .expect_err("unknown topic → BadArgs at verb surface");
    assert!(matches!(err, VerbError::BadArgs { .. }));
}

// ------- project-agnostic invariant -------

#[test]
fn verb_is_project_agnostic() {
    let prior_a = empty_project();
    let mut prior_b = empty_project();
    prior_b.name = "different-name".to_string();
    prior_b.duration_tk = verbreel_types::Tick::new(123_456);

    let (_, _, data_a) = compute_patch(&prior_a, &args_no_topic()).expect("a");
    let (_, _, data_b) = compute_patch(&prior_b, &args_no_topic()).expect("b");
    assert_eq!(data_a, data_b);
}

// ------- patch / warnings invariants -------

#[test]
fn patch_is_empty() {
    let prior = empty_project();
    let (patch, _, _) = compute_patch(&prior, &args_no_topic()).expect("happy path");
    assert_eq!(patch, json!([]));
}

#[test]
fn warnings_are_empty() {
    let prior = empty_project();
    let (_, warnings, _) = compute_patch(&prior, &args_no_topic()).expect("happy path");
    assert!(warnings.is_empty());
}

// ------- HelpData serialization shape -------

#[test]
fn help_data_serializes_with_exactly_one_field_no_topic() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args_no_topic()).expect("happy path");
    let value = serde_json::to_value(&data).expect("serializes");
    let obj = value.as_object().expect("object");
    assert_eq!(obj.keys().count(), 1);
    assert!(obj.contains_key("nouns"));
}

#[test]
fn help_data_serializes_with_exactly_one_field_noun_topic() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args_with_topic("clip")).expect("happy path");
    let value = serde_json::to_value(&data).expect("serializes");
    let obj = value.as_object().expect("object");
    assert_eq!(obj.keys().count(), 1);
    assert!(obj.contains_key("verbs"));
}

#[test]
fn help_data_serializes_with_exactly_one_field_full_verb_topic() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args_with_topic("clip.add")).expect("happy path");
    let value = serde_json::to_value(&data).expect("serializes");
    let obj = value.as_object().expect("object");
    assert_eq!(obj.keys().count(), 1);
    assert!(obj.contains_key("verb"));
}

// ------- reconstructor / registry -------

#[test]
fn reconstruct_byte_identical_to_compute_patch() {
    let prior = empty_project();
    let (_, _, data) = compute_patch(&prior, &args_no_topic()).expect("happy path");
    let envelope = data_envelope_from_post_state(&args_no_topic(), &prior).expect("envelope");
    let lhs = serde_json::to_vec(&data).expect("forward serializes");
    let rhs = serde_json::to_vec(&envelope).expect("rebuilt serializes");
    assert_eq!(lhs, rhs);
}

#[test]
fn reconstruct_from_default_fixture() {
    let fixture = default_fixtures()
        .into_iter()
        .find(|event| event.verb == "help")
        .expect("default_fixtures includes help");

    let mut registry = VerbRegistry::new();
    registry
        .register(Arc::new(HelpVerb))
        .expect("register help verb");

    let report =
        validate_reconstructors(&registry, &[fixture]).expect("help reconstruct from fixture");
    assert_eq!(report.verbs_checked, vec!["help"]);
    assert_eq!(report.fixtures_run, 1);
}

#[test]
fn verb_trait_surface_lookup_via_default_registry() {
    let registry = default_registry();
    let verb = registry
        .get("help")
        .expect("help registered in default_registry");

    let prior = empty_project();
    let (patch, data, warnings) = verb
        .compute_patch(&prior, &json!({ "project_id": FIXTURE_PROJECT_ID }))
        .expect("default_registry invocation succeeds");

    assert!(patch.is_empty());
    assert!(warnings.is_empty());
    let typed: HelpData = serde_json::from_value(data).expect("envelope deserializes to HelpData");
    assert!(typed.nouns.is_some());
}

#[cfg(feature = "native")]
#[test]
fn verb_routes_through_mutate_via_verb() {
    use tempfile::TempDir;

    let dir = TempDir::new().expect("tempdir");
    let mut store = ProjectStore::create_with_registry(
        dir.path(),
        empty_project(),
        &default_registry(),
        &default_fixtures(),
    )
    .expect("create_with_registry succeeds");

    let outcome = store
        .mutate_via_verb("help", json!({ "project_id": FIXTURE_PROJECT_ID }), None)
        .expect("help should route");

    let MutateOutcome::NoOp { data, warnings, .. } = outcome else {
        panic!("expected NoOp outcome from help");
    };
    assert!(warnings.is_empty());

    let data: HelpData = serde_json::from_value(data).expect("help data deserializes");
    let nouns = data.nouns.expect("nouns populated");
    assert!(nouns.iter().any(|n| n == "clip"));
}

// ------- direct VerbDoc construction (re-export sanity) -------

#[test]
fn verb_doc_struct_re_exported_via_crate_root() {
    let doc = VerbDoc {
        name: "x.y".to_string(),
        summary: String::new(),
        args_schema_id: String::new(),
    };
    let value = serde_json::to_value(&doc).expect("serializes");
    let obj = value.as_object().expect("object");
    assert_eq!(obj.keys().count(), 3);
}
