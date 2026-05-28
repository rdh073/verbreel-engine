//! `verbreel_conformance::run` exercised against the workspace's live
//! [`default_registry`](verbreel_state::default_registry) +
//! [`default_fixtures`](verbreel_state::default_fixtures).
//!
//! The crate's load-bearing contract: when called with the workspace's
//! default registry and fixtures, [`run`] returns `0` (PASS) and emits
//! a CI-parseable report. These tests pin both legs — exit code and
//! the report shape — so a regression in either is caught locally
//! before reaching CI.
//!
//! Tests use `&mut Vec<u8>` rather than spawning a subprocess, which
//! mirrors the verbreel-cli #339 pattern (`tests/project.rs`) and
//! keeps test runtime bounded to a single in-process call.

use verbreel_conformance::run;

/// Helper — run the binary into a buffer and return
/// `(exit_code, stdout_as_string)`. The closure-of-truth for every
/// test below.
fn invoke() -> (i32, String) {
    let mut out: Vec<u8> = Vec::new();
    let code = run(&mut out);
    let body = String::from_utf8(out).expect("output must be UTF-8");
    (code, body)
}

#[test]
fn run_returns_zero_against_live_registry() {
    let (code, body) = invoke();
    assert_eq!(
        code, 0,
        "live registry + fixtures must PASS conformance; got body:\n{body}"
    );
}

#[test]
fn run_output_starts_with_pass_marker() {
    let (_, body) = invoke();
    assert!(
        body.starts_with("conformance: PASS — "),
        "first line must be the pinned PASS prefix, got:\n{body}"
    );
}

#[test]
fn run_output_first_line_carries_both_counts() {
    let (_, body) = invoke();
    let first = body.lines().next().expect("output has at least one line");
    // Pinned tail: "<N> verbs, <M> fixtures" — both counts present
    // and labelled, so a future "verbs only" regression is caught.
    assert!(
        first.contains(" verbs, ") && first.ends_with(" fixtures"),
        "first line must carry both verb and fixture counts, got: {first:?}"
    );
}

#[test]
fn run_verb_lines_use_bullet_prefix() {
    let (_, body) = invoke();
    // Skip the header; every subsequent non-empty line is a verb
    // listing of the form "  - <verb_id>".
    let bullets: Vec<&str> = body.lines().skip(1).filter(|l| !l.is_empty()).collect();
    assert!(
        !bullets.is_empty(),
        "live registry must surface at least one verb in the bullet list"
    );
    for line in &bullets {
        assert!(
            line.starts_with("  - "),
            "verb bullet line must use `  - ` prefix, got: {line:?}"
        );
    }
}

#[test]
fn run_lists_at_least_one_verb() {
    let (_, body) = invoke();
    let bullets = body
        .lines()
        .skip(1)
        .filter(|l| l.starts_with("  - "))
        .count();
    assert!(
        bullets >= 1,
        "live registry should advertise at least one registered verb, got body:\n{body}"
    );
}

#[test]
fn run_verb_list_is_sorted_and_unique() {
    let (_, body) = invoke();
    let verbs: Vec<&str> = body
        .lines()
        .skip(1)
        .filter_map(|l| l.strip_prefix("  - "))
        .collect();

    let mut sorted = verbs.clone();
    sorted.sort_unstable();
    assert_eq!(
        verbs, sorted,
        "verbs_checked is documented as sorted; output must reflect that"
    );

    let mut deduped = sorted.clone();
    deduped.dedup();
    assert_eq!(
        sorted.len(),
        deduped.len(),
        "verbs_checked is documented as deduplicated; output has duplicates"
    );
}

#[test]
fn run_advertises_a_known_v1_floor_verb() {
    // `project.set_metadata` was the first reconstructor-purity slice
    // and has been in `default_registry()` since slice B. If it ever
    // disappears from the live registry, this test fails loudly —
    // catching either an accidental deregistration or a fixture-table
    // drift that would otherwise pass silently.
    let (_, body) = invoke();
    assert!(
        body.contains("  - project.set_metadata\n"),
        "live registry must still include the v1-floor verb `project.set_metadata`, got body:\n{body}"
    );
}

#[test]
fn run_output_is_deterministic_across_invocations() {
    let (code_a, body_a) = invoke();
    let (code_b, body_b) = invoke();
    assert_eq!(code_a, code_b, "exit code must be stable across runs");
    assert_eq!(
        body_a, body_b,
        "report body must be stable across runs (sort + dedup are deterministic)"
    );
}

#[test]
fn run_output_ends_with_newline() {
    let (_, body) = invoke();
    assert!(
        body.ends_with('\n'),
        "writeln! must terminate output with a newline, got tail: {:?}",
        body.chars().rev().take(10).collect::<String>()
    );
}

#[test]
fn run_writes_at_least_one_byte() {
    // A silent PASS would be ambiguous in CI logs — confirm `run`
    // never returns `0` with empty stdout. The PASS header alone is
    // ~30 bytes, so any non-trivial threshold catches a stdout-eating
    // regression.
    let mut out: Vec<u8> = Vec::new();
    let _ = run(&mut out);
    assert!(
        out.len() > 10,
        "run must produce a non-trivial report; got {} bytes",
        out.len()
    );
}

// --- Exhaustive verb-list assertion ---------------------------------------

#[test]
fn pass_output_mentions_every_verb_in_default_fixtures() {
    // Pin the FULL verb list — every verb in default_fixtures() must
    // appear as a bullet line. A regression where the report silently
    // dropped or duplicated a verb would fail loudly.
    let (_code, body) = invoke();
    let fixtures = verbreel_state::default_fixtures();
    let mut expected: Vec<&str> = fixtures.iter().map(|f| f.verb.as_str()).collect();
    expected.sort_unstable();
    expected.dedup();
    for verb in &expected {
        let bullet = format!("  - {verb}\n");
        assert!(
            body.contains(&bullet),
            "report must include bullet line for `{verb}`; full body:\n{body}"
        );
    }
}

// --- Synthetic-failure injection -----------------------------------------

use verbreel_conformance::run_with;
use verbreel_state::{ProjectId, RecordedEvent, VerbRegistry, synthetic_empty_project};

#[test]
fn run_with_unknown_verb_fixture_returns_1_and_writes_fail() {
    // Empty registry + a fixture for a verb that's not registered must
    // trigger ValidationError::UnknownVerb, which run_with formats as a
    // single "FAIL — UnknownVerb on <verb>" line. Exit code 1.
    let registry = VerbRegistry::default();
    let bogus = RecordedEvent {
        verb: "fake.verb".to_string(),
        args: serde_json::json!({}),
        patch: serde_json::json!([]),
        warnings: vec![],
        post_state: synthetic_empty_project(ProjectId::now()),
        expected_data: serde_json::json!(null),
    };

    let mut out: Vec<u8> = Vec::new();
    let code = run_with(&registry, std::slice::from_ref(&bogus), &mut out);

    assert_eq!(code, 1, "synthetic UnknownVerb must surface as exit 1");
    let body = String::from_utf8(out).expect("output is utf-8");
    assert!(
        body.contains("conformance: FAIL"),
        "output must start with FAIL marker, got:\n{body}"
    );
    assert!(
        body.contains("UnknownVerb"),
        "output must name the variant, got:\n{body}"
    );
    assert!(
        body.contains("fake.verb"),
        "output must echo the offending verb, got:\n{body}"
    );
}

#[test]
fn run_with_empty_inputs_passes_with_zero_counts() {
    // The vacuous-pass contract from verbreel-state: empty registry +
    // empty fixtures is a PASS with both counts at 0. run_with must
    // emit the same "PASS — 0 verbs, 0 fixtures" line and exit 0.
    let registry = VerbRegistry::default();
    let fixtures: Vec<RecordedEvent> = vec![];

    let mut out: Vec<u8> = Vec::new();
    let code = run_with(&registry, &fixtures, &mut out);

    assert_eq!(code, 0, "vacuous-pass must exit 0");
    let body = String::from_utf8(out).expect("output is utf-8");
    assert!(
        body.contains("conformance: PASS — 0 verbs, 0 fixtures"),
        "output must report zeros explicitly, got:\n{body}"
    );
}
