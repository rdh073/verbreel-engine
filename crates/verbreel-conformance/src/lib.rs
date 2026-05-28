//! verbreel-conformance — Research 06 Layer 3 binary.
//!
//! Runs [`verbreel_state::validate_reconstructors`] over the workspace's
//! [`default_registry`](verbreel_state::default_registry) +
//! [`default_fixtures`](verbreel_state::default_fixtures) and emits a
//! CI-parseable PASS/FAIL line.
//!
//! ## Why this crate exists
//!
//! Per Research 06 §1, the §0.8 reconstructor-purity contract is
//! "engine startup runs a hardcoded conformance subset to catch
//! reconstructor drift." This binary is the CI-level full check — the
//! dedicated artifact every release gate calls. The validator itself
//! lives in `verbreel-state`; this crate is a thin CLI wrapper around
//! that public function plus a pinned output format so CI parsers
//! don't silently break when output drifts.
//!
//! ## Output format (pinned — CI parses this)
//!
//! PASS:
//!
//! ```text
//! conformance: PASS — N verbs, M fixtures
//!   - <verb_id>
//!   - <verb_id>
//!   ...
//! ```
//!
//! FAIL: one of four variants, mirroring
//! [`verbreel_state::ValidationError`]:
//!
//! ```text
//! conformance: FAIL — UnknownVerb on <verb>
//! conformance: FAIL — ReconstructError on <verb>
//!   source: <error message>
//! conformance: FAIL — DataMismatch on <verb>
//!   expected sha: <hex>
//!   produced sha: <hex>
//!   expected:
//! <pretty-printed JSON>
//!   produced:
//! <pretty-printed JSON>
//! conformance: FAIL — Canonicalize: <detail>
//! ```
//!
//! The leading line is the same shape for every variant
//! (`conformance: FAIL — <ErrorVariant> on <verb>` for verb-bound
//! errors, `conformance: FAIL — Canonicalize: …` for the global one)
//! so a regex pinned to `^conformance: (PASS|FAIL)` is sufficient for
//! CI gating.
//!
//! ## Architecture
//!
//! Per the crate dependency rule in `CLAUDE.md`:
//!
//! ```text
//! verbreel-conformance → verbreel-state
//! ```
//!
//! No other internal deps. The crate ships a thin `main.rs` that
//! delegates to [`run`]; integration tests call [`run`] directly with
//! `&mut Vec<u8>` so the contract is verified pure-Rust, no subprocess
//! plumbing.

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]

use std::io::Write;

use verbreel_state::{
    RecordedEvent, ValidationError, ValidationReport, VerbRegistry, default_fixtures,
    default_registry, validate_reconstructors,
};

/// Run the Layer 3 conformance check and emit a CI-parseable report
/// to `out`.
///
/// Returns the process exit code: `0` on PASS, `1` on FAIL. The binary
/// `main.rs` is a thin wrapper that calls this with a locked stdout
/// handle; integration tests pass a `&mut Vec<u8>` so they can pin the
/// output shape without subprocess plumbing.
///
/// Write failures (e.g. a broken pipe on stdout) are swallowed
/// deliberately — the exit code, not the output, is the load-bearing
/// signal for CI gating, and a panic on write would mask the
/// underlying conformance result.
pub fn run(out: &mut dyn Write) -> i32 {
    let registry = default_registry();
    let fixtures = default_fixtures();
    run_with(&registry, &fixtures, out)
}

/// Parameterised variant of [`run`] — accepts a caller-supplied
/// registry and fixture set so synthetic inputs can drive the
/// FAIL-formatting branches without mutating workspace defaults.
///
/// Returns the same exit-code contract as [`run`]: `0` on full pass,
/// `1` on any [`ValidationError`].
pub fn run_with(registry: &VerbRegistry, fixtures: &[RecordedEvent], out: &mut dyn Write) -> i32 {
    match validate_reconstructors(registry, fixtures) {
        Ok(report) => {
            write_pass(out, &report);
            0
        }
        Err(err) => {
            write_fail(out, &err);
            1
        }
    }
}

fn write_pass(out: &mut dyn Write, report: &ValidationReport) {
    let _ = writeln!(
        out,
        "conformance: PASS — {} verbs, {} fixtures",
        report.verbs_checked.len(),
        report.fixtures_run,
    );
    for v in &report.verbs_checked {
        let _ = writeln!(out, "  - {v}");
    }
}

fn write_fail(out: &mut dyn Write, err: &ValidationError) {
    match err {
        ValidationError::UnknownVerb { verb } => {
            let _ = writeln!(out, "conformance: FAIL — UnknownVerb on {verb}");
        }
        ValidationError::ReconstructError { verb, source } => {
            let _ = writeln!(out, "conformance: FAIL — ReconstructError on {verb}");
            let _ = writeln!(out, "  source: {source}");
        }
        ValidationError::DataMismatch {
            verb,
            expected_sha,
            produced_sha,
            expected_pretty,
            produced_pretty,
        } => {
            let _ = writeln!(out, "conformance: FAIL — DataMismatch on {verb}");
            let _ = writeln!(out, "  expected sha: {expected_sha}");
            let _ = writeln!(out, "  produced sha: {produced_sha}");
            let _ = writeln!(out, "  expected:");
            let _ = writeln!(out, "{expected_pretty}");
            let _ = writeln!(out, "  produced:");
            let _ = writeln!(out, "{produced_pretty}");
        }
        ValidationError::Canonicalize(detail) => {
            let _ = writeln!(out, "conformance: FAIL — Canonicalize: {detail}");
        }
    }
}
