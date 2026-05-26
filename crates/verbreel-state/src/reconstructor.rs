//! Reconstructor purity scaffolding — the §0.8 startup gate (Slice A).
//!
//! ## The contract (verbatim, §0.8 `spec/commands/conventions.md`)
//!
//! > **Reconstructor purity**: reconstructors MUST be pure functions of
//! > their recorded inputs — no `crypto.randomUUID()`, no wall-clock
//! > reads, no filesystem I/O, no environment lookups. Any
//! > non-determinism produces a replay state that diverges from the
//! > original execution, which violates the Appendix C
//! > replay-determinism contract. Concretely: a verb whose `data`
//! > carries a newly-minted UUID must record that UUID in the patch (as
//! > the `id` field of an `add` op) — `randomUUID()` happens only once,
//! > during the original execution, and the patch is the
//! > record-of-truth for the value. Verbs whose `data` carries a hash, a
//! > count, a derived path, or any other function of (args, patch,
//! > post-state) are safe; verbs that would call `randomUUID()` at
//! > reconstruction time fail the startup validation. If a future
//! > verb's `data` would not be reconstructable from the recorded
//! > inputs, the verb MUST store the missing field in `warnings.details`
//! > (the only forward-compatible audit channel) at first execution;
//! > replay reads it back. The engine validates this invariant at
//! > startup by exercising every registered verb's reconstructor against
//! > a recorded event — a verb whose reconstructor cannot produce
//! > identical `data` fails `list_capabilities` and the engine refuses
//! > to start with that verb registered.
//!
//! The canonical reconstructor signature is the **5-tuple**
//! `(verb, args, patch, warnings, post-state)`:
//!
//! - `verb`: stable string id (e.g. `"project.set_metadata"`).
//! - `args`: the recorded verb args ([`serde_json::Value`]).
//! - `patch`: the RFC 6902 op list ([`serde_json::Value`]).
//! - `warnings`: the recorded warnings (`&[Value]`) — carries the
//!   `details.*` pre-state escape hatch per §0.8's closing paragraph
//!   (e.g. `clip.set_speed`'s `W_NOOP_FLAG.details.prior_speed`).
//! - `post-state`: the project graph **after** the patch has been
//!   applied during replay (`&Project`).
//!
//! The reconstructor returns the envelope `.data` field
//! ([`serde_json::Value`]) that the verb originally produced.
//!
//! ## Vacuous-pass rationale
//!
//! An empty registry validated against empty fixtures passes: zero
//! registered verbs is zero obligation. The gate is "every *registered*
//! verb's reconstructor must round-trip" — with nothing registered the
//! universally-quantified claim is vacuously true. [`validate_reconstructors`]
//! therefore returns `Ok` on `(&VerbRegistry::new(), &[])`.
//!
//! ## "Verb in registry without a fixture" boundary
//!
//! [`validate_reconstructors`] does **not** flag a verb that is present
//! in the registry but has no fixture in the supplied slice. A verb may
//! legitimately have zero, one, or many fixtures in a given validation
//! pass — enforcing "every registered verb has at least one fixture" is
//! the *test harness's* invariant, not the validator's. The validator
//! only checks the fixtures it is handed; it walks fixtures, not the
//! registry. (A fixture whose verb is *absent* from the registry is the
//! opposite case and IS a hard error — see
//! [`ValidationError::UnknownVerb`].)
//!
//! ## Slice scope
//!
//! This is **Slice A** — pure scaffolding. The validator is a
//! freestanding function; it is **not yet** wired into
//! `ProjectStore::open`. Slice B lands the first real verb
//! (`project.set_metadata`) and the `project.open` startup-gate wiring.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use thiserror::Error;

use crate::Project;

/// A per-verb implementation: owns both the §0.8 forward path
/// (`compute_patch`) and the replay path (`reconstruct`).
///
/// Implementations MUST be **pure functions** of their inputs — no
/// `randomUUID()`, no wall-clock reads, no filesystem I/O, no
/// environment lookups (§0.8 reconstructor purity). Any value that
/// cannot be derived from the recorded inputs must already live in
/// `args`, the `patch`, or `warnings.details` so replay can recover it.
///
/// `Send + Sync + 'static` because the [`VerbRegistry`] stores trait
/// objects behind [`Arc`] and the engine shares the registry across
/// threads.
///
/// ## Slice B3 rename
///
/// Promoted from the original `VerbReconstructor` (Slice A/B1/B2 name)
/// to `Verb` in Slice B3 because the trait now carries both legs of the
/// §0.8 verb contract — `compute_patch` for the forward path and
/// `reconstruct` for the replay path. The old name is kept as a
/// `#[deprecated]` alias for one slice cycle to ease downstream
/// migration.
pub trait Verb: Send + Sync + 'static {
    /// Stable verb id (e.g. `"project.set_metadata"`).
    ///
    /// Returns `&'static str` so registration is static-string-keyed —
    /// no `String` allocation at lookup time.
    fn verb(&self) -> &'static str;

    /// §0.8 forward path: given a pre-state and args, return the
    /// RFC 6902 patch, data envelope, and warnings that should be
    /// persisted on the event line.
    ///
    /// MUST be a pure function of `(prior, args)` — no I/O, no clock,
    /// no RNG. The same `(prior, args)` MUST produce a
    /// canonically-equal `(patch, data, warnings)` across runs.
    ///
    /// Each warning is an opaque [`serde_json::Value`] whose nominal
    /// schema is `{ code: string, message: string, details?: object }`.
    /// Typed warning catalogs are intentionally deferred to a later
    /// slice.
    ///
    /// # Errors
    ///
    /// Returns [`VerbError`] when:
    ///
    /// - the supplied `args` are malformed for this verb (shape
    ///   mismatch, mutually-exclusive flags, missing required fields)
    ///   → [`VerbError::BadArgs`];
    /// - the verb would violate a §0.13 engine invariant if the patch
    ///   were applied (cap overflow, structural rule break) →
    ///   [`VerbError::InvariantViolation`];
    /// - a verb-specific failure mode that does not fit the structured
    ///   variants — e.g. patch construction failed at the
    ///   `Value`-to-`json_patch::Patch` conversion boundary →
    ///   [`VerbError::Custom`].
    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError>;

    /// Reconstruct the envelope `.data` field from the recorded 5-tuple.
    ///
    /// MUST be pure — no I/O, no clock, no RNG. See the trait-level docs
    /// and §0.8.
    ///
    /// # Errors
    ///
    /// Returns [`ReconstructError`] when the recorded inputs are
    /// malformed for this verb (a verb-author bug surfaced at the
    /// startup gate) — e.g. a field the reconstructor expected is
    /// missing from `args`/`warnings`, has the wrong type, or the
    /// post-state walk fails.
    fn reconstruct(
        &self,
        args: &Value,
        patch: &Value,
        warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError>;
}

/// Deprecated alias kept for one slice cycle to ease downstream
/// migration. New code MUST use [`Verb`].
#[deprecated(since = "0.0.0", note = "use `Verb` (renamed in Slice B3)")]
pub use Verb as VerbReconstructor;

/// Errors surfaced by [`Verb::compute_patch`].
///
/// Each variant maps onto the verb-layer envelope error taxonomy:
///
/// - [`VerbError::BadArgs`] → `E_ARGS_INCOMPATIBLE` / `E_SCHEMA_VIOLATION`
///   (the args themselves are malformed: mutually-exclusive flags,
///   missing required fields, wrong shape).
/// - [`VerbError::InvariantViolation`] → `E_SCHEMA_VIOLATION` (the
///   args are well-formed but the resulting patch would violate a
///   §0.13 engine invariant — e.g. cap overflow, structural rule break).
/// - [`VerbError::Custom`] → escape hatch for verb-specific failures
///   that don't fit the structured variants (e.g. patch construction
///   failed at the `Value`-to-`json_patch::Patch` conversion).
#[derive(Debug, thiserror::Error)]
pub enum VerbError {
    /// The supplied args were malformed for this verb. Surfaces as
    /// `E_ARGS_INCOMPATIBLE` / `E_SCHEMA_VIOLATION` at the verb-layer
    /// envelope.
    #[error("verb arguments invalid: {detail}")]
    BadArgs {
        /// Human-readable description of what was wrong with the args.
        detail: String,
    },

    /// The args are well-formed but the resulting patch would violate
    /// a §0.13 engine invariant. Surfaces as `E_SCHEMA_VIOLATION`.
    #[error("§0.13 invariant would be violated: {detail}")]
    InvariantViolation {
        /// Human-readable description of which invariant would break.
        detail: String,
    },

    /// Verb-specific failure that does not fit the structured variants
    /// — e.g. patch construction failed at the `Value`-to-
    /// `json_patch::Patch` conversion boundary.
    #[error("verb-specific failure: {0}")]
    Custom(String),
}

/// Failure modes a [`Verb::reconstruct`] may report.
///
/// Every variant signals a **verb-author bug** discovered at the startup
/// gate: the recorded event does not carry what the reconstructor needs
/// to rebuild `data` deterministically. These are not runtime user
/// errors — a passing gate is a precondition for the verb to be
/// registered at all.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ReconstructError {
    /// `args` or `warnings` is missing a field the reconstructor
    /// expected. The recorded event is malformed for this verb.
    #[error("reconstructor input missing field `{name}`")]
    MissingField {
        /// The field name the reconstructor required but did not find.
        name: &'static str,
    },

    /// A field was present but had the wrong JSON type.
    #[error("reconstructor input field `{name}` has wrong type (expected {expected})")]
    TypeMismatch {
        /// The field name whose type did not match.
        name: &'static str,
        /// The JSON type the reconstructor expected (e.g. `"string"`).
        expected: &'static str,
    },

    /// The post-state walk failed — e.g. the patch was supposed to add a
    /// node the verb's `data` references, but that node's id is absent
    /// from the post-state graph.
    #[error("post-state walk failed: {detail}")]
    PostStateMissing {
        /// Human-readable description of which post-state lookup failed.
        detail: String,
    },

    /// Escape hatch for verb-specific failure descriptions that do not
    /// fit the structured variants above.
    #[error("reconstructor error: {0}")]
    Custom(String),
}

/// Errors returned when mutating a [`VerbRegistry`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RegistryError {
    /// A reconstructor for this verb id is already registered.
    /// Registration is rejected — a verb id maps to exactly one
    /// reconstructor.
    #[error("verb `{verb}` is already registered")]
    DuplicateVerb {
        /// The duplicate verb id.
        verb: &'static str,
    },
}

/// The registry of per-verb implementations.
///
/// Keyed by `&'static str` verb id so lookups allocate nothing. Stores
/// trait objects behind [`Arc`] so the engine can share the registry
/// across threads and hand out cheap clones. Registration rejects
/// duplicate verb ids ([`RegistryError::DuplicateVerb`]).
///
/// Slice B3: the registry now stores `Arc<dyn Verb>` (was `Arc<dyn
/// VerbReconstructor>`). The trait promotion means the same registry
/// instance powers both the §0.8 startup gate
/// ([`validate_reconstructors`] still calls `.reconstruct()`) and the
/// runtime forward router
/// ([`crate::lifecycle::ProjectStore::mutate_via_verb`] calls
/// `.compute_patch()`). Single source of truth per verb.
#[derive(Default, Clone)]
pub struct VerbRegistry {
    map: HashMap<&'static str, Arc<dyn Verb>>,
}

impl VerbRegistry {
    /// Construct an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a verb under its [`Verb::verb`] id.
    ///
    /// # Errors
    ///
    /// [`RegistryError::DuplicateVerb`] if a verb with the same id is
    /// already registered.
    pub fn register(&mut self, r: Arc<dyn Verb>) -> Result<(), RegistryError> {
        let verb = r.verb();
        if self.map.contains_key(verb) {
            return Err(RegistryError::DuplicateVerb { verb });
        }
        self.map.insert(verb, r);
        Ok(())
    }

    /// Look up the verb for `verb`, if registered.
    #[must_use]
    pub fn get(&self, verb: &str) -> Option<Arc<dyn Verb>> {
        self.map.get(verb).cloned()
    }

    /// All registered verb ids, sorted for deterministic output.
    #[must_use]
    pub fn verbs(&self) -> Vec<&'static str> {
        let mut verbs: Vec<&'static str> = self.map.keys().copied().collect();
        verbs.sort_unstable();
        verbs
    }

    /// Iterate over `(verb_id, verb)` pairs in unspecified order.
    ///
    /// Callers that need a deterministic order must sort by the verb
    /// id themselves; this iterator reflects the underlying
    /// [`HashMap`] order.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, Arc<dyn Verb>)> + '_ {
        self.map.iter().map(|(k, v)| (*k, Arc::clone(v)))
    }
}

/// A recorded event exercised against the startup gate.
///
/// Mirrors the load-bearing fields of an `events.jsonl` line (Appendix C)
/// — the 5-tuple `(verb, args, patch, warnings, post-state)` — plus
/// `expected_data`: what the envelope `.data` was at the original
/// execution. The gate replays the reconstructor against this tuple and
/// asserts the produced `data` matches `expected_data` byte-for-byte
/// under RFC 8785 canonicalization.
#[derive(Debug, Clone)]
pub struct RecordedEvent {
    /// The verb id (e.g. `"project.set_metadata"`). Looked up in the
    /// registry; a verb absent from the registry is a test-author bug
    /// ([`ValidationError::UnknownVerb`]).
    pub verb: String,
    /// The recorded verb args.
    pub args: Value,
    /// The recorded RFC 6902 patch.
    pub patch: Value,
    /// The recorded warnings array — carries the `details.*` pre-state
    /// escape hatch per §0.8.
    pub warnings: Vec<Value>,
    /// The project graph after the patch has been applied during replay.
    pub post_state: Project,
    /// The envelope `.data` field at original execution — the value the
    /// reconstructor must reproduce.
    pub expected_data: Value,
}

/// Errors returned by [`validate_reconstructors`].
#[derive(Debug, PartialEq, Eq, Error)]
pub enum ValidationError {
    /// A fixture's verb is not registered. Fixtures whose verbs are not
    /// in the registry are a test-author bug, not a runtime concern —
    /// the validator fails loudly rather than silently skipping.
    #[error("fixture references unregistered verb `{verb}`")]
    UnknownVerb {
        /// The unregistered verb id from the fixture.
        verb: String,
    },

    /// The reconstructor produced `data` whose canonical SHA differs
    /// from the recorded `expected_data`'s. The verb is not
    /// replay-deterministic and would fail `list_capabilities`.
    ///
    /// `expected_sha` / `produced_sha` drive the equality check;
    /// `expected_pretty` / `produced_pretty` are pretty-printed JSON for
    /// human-readable test diffs.
    #[error(
        "verb `{verb}` reconstructor produced data that does not match the recorded data\n\
         expected sha: {expected_sha}\nproduced sha: {produced_sha}\n\
         --- expected ---\n{expected_pretty}\n--- produced ---\n{produced_pretty}"
    )]
    DataMismatch {
        /// The verb whose reconstruction mismatched.
        verb: &'static str,
        /// Canonical SHA-256 (hex) of the recorded `expected_data`.
        expected_sha: String,
        /// Canonical SHA-256 (hex) of the reconstructed `data`.
        produced_sha: String,
        /// Pretty-printed recorded `expected_data` (diagnostic only).
        expected_pretty: String,
        /// Pretty-printed reconstructed `data` (diagnostic only).
        produced_pretty: String,
    },

    /// The reconstructor itself returned an error against the fixture —
    /// the recorded inputs were malformed for the verb.
    #[error("verb `{verb}` reconstructor failed: {source}")]
    ReconstructError {
        /// The verb whose reconstructor failed.
        verb: &'static str,
        /// The underlying reconstruction failure.
        #[source]
        source: ReconstructError,
    },

    /// RFC 8785 canonicalization of a `data` value failed (e.g. a
    /// non-finite number). Surfaces the underlying
    /// `verbreel_canon::CanonError` message.
    #[error("canonicalization failed: {0}")]
    Canonicalize(String),
}

/// Summary of a successful validation pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReport {
    /// Distinct verb ids exercised, sorted for deterministic output.
    pub verbs_checked: Vec<&'static str>,
    /// Number of fixtures run.
    pub fixtures_run: usize,
}

/// Validate every fixture's reconstructor against its recorded `data`.
///
/// For each fixture:
///
/// 1. Look up the verb in `registry`. If absent →
///    [`ValidationError::UnknownVerb`] (a test-author bug — fail loudly).
/// 2. Run the reconstructor over the recorded 5-tuple. If it errors →
///    [`ValidationError::ReconstructError`].
/// 3. Compare the produced `data` against the recorded `expected_data`
///    via RFC 8785 canonical SHA-256 (so JSON key-order differences do
///    not false-positive). On mismatch →
///    [`ValidationError::DataMismatch`].
///
/// On full pass, returns a [`ValidationReport`] with the distinct verbs
/// exercised (sorted) and the fixture count.
///
/// **Vacuous-pass**: an empty registry validated against empty fixtures
/// returns `Ok` (see the module-level docs). A verb present in the
/// registry but absent from `fixtures` is **not** flagged — the
/// validator walks fixtures, not the registry; the "every registered
/// verb has a fixture" invariant belongs to the test harness.
///
/// # Errors
///
/// Returns [`ValidationError`] on the first failing fixture (unknown
/// verb, reconstructor error, data mismatch, or a canonicalization
/// failure).
pub fn validate_reconstructors(
    registry: &VerbRegistry,
    fixtures: &[RecordedEvent],
) -> Result<ValidationReport, ValidationError> {
    let mut verbs_checked: Vec<&'static str> = Vec::with_capacity(fixtures.len());

    for fixture in fixtures {
        let reconstructor =
            registry
                .get(&fixture.verb)
                .ok_or_else(|| ValidationError::UnknownVerb {
                    verb: fixture.verb.clone(),
                })?;

        let produced = reconstructor
            .reconstruct(
                &fixture.args,
                &fixture.patch,
                &fixture.warnings,
                &fixture.post_state,
            )
            .map_err(|source| ValidationError::ReconstructError {
                verb: reconstructor.verb(),
                source,
            })?;

        let produced_sha = canonical_sha(&produced)?;
        let expected_sha = canonical_sha(&fixture.expected_data)?;

        if produced_sha != expected_sha {
            return Err(ValidationError::DataMismatch {
                verb: reconstructor.verb(),
                expected_sha,
                produced_sha,
                expected_pretty: pretty(&fixture.expected_data),
                produced_pretty: pretty(&produced),
            });
        }

        verbs_checked.push(reconstructor.verb());
    }

    verbs_checked.sort_unstable();
    verbs_checked.dedup();

    Ok(ValidationReport {
        verbs_checked,
        fixtures_run: fixtures.len(),
    })
}

/// RFC 8785 canonical SHA-256 (hex) of `v`, via `verbreel-canon`.
///
/// `verbreel_canon::sha256_hex` canonicalizes internally then hashes, so
/// this is a one-call wrapper that maps the canon error into a
/// [`ValidationError`].
fn canonical_sha(v: &Value) -> Result<String, ValidationError> {
    verbreel_canon::sha256_hex(v).map_err(|e| ValidationError::Canonicalize(e.to_string()))
}

/// Pretty-print a JSON value for human-readable diffs. Diagnostic only —
/// never feeds the equality check (that is the canonical SHA's job).
fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}
