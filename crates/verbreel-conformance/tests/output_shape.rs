//! Pinned-format tests for the CI-parseable PASS line.
//!
//! The PASS header is the contract surface CI matrices and release
//! gates parse with `grep` / `awk` / shell pipelines. Drift in the
//! header — punctuation changes, plurality flips, count-label
//! reorderings — silently breaks downstream tooling without breaking
//! `run()`'s exit code. These tests pin the header byte-for-byte so a
//! header rewrite has to be intentional.
//!
//! The em-dash `—` (U+2014) is deliberate, matches the Research 06
//! output spec, and is the discriminator between this binary's
//! reports and the surrounding cargo / tracing output that uses
//! plain ASCII hyphens.

use verbreel_conformance::run;

/// Helper — invoke the binary into a buffer and return the captured
/// stdout as a `String`. Identical shape to `tests/run.rs::invoke`
/// but local so the two files stay independent.
fn capture() -> String {
    let mut out: Vec<u8> = Vec::new();
    let code = run(&mut out);
    assert_eq!(code, 0, "live registry must PASS — see tests/run.rs");
    String::from_utf8(out).expect("output must be UTF-8")
}

#[test]
fn pass_header_uses_em_dash_separator() {
    let body = capture();
    let header = body.lines().next().expect("output has at least one line");
    // The em-dash discriminates the conformance line from any
    // surrounding shell / cargo output that uses plain ASCII hyphens.
    assert!(
        header.contains(" — "),
        "PASS header must use U+2014 em-dash separator, got: {header:?}"
    );
}

#[test]
fn pass_header_matches_pinned_prefix() {
    let body = capture();
    assert!(
        body.starts_with("conformance: PASS — "),
        "PASS header must match the documented prefix `conformance: PASS — `, got:\n{body}"
    );
}

#[test]
fn pass_header_advertises_verbs_then_fixtures() {
    let body = capture();
    let header = body.lines().next().unwrap();
    // The labels appear in order: "verbs," precedes "fixtures".
    let verbs_at = header.find(" verbs, ").expect("`verbs,` label missing");
    let fixtures_at = header.find(" fixtures").expect("`fixtures` label missing");
    assert!(
        verbs_at < fixtures_at,
        "header must advertise verbs before fixtures, got: {header:?}"
    );
}

#[test]
fn pass_header_count_fields_parse_as_usize() {
    let body = capture();
    let header = body.lines().next().unwrap();
    // Strip the prefix and trailing label; what's left is
    // "<N> verbs, <M>".
    let tail = header
        .strip_prefix("conformance: PASS — ")
        .and_then(|s| s.strip_suffix(" fixtures"))
        .unwrap_or_else(|| panic!("PASS header shape changed: {header:?}"));
    let (verbs_str, fixtures_str) = tail
        .split_once(" verbs, ")
        .unwrap_or_else(|| panic!("PASS header shape changed: {header:?}"));
    let _: usize = verbs_str
        .parse()
        .unwrap_or_else(|e| panic!("verb count must parse as usize: {e}; got {verbs_str:?}"));
    let _: usize = fixtures_str
        .parse()
        .unwrap_or_else(|e| panic!("fixture count must parse as usize: {e}; got {fixtures_str:?}"));
}

#[test]
fn pass_header_counts_are_consistent_with_listed_verbs() {
    let body = capture();
    let header = body.lines().next().unwrap();
    let tail = header.strip_prefix("conformance: PASS — ").unwrap();
    let (verbs_str, _) = tail.split_once(" verbs, ").unwrap();
    let advertised: usize = verbs_str.parse().unwrap();

    let listed = body
        .lines()
        .skip(1)
        .filter(|l| l.starts_with("  - "))
        .count();
    assert_eq!(
        advertised, listed,
        "advertised verb count must equal the number of bulleted verb lines"
    );
}

#[test]
fn pass_header_first_line_has_no_trailing_whitespace() {
    let body = capture();
    let header = body.lines().next().unwrap();
    assert_eq!(
        header.trim_end(),
        header,
        "PASS header must not carry trailing whitespace, got: {header:?}"
    );
}

#[test]
fn pass_header_is_single_line() {
    let body = capture();
    // The header is exactly the first line; the bullet list begins on
    // the second. A header that accidentally wraps would silently
    // break shell parsers that read only `head -1`.
    let first = body.lines().next().unwrap();
    assert!(
        !first.is_empty(),
        "PASS header line must not be empty, got body:\n{body}"
    );
    assert!(
        !first.contains('\n'),
        "PASS header must be a single line, got: {first:?}"
    );
}
