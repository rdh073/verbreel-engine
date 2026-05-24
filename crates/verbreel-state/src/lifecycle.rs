//! [`ProjectStore`] — `project.create` / `project.open` / `project.save`
//! lifecycle with §0.8 write-ordering.
//!
//! Phase 2 events-integration slice (#31). After this lands:
//!
//! - Projects persist to disk in `<project_root>/project.json` +
//!   `<project_root>/.verbreel/events.jsonl`.
//! - Mutations follow §0.8: compute patch → write event + fsync → apply.
//! - `project.open` replays post-save events on top of the snapshot per
//!   §2.2's 6-step load workflow.
//! - Torn-line recovery via [`EventBackend::truncate`].
//! - Atomic project.json writes via [`tempfile::NamedTempFile::persist`]
//!   + parent-dir fsync.
//!
//! ## What this module does NOT do
//!
//! `mutate()` still applies §0.8 step 3 via [`Project::apply`], which
//! enforces only **type-level** validity. §0.13 engine invariants (fade
//! clamp, track contiguity, no-overlap, `duration_tk` maintenance, etc.)
//! are deliberately NOT enforced — they land in follow-up slices, one
//! invariant family per slice, each gated by a new
//! `ApplyError::InvariantViolation::<Kind>` variant.
//!
//! Idempotency dedup (the in-memory hash → `event_id` index that
//! catches same-key retries) is also a separate slice.
//! `Event.idempotency_key` is wired structurally now but the dedup
//! behaviour lands later.
//!
//! ## Spec references
//!
//! - §0.6 — JSON Patch contract (RFC 6902).
//! - §0.8 — write-ordering protocol (steps 1-3).
//! - §2.1 — `project.create`.
//! - §2.2 — `project.open` (6-step load workflow).
//! - §2.3 — `project.save` (atomic write + `last_saved_event_id`).
//! - Appendix C — events.jsonl line shape.
//! - Appendix D — file layout.

use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use tempfile::NamedTempFile;
use tracing::warn;
use verbreel_events::{BackendError, Event, EventBackend, EventBuilder, NativeBackend};
use verbreel_types::EventId;

use crate::apply::ApplyError;
use crate::idempotency::{IdempotencyIndex, LookupOutcome};
use crate::project::Project;
use crate::reconstructor::{
    ReconstructError, RecordedEvent, ValidationError, VerbError, VerbRegistry,
    validate_reconstructors,
};

// ---------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------

/// Errors surfaced by [`ProjectStore`] operations. Covers IO, backend,
/// apply, and snapshot-validation failure modes.
#[derive(Debug, thiserror::Error)]
pub enum LifecycleError {
    /// Filesystem IO error (open, read, write, rename, fsync).
    #[error("filesystem IO failed: {0}")]
    Io(#[from] std::io::Error),

    /// Underlying [`EventBackend`] failed.
    #[error("event backend failed: {0}")]
    Backend(#[from] BackendError),

    /// `Project::apply` rejected the patch — either RFC 6902 op failed
    /// or the result violated the typed schema.
    #[error("project apply failed: {0}")]
    Apply(#[from] ApplyError),

    /// Snapshot deserialization failed (project.json is corrupt or
    /// schema-mismatched).
    #[error("snapshot corrupt: {detail}")]
    SnapshotCorrupt {
        /// Free-form detail (typically the underlying serde error).
        detail: String,
    },

    /// `events.jsonl` had a torn final line. Recoverable — the file is
    /// truncated to `offset` and the store continues. Surfaced through
    /// `tracing::warn` and (in tests) optionally via the
    /// [`ProjectStore::open_with_warnings`] entry point.
    #[error("torn event line at offset {offset}; truncated")]
    TornEventLine {
        /// Byte offset where the torn line begins.
        offset: u64,
    },

    /// `<project_root>/.verbreel/events.jsonl` is locked by another
    /// process (`fs4::flock` returned `WouldBlock`).
    #[error("project lock held by another process")]
    LockHeldByAnotherProcess {
        /// PID of the holder if discoverable. Currently always `None`
        /// — surfacing the PID requires platform-specific work.
        holder_pid: Option<i32>,
    },

    /// `Project.schema_version` does not match the engine's supported
    /// version. Surfaced before any patch replay so callers can run a
    /// migration in a follow-up slice.
    #[error("project schema_version mismatch: found {found}, expected {expected}")]
    SchemaMismatch {
        /// The version string read from project.json.
        found: String,
        /// The version this build supports.
        expected: String,
    },

    /// `<project_root>/project.json` does not exist.
    #[error("no project.json at <project_root>")]
    NoProjectJson,

    /// `create()` was called on a path that already has a project.json
    /// — refuse to overwrite per the §2.1 contract.
    #[error("project.json already exists; refusing to overwrite")]
    ProjectAlreadyExists,

    /// Mutating verb called with an `idempotency_key` whose slot is
    /// currently `in_progress` (a concurrent in-flight call holds it).
    /// Per §0.8 this maps to the verb-layer `E_BUSY` envelope with
    /// `details.idempotency_state = "in_progress"`.
    #[error("idempotency key {key:?} is in_progress; retry after the holder completes")]
    IdempotencyBusy {
        /// Key that's currently `in_progress`.
        key: String,
    },

    /// Mutating verb called with the same `idempotency_key` as a prior
    /// completed call but **different** args (different fingerprint).
    /// Per §0.8 this maps to the verb-layer `E_IDEMPOTENCY_CONFLICT`
    /// envelope.
    #[error(
        "idempotency key {key:?} reused with a different args fingerprint \
         (existing: {existing_fingerprint})"
    )]
    IdempotencyConflict {
        /// The key whose fingerprint changed between calls.
        key: String,
        /// Hex-encoded SHA-256 fingerprint of the *original* call's args.
        existing_fingerprint: String,
    },

    /// `verbreel-canon` could not canonicalize the verb args for the
    /// idempotency fingerprint computation. Surfaced as a typed error
    /// instead of a panic because well-formed args from the verb-layer
    /// will always canonicalize — hitting this means either the caller
    /// snuck a `NaN` / `±Infinity` into `args` or a malformed historical
    /// event got loaded during rebuild.
    #[error("idempotency fingerprint canonicalization failed: {0}")]
    IdempotencyCanonicalize(#[from] verbreel_canon::CanonError),

    /// §0.8 reconstructor-purity startup gate refused to let the engine
    /// open the project: a registered verb's reconstructor could not
    /// round-trip its recorded fixture. Engine-wide config failure —
    /// fires before any project IO so no project-specific state is
    /// touched on the failing path.
    #[error("reconstructor gate failed: {source}")]
    ReconstructorGateFailed {
        /// Underlying gate failure (unknown verb, reconstruct error,
        /// data SHA mismatch, or canonicalization failure).
        #[from]
        source: ValidationError,
    },

    /// [`ProjectStore::mutate_via_verb`] was called with a verb id that
    /// is not registered in the store's [`VerbRegistry`]. Surfaces at
    /// the kernel routing layer — verbs are looked up by id, and an
    /// unknown id is a caller bug (or an empty / mis-built registry).
    #[error("unknown verb: {verb_id}")]
    UnknownVerb {
        /// The verb id that failed lookup.
        verb_id: String,
    },

    /// [`ProjectStore::mutate_via_verb`] looked the verb up successfully
    /// but the verb's [`crate::reconstructor::Verb::compute_patch`]
    /// returned an error (bad args, would-violate-§0.13 invariant, or
    /// a verb-specific failure).
    #[error("verb execution failed: verb={verb_id}: {source}")]
    VerbExecutionFailed {
        /// The verb id whose `compute_patch` failed.
        verb_id: String,
        /// The underlying verb-layer error.
        #[source]
        source: VerbError,
    },

    /// [`ProjectStore::mutate_via_verb`] hit the §0.8 idempotency replay
    /// path, read the recorded event line from `events.jsonl`, and the
    /// verb's [`crate::reconstructor::Verb::reconstruct`] returned an
    /// error against the recorded 5-tuple
    /// `(args, patch, warnings, post_state)`.
    ///
    /// This is a verb-author bug surfaced at replay time: the
    /// reconstructor purity startup gate
    /// ([`validate_reconstructors`]) is supposed to catch
    /// reconstruction failures at engine start, but a verb may still
    /// fail at runtime against an event the gate's fixtures did not
    /// cover (e.g. a `details.*` field shape that no fixture
    /// exercised). The replay request is rejected rather than silently
    /// fabricating a `data` envelope.
    #[error("replay reconstruct failed for verb={verb_id} event={event_id}: {source}")]
    ReplayReconstructFailed {
        /// The event id whose reconstruction failed.
        event_id: EventId,
        /// The verb id whose reconstructor returned the error.
        verb_id: String,
        /// The underlying reconstructor failure.
        #[source]
        source: ReconstructError,
    },

    /// [`ProjectStore::mutate_via_verb`] hit the §0.8 idempotency replay
    /// path with a `Completed { event_id }` index entry, but
    /// [`EventBackend::read_by_id`] returned `None` — the event id the
    /// idempotency index pointed at is not present in `events.jsonl`.
    ///
    /// This indicates either log corruption (the recorded line was
    /// truncated away after the in-memory index recorded it) or a
    /// bookkeeping bug. Defensive: in normal operation the index is
    /// rebuilt from the log on `project.open`, so by construction the
    /// id is always present.
    #[error("replay event missing from log: {event_id}")]
    ReplayEventMissing {
        /// The event id the idempotency index pointed at.
        event_id: EventId,
    },
}

// ---------------------------------------------------------------------
// SaveInfo
// ---------------------------------------------------------------------

/// Result of a [`ProjectStore::save`] operation.
///
/// Mirrors the §2.3 return shape — callers (verb-layer) need
/// `bytes_written` for diagnostic / metrics output and `path` to
/// surface to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveInfo {
    /// Absolute path of the written `project.json`.
    pub path: PathBuf,
    /// Number of bytes written to `project.json` (the serialized
    /// `Project` JSON).
    pub bytes_written: u64,
}

// ---------------------------------------------------------------------
// MutateOutcome
// ---------------------------------------------------------------------

/// Result of a [`ProjectStore::mutate`] / [`ProjectStore::mutate_via_verb`]
/// call.
///
/// `Applied` is the first-call (or no-key) path — the patch was
/// validated, written to events.jsonl, and applied to the in-memory
/// project. `Replayed` is the §0.8 dedup path — the same key + same
/// args was already executed, no new event was written, and the
/// in-memory project is unchanged from before the call.
///
/// ## Slice B3: typed `data` envelope
///
/// Both variants carry `data` and `warnings` fields. For callers that
/// route via [`ProjectStore::mutate_via_verb`], these are the verb's
/// typed envelope and emitted warnings. For callers that use the raw
/// [`ProjectStore::mutate`] tuple API, `data` is [`Value::Null`] and
/// `warnings` is always empty — the raw API does not execute the verb
/// trait and therefore cannot emit warnings yet.
///
/// ## Replay-path semantics
///
/// On the [`MutateOutcome::Replayed`] path, the `data` field is
/// reconstructed from the on-disk event line per §0.8: the kernel
/// looks the original event up via
/// [`verbreel_events::EventBackend::read_by_id`] using the id from the
/// idempotency-index lookup and calls
/// [`crate::reconstructor::Verb::reconstruct`] against the recorded
/// 5-tuple `(args, patch, warnings, post_state)`. The duplicate call's
/// freshly-computed envelope is discarded — the recorded inputs are
/// the source of truth, not the retry's args, so a retry that lost
/// `warnings.details.*` content still surfaces the original envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutateOutcome {
    /// First call (or call without an `idempotency_key`). The patch was
    /// written to events.jsonl and applied in memory.
    Applied {
        /// Id of the event the call emitted.
        event_id: EventId,
        /// The verb's typed envelope `.data` value. [`Value::Null`] when
        /// the call came through the raw [`ProjectStore::mutate`] tuple
        /// API (which cannot infer typed data).
        data: Value,
        /// Emitted warning JSON values.
        ///
        /// For raw [`ProjectStore::mutate`] callers this is always
        /// `Vec::new()`.
        warnings: Vec<Value>,
    },
    /// Same-key + same-fingerprint replay. Nothing was written, nothing
    /// was applied. `event_id` is the id of the original first call's
    /// event — re-fetch from events.jsonl for the original args / patch.
    Replayed {
        /// Id of the event the original first call emitted.
        event_id: EventId,
        /// The verb's typed envelope `.data` value. See the
        /// `Replayed` caveat on [`MutateOutcome`].
        data: Value,
        /// Emitted warning JSON values.
        ///
        /// Replay semantics preserve the original event's warnings and
        /// append `W_REPLAY` as the final element.
        warnings: Vec<Value>,
    },
}

// ---------------------------------------------------------------------
// ProjectStore
// ---------------------------------------------------------------------

/// In-memory project handle bound to an on-disk layout
/// (`project.json` + `.verbreel/events.jsonl`). Holds an exclusive
/// cross-process lock (`fs4::flock`) on `events.jsonl` for the
/// lifetime of the store.
///
/// All four lifecycle operations ([`Self::create`], [`Self::open`],
/// [`Self::mutate`], [`Self::save`]) are documented inline; the
/// algorithms follow §2.1, §2.2, §0.8, and §2.3 respectively.
pub struct ProjectStore {
    project: Project,
    backend: Arc<NativeBackend>,
    root: PathBuf,
    /// Id of the most recently applied event. Used by [`Self::save`]
    /// to update `Project.last_saved_event_id`.
    last_applied_event_id: Option<EventId>,
    /// §0.8 idempotency dedup index. Rebuilt on [`Self::open`] from
    /// the events.jsonl scan; updated in lockstep with each
    /// [`Self::mutate`] call that carries an `idempotency_key`.
    idempotency: IdempotencyIndex,
    /// §0.8 verb registry — Slice B3.
    ///
    /// Same registry instance powers (a) the startup gate
    /// ([`validate_reconstructors`] over `.reconstruct()`) and (b) the
    /// runtime forward router ([`Self::mutate_via_verb`] over
    /// `.compute_patch()`). Single source of truth per verb.
    ///
    /// **Frozen for the lifetime of the store**: the registry is
    /// snapshotted at construction (`create_with_registry` /
    /// `open_with_registry`) and never mutated afterwards. There is no
    /// API surface to register a new verb on a live store — this
    /// matches §0.8's "engine refuses to start with a misconfigured
    /// registry" contract: the registry is per-engine-build, not
    /// per-mutation.
    ///
    /// Construction via the un-registry-aware [`Self::create`] /
    /// [`Self::open`] stores an empty registry; [`Self::mutate_via_verb`]
    /// on an empty registry returns [`LifecycleError::UnknownVerb`] for
    /// every verb id.
    verbs: Arc<VerbRegistry>,
}

impl std::fmt::Debug for ProjectStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Skip the backend — it carries a Mutex<File> which doesn't
        // implement Debug, and the file's path is already on
        // `self.root` so re-printing it adds nothing.
        f.debug_struct("ProjectStore")
            .field("root", &self.root)
            .field("project_id", &self.project.id)
            .field("project_name", &self.project.name)
            .field("last_applied_event_id", &self.last_applied_event_id)
            .field("idempotency", &self.idempotency)
            .field("verbs", &self.verbs.verbs())
            .finish_non_exhaustive()
    }
}

impl ProjectStore {
    /// Create a fresh project on disk per §2.1.
    ///
    /// Refuses to overwrite an existing `project.json` (returns
    /// [`LifecycleError::ProjectAlreadyExists`]). Creates
    /// `<root>/.verbreel/` if absent, opens the events.jsonl backend
    /// (acquires the exclusive lock), serializes `project` to
    /// `project.json` atomically.
    ///
    /// Thin wrapper around [`Self::create_with_registry`] with an empty
    /// [`VerbRegistry`] and no fixtures — the §0.8 reconstructor gate
    /// passes vacuously (zero registered verbs → zero obligations).
    /// Callers that want the gate's protection should construct via
    /// [`Self::create_with_registry`] using
    /// [`crate::default_registry`] + [`crate::default_fixtures`].
    ///
    /// # Errors
    ///
    /// - [`LifecycleError::ProjectAlreadyExists`] — `project.json` exists.
    /// - [`LifecycleError::Io`] — filesystem failure.
    /// - [`LifecycleError::Backend`] — events.jsonl couldn't be opened.
    /// - [`LifecycleError::LockHeldByAnotherProcess`] — another process
    ///   holds the events.jsonl lock.
    pub fn create(root: impl AsRef<Path>, project: Project) -> Result<Self, LifecycleError> {
        Self::create_with_registry(root, project, &VerbRegistry::new(), &[])
    }

    /// Create a fresh project on disk per §2.1 with the §0.8
    /// reconstructor-purity startup gate enabled.
    ///
    /// **Step 0** (before any IO): runs [`validate_reconstructors`]
    /// over `registry` + `fixtures`. If a fixture trips its verb's
    /// reconstructor — unknown verb, reconstruct error, or `data` SHA
    /// mismatch — returns
    /// [`LifecycleError::ReconstructorGateFailed`] WITHOUT touching the
    /// filesystem. This matches §0.8's "engine refuses to start with
    /// that verb registered" contract: the gate is engine-wide config
    /// validation, so no project-specific IO should leak through a
    /// failing gate.
    ///
    /// On gate pass, proceeds with the same body as [`Self::create`].
    ///
    /// # Errors
    ///
    /// - [`LifecycleError::ReconstructorGateFailed`] — gate refused the
    ///   registry/fixtures.
    /// - [`LifecycleError::ProjectAlreadyExists`] — `project.json` exists.
    /// - [`LifecycleError::Io`] — filesystem failure.
    /// - [`LifecycleError::Backend`] — events.jsonl couldn't be opened.
    /// - [`LifecycleError::LockHeldByAnotherProcess`] — another process
    ///   holds the events.jsonl lock.
    pub fn create_with_registry(
        root: impl AsRef<Path>,
        project: Project,
        registry: &VerbRegistry,
        fixtures: &[RecordedEvent],
    ) -> Result<Self, LifecycleError> {
        // Step 0: §0.8 reconstructor-purity gate. Runs BEFORE any IO so
        // a misconfigured registry cannot leave half-created state on
        // disk and so callers see the engine-wide config error before
        // the project-specific one.
        let report = validate_reconstructors(registry, fixtures)?;
        tracing::info!(
            verbs_checked = ?report.verbs_checked,
            fixtures_run = report.fixtures_run,
            "reconstructor gate passed"
        );

        let root = root.as_ref().to_path_buf();
        let project_json = root.join("project.json");
        if project_json.exists() {
            return Err(LifecycleError::ProjectAlreadyExists);
        }
        fs::create_dir_all(&root)?;
        let verbreel_dir = root.join(".verbreel");
        fs::create_dir_all(&verbreel_dir)?;

        let backend = open_backend(&verbreel_dir)?;

        let mut store = ProjectStore {
            project,
            backend: Arc::new(backend),
            root,
            last_applied_event_id: None,
            idempotency: IdempotencyIndex::new(),
            verbs: Arc::new(registry.clone()),
        };
        store.save()?;
        Ok(store)
    }

    /// Open a project from `<root>/project.json` per §2.2. Replays
    /// post-save events from `<root>/.verbreel/events.jsonl`.
    ///
    /// Six-step load workflow:
    /// 1. Verify project.json exists.
    /// 2. Deserialize project.json.
    /// 3. Open the events backend (acquires the cross-process lock).
    /// 4. Read events.jsonl, parse line-by-line.
    /// 5. If the final line is torn, truncate to the last valid offset
    ///    (recoverable warning via `tracing::warn!`).
    /// 6. Filter events whose id sorts strictly after
    ///    `project.last_saved_event_id` (byte-comparison on the lower-
    ///    case hyphenated `UUIDv7` string — v7 is time-sortable so byte
    ///    order = chronological order). Apply each in sequence.
    ///
    /// §0.13 reconciliation passes, §0.14 dense-defaults normalization,
    /// and asset integrity checks are deliberately deferred to
    /// follow-up slices.
    ///
    /// # Errors
    ///
    /// - [`LifecycleError::NoProjectJson`] — no `project.json` at path.
    /// - [`LifecycleError::SnapshotCorrupt`] — project.json doesn't
    ///   deserialize.
    /// - [`LifecycleError::Backend`] — backend open / read failed.
    /// - [`LifecycleError::LockHeldByAnotherProcess`] — events.jsonl
    ///   lock contention.
    /// - [`LifecycleError::Apply`] — a replayed patch failed (either
    ///   RFC 6902 op error or `TypeViolation`).
    pub fn open(root: impl AsRef<Path>) -> Result<Self, LifecycleError> {
        Self::open_with_registry(root, &VerbRegistry::new(), &[])
    }

    /// Open a project from `<root>/project.json` per §2.2 with the §0.8
    /// reconstructor-purity startup gate enabled.
    ///
    /// **Step 0** (before any IO): runs [`validate_reconstructors`]
    /// over `registry` + `fixtures`. Same contract as
    /// [`Self::create_with_registry`] — gate fires before the
    /// `project.json` existence check, so a misconfigured registry
    /// returns [`LifecycleError::ReconstructorGateFailed`] regardless
    /// of whether the path holds a valid project. This makes the
    /// engine-wide config error distinguishable from a missing-project
    /// error.
    ///
    /// On gate pass, proceeds with the same body as [`Self::open`].
    ///
    /// # Errors
    ///
    /// - [`LifecycleError::ReconstructorGateFailed`] — gate refused the
    ///   registry/fixtures.
    /// - [`LifecycleError::NoProjectJson`] — no `project.json` at path.
    /// - [`LifecycleError::SnapshotCorrupt`] — project.json doesn't
    ///   deserialize.
    /// - [`LifecycleError::Backend`] — backend open / read failed.
    /// - [`LifecycleError::LockHeldByAnotherProcess`] — events.jsonl
    ///   lock contention.
    /// - [`LifecycleError::Apply`] — a replayed patch failed.
    pub fn open_with_registry(
        root: impl AsRef<Path>,
        registry: &VerbRegistry,
        fixtures: &[RecordedEvent],
    ) -> Result<Self, LifecycleError> {
        // Step 0: §0.8 reconstructor-purity gate. Runs BEFORE the
        // project.json existence check so the engine-wide config error
        // ([`LifecycleError::ReconstructorGateFailed`]) is
        // distinguishable from [`LifecycleError::NoProjectJson`].
        let report = validate_reconstructors(registry, fixtures)?;
        tracing::info!(
            verbs_checked = ?report.verbs_checked,
            fixtures_run = report.fixtures_run,
            "reconstructor gate passed"
        );

        let root = root.as_ref().to_path_buf();
        let project_json = root.join("project.json");
        if !project_json.exists() {
            return Err(LifecycleError::NoProjectJson);
        }
        let bytes = fs::read(&project_json)?;
        let mut project: Project =
            serde_json::from_slice(&bytes).map_err(|e| LifecycleError::SnapshotCorrupt {
                detail: e.to_string(),
            })?;

        let verbreel_dir = root.join(".verbreel");
        fs::create_dir_all(&verbreel_dir)?;
        let backend = open_backend(&verbreel_dir)?;

        // Parse events. Torn-line recovery returns the list of valid
        // events; the backend is truncated to the last good offset
        // before this call returns.
        let events = parse_and_repair_log(&backend)?;

        let last_saved = project.last_saved_event_id;
        let mut last_applied_event_id = last_saved;
        for ev in &events {
            // §0.3 says UUIDv7 strings are time-sortable byte-wise.
            // Filter events strictly newer than the snapshot's
            // last_saved_event_id.
            let keep = match last_saved {
                Some(ref baseline) => ev.id.to_string().as_str() > baseline.to_string().as_str(),
                None => true,
            };
            if !keep {
                continue;
            }
            project = project.apply(&ev.patch)?;
            last_applied_event_id = Some(ev.id);
        }

        // Rebuild the §0.8 idempotency index from the full events
        // history (not just the post-snapshot tail): same-key retries
        // that crossed a save still need to dedup. Last write wins —
        // matches the events.jsonl "source of truth" contract.
        let idempotency = IdempotencyIndex::new();
        idempotency.rebuild_from_events(&events);

        Ok(ProjectStore {
            project,
            backend: Arc::new(backend),
            root,
            last_applied_event_id,
            idempotency,
            verbs: Arc::new(registry.clone()),
        })
    }

    /// §0.8 write-ordering: validate patch → fsync event → apply.
    /// Optionally routes the call through the in-memory dedup index
    /// when `idempotency_key` is `Some`.
    ///
    /// Algorithm when `idempotency_key` is `None` (legacy / un-keyed
    /// path — behaviour unchanged from PR #32):
    /// 1. Validate by `self.project.clone().apply(&patch)` — surfaces
    ///    any [`ApplyError`] without touching the real in-memory state.
    /// 2. Build the [`Event`] (id = `EventId::now()`, ts = now), serialize
    ///    via `serde_json::to_string` (single line — verified by a
    ///    dedicated test), and `backend.append` — which fsyncs.
    /// 3. Apply for real to `self.project`. If this fails (shouldn't
    ///    after step 1 succeeded — `apply` is pure deterministic), the
    ///    event is already on disk and the next `open()` will replay
    ///    it cleanly. This is the §0.8 contract working as designed.
    /// 4. Update `last_applied_event_id` so the next `save()` can
    ///    refresh `Project.last_saved_event_id`.
    /// 5. Return [`MutateOutcome::Applied`].
    ///
    /// Algorithm when `idempotency_key` is `Some`:
    /// 0a. Compute `fingerprint = sha256_hex(canonicalize(args))` via
    ///     [`verbreel_canon`].
    /// 0b. Call [`IdempotencyIndex::lookup`]:
    ///     - [`LookupOutcome::Absent`] / [`LookupOutcome::Expired`]:
    ///       reserve the slot with [`IdempotencyIndex::start`] then
    ///       run the un-keyed algorithm. On success, transition the
    ///       slot to [`crate::idempotency::EntryState::Completed`] via
    ///       [`IdempotencyIndex::complete`] and tag the event line with
    ///       the key. On failure, release the slot with
    ///       [`IdempotencyIndex::abort`].
    ///     - [`LookupOutcome::InProgress`]: return
    ///       [`LifecycleError::IdempotencyBusy`].
    ///     - [`LookupOutcome::Completed { event_id }`]: return
    ///       [`MutateOutcome::Replayed { event_id }`] — no event
    ///       written, no patch applied.
    ///     - [`LookupOutcome::ConflictingFingerprint`]: return
    ///       [`LifecycleError::IdempotencyConflict`].
    ///
    /// # Errors
    ///
    /// - [`LifecycleError::Apply`] — patch validation or replay failed.
    /// - [`LifecycleError::Io`] — event serialization or append failed.
    /// - [`LifecycleError::Backend`] — backend fsync failed.
    /// - [`LifecycleError::IdempotencyBusy`] — keyed call collided with
    ///   an in-flight first call.
    /// - [`LifecycleError::IdempotencyConflict`] — keyed call reused a
    ///   key with a different args fingerprint.
    /// - [`LifecycleError::IdempotencyCanonicalize`] — fingerprint
    ///   computation failed (caller-supplied args contained
    ///   `NaN`/`±Infinity` or otherwise broke canonical JSON rules).
    pub fn mutate(
        &mut self,
        verb: &str,
        args: Value,
        patch: &json_patch::Patch,
        idempotency_key: Option<String>,
    ) -> Result<MutateOutcome, LifecycleError> {
        self.mutate_with_warnings(verb, args, patch, idempotency_key, Vec::new())
    }

    fn mutate_with_warnings(
        &mut self,
        verb: &str,
        args: Value,
        patch: &json_patch::Patch,
        idempotency_key: Option<String>,
        warnings: Vec<Value>,
    ) -> Result<MutateOutcome, LifecycleError> {
        // Un-keyed path — preserves the PR #32 contract. The raw API
        // cannot infer typed data, so `data` is filled with
        // [`Value::Null`]; callers that need a typed envelope must
        // route via [`Self::mutate_via_verb`].
        let Some(key) = idempotency_key else {
            let event_id = self.apply_write_ordering(verb, args, patch, None, warnings.clone())?;
            return Ok(MutateOutcome::Applied {
                event_id,
                data: Value::Null,
                warnings,
            });
        };

        // Keyed path — fingerprint then dispatch via the index.
        let fingerprint = verbreel_canon::sha256_hex(&args)?;
        match self.idempotency.lookup(&key, &fingerprint) {
            LookupOutcome::Completed { event_id } => Ok(MutateOutcome::Replayed {
                event_id,
                data: Value::Null,
                warnings: Vec::new(),
            }),
            LookupOutcome::InProgress => Err(LifecycleError::IdempotencyBusy { key }),
            LookupOutcome::ConflictingFingerprint {
                existing_fingerprint,
            } => Err(LifecycleError::IdempotencyConflict {
                key,
                existing_fingerprint,
            }),
            LookupOutcome::Absent | LookupOutcome::Expired => {
                // First-call (or aged-out) path — reserve the slot,
                // run write-ordering, transition to Completed on
                // success, abort the reservation on failure.
                //
                // start() can still return AlreadyExists if a
                // concurrent caller raced us between the lookup and
                // the insert — convert it to the typed error variant
                // that matches its current state.
                if let Err(err) = self.idempotency.start(key.clone(), fingerprint) {
                    match err.existing_state {
                        crate::idempotency::EntryState::InProgress => {
                            return Err(LifecycleError::IdempotencyBusy { key: err.key });
                        }
                        crate::idempotency::EntryState::Completed { event_id } => {
                            // Lookup said Absent/Expired but start()
                            // saw a Completed — only possible if
                            // another caller raced in. Treat as a
                            // successful replay to keep the §0.8
                            // contract consistent.
                            return Ok(MutateOutcome::Replayed {
                                event_id,
                                data: Value::Null,
                                warnings: Vec::new(),
                            });
                        }
                    }
                }

                let event_warnings = warnings;
                match self.apply_write_ordering(
                    verb,
                    args,
                    patch,
                    Some(key.clone()),
                    event_warnings.clone(),
                ) {
                    Ok(event_id) => {
                        self.idempotency.complete(&key, event_id);
                        Ok(MutateOutcome::Applied {
                            event_id,
                            data: Value::Null,
                            warnings: event_warnings,
                        })
                    }
                    Err(e) => {
                        self.idempotency.abort(&key);
                        Err(e)
                    }
                }
            }
        }
    }

    /// Kernel-side verb routing (§0.8 forward path) — Slice B3.
    ///
    /// Given a verb id, raw JSON args, and an optional idempotency key,
    /// this method:
    ///
    /// 1. Looks up `verb_id` in the store's [`VerbRegistry`]. Unknown
    ///    id → [`LifecycleError::UnknownVerb`].
    /// 2. Calls the verb's [`crate::reconstructor::Verb::compute_patch`]
    ///    against the in-memory project + args. Failure →
    ///    [`LifecycleError::VerbExecutionFailed`] with the underlying
    ///    [`VerbError`] as `#[source]`.
    /// 3. Delegates to [`Self::mutate`] for the §0.8 write-ordering
    ///    (validate → fsync event → apply) and idempotency dedup,
    ///    threading the verb-computed `data` envelope through the
    ///    returned [`MutateOutcome`].
    ///
    /// ## When to use this vs. the raw [`Self::mutate`] API
    ///
    /// Prefer `mutate_via_verb` whenever the caller has a verb id —
    /// the verb-trait routing is the §0.8 canonical forward path and
    /// the only way to receive a typed `data` envelope on the
    /// `MutateOutcome`. The raw [`Self::mutate`] tuple API is kept for
    /// callers that pre-compute their own patch (kernel internals,
    /// migration tooling); it is **purely additive** to keep
    /// `mutate_via_verb` slim.
    ///
    /// ## Replay path semantics
    ///
    /// When this call hits the §0.8 idempotency replay path
    /// (`MutateOutcome::Replayed`), the returned `data` is reconstructed
    /// from the **on-disk** event line — not the duplicate call's
    /// freshly-computed envelope. The kernel:
    ///
    /// 1. Takes the `event_id` returned by the idempotency-index
    ///    lookup.
    /// 2. Reads the recorded event via
    ///    [`verbreel_events::EventBackend::read_by_id`].
    /// 3. Calls
    ///    [`crate::reconstructor::Verb::reconstruct`] over the recorded
    ///    5-tuple `(args, patch, warnings, post_state)` per §0.8.
    ///
    /// `post_state` passed to `reconstruct` is `&self.project` — the
    /// current in-memory project. By the time replay fires, the
    /// original first call's patch has already been applied (that
    /// produced the same idempotency-index `Completed { event_id }`
    /// entry the lookup just returned), so the in-memory project is
    /// the post-state for that patch. The replay does **not** re-apply
    /// the patch and does **not** write a new event line.
    ///
    /// Two failure modes are surfaced on this path beyond the
    /// `Self::mutate` set:
    ///
    /// - [`LifecycleError::ReplayReconstructFailed`] — the verb's
    ///   `reconstruct` returned an error against the recorded
    ///   5-tuple. Verb-author bug; the §0.8 startup gate
    ///   ([`crate::reconstructor::validate_reconstructors`]) should
    ///   catch the common cases but a verb may still fail on an
    ///   event shape no fixture exercised.
    /// - [`LifecycleError::ReplayEventMissing`] — defensive: the
    ///   idempotency index pointed at an event id absent from the
    ///   log. Should not happen in normal operation because the
    ///   index is rebuilt from the same log on `project.open`.
    ///
    /// # Errors
    ///
    /// - [`LifecycleError::UnknownVerb`] — verb id not in registry.
    /// - [`LifecycleError::VerbExecutionFailed`] — verb's
    ///   `compute_patch` rejected the args (bad shape, cap violation,
    ///   etc.).
    /// - [`LifecycleError::ReplayReconstructFailed`] — replay path:
    ///   the verb's `reconstruct` returned an error.
    /// - [`LifecycleError::ReplayEventMissing`] — replay path: the
    ///   recorded event id is not present in the log.
    /// - Everything [`Self::mutate`] can fail with — apply, IO,
    ///   backend, idempotency-busy / -conflict / -canonicalize.
    pub fn mutate_via_verb(
        &mut self,
        verb_id: &str,
        args: Value,
        idempotency_key: Option<String>,
    ) -> Result<MutateOutcome, LifecycleError> {
        // Step A: look up the verb.
        let verb = self
            .verbs
            .get(verb_id)
            .ok_or_else(|| LifecycleError::UnknownVerb {
                verb_id: verb_id.to_string(),
            })?;

        // Step B: compute the patch + data envelope from the verb.
        let (patch, data, warnings) =
            verb.compute_patch(&self.project, &args).map_err(|source| {
                LifecycleError::VerbExecutionFailed {
                    verb_id: verb_id.to_string(),
                    source,
                }
            })?;

        // Step C: delegate to the existing raw mutate() for §0.8
        // write-ordering + idempotency. Then thread the typed `data`
        // through the returned outcome.
        match self.mutate_with_warnings(verb_id, args, &patch, idempotency_key, warnings)? {
            MutateOutcome::Applied {
                event_id, warnings, ..
            } => Ok(MutateOutcome::Applied {
                event_id,
                data,
                warnings,
            }),
            MutateOutcome::Replayed {
                event_id,
                warnings: _,
                ..
            } => {
                // §0.8 replay: drop the freshly-computed `data` and
                // reconstruct from the on-disk event line per the
                // method's "Replay path semantics" rustdoc.
                let recorded = self
                    .backend
                    .read_by_id(&event_id)?
                    .ok_or(LifecycleError::ReplayEventMissing { event_id })?;

                // `Verb::reconstruct` takes `patch: &Value` (the wire
                // shape) — convert the recorded `json_patch::Patch`.
                let patch_value = serde_json::to_value(&recorded.patch).map_err(|e| {
                    LifecycleError::ReplayReconstructFailed {
                        event_id,
                        verb_id: verb_id.to_string(),
                        source: ReconstructError::Custom(format!(
                            "recorded patch could not be re-serialized to Value: {e}"
                        )),
                    }
                })?;

                let mut warnings = recorded.warnings.clone();
                warnings.push(json!({
                    "code": "W_REPLAY",
                    "message": "idempotent replay"
                }));

                let data = verb
                    .reconstruct(
                        &recorded.args,
                        &patch_value,
                        &recorded.warnings,
                        &self.project,
                    )
                    .map_err(|source| LifecycleError::ReplayReconstructFailed {
                        event_id,
                        verb_id: verb_id.to_string(),
                        source,
                    })?;

                Ok(MutateOutcome::Replayed {
                    event_id,
                    data,
                    warnings,
                })
            }
        }
    }

    /// Steps 1-3 of §0.8 write-ordering. Shared between the keyed and
    /// un-keyed [`Self::mutate`] paths. Returns the event id on
    /// success.
    fn apply_write_ordering(
        &mut self,
        verb: &str,
        args: Value,
        patch: &json_patch::Patch,
        idempotency_key: Option<String>,
        warnings: Vec<Value>,
    ) -> Result<EventId, LifecycleError> {
        // Step 1: validate without touching real state.
        let _candidate = self.project.apply(patch)?;

        // Step 2: build and durably write the event.
        let mut event = EventBuilder::new()
            .verb(verb)
            .args(args)
            .patch(patch.clone())
            .warnings(warnings);
        if let Some(idempotency_key) = idempotency_key {
            event = event.idempotency_key(idempotency_key);
        }
        let event = event.build();
        let event_id = event.id;
        let line = serde_json::to_string(&event).map_err(|e| {
            LifecycleError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        })?;
        self.backend.append(line.as_bytes())?;

        // Step 3: apply for real.
        self.project = self.project.apply(patch)?;
        self.last_applied_event_id = Some(event_id);

        Ok(event_id)
    }

    /// §2.3 `project.save`: atomic write of in-memory project to
    /// `<root>/project.json`. Bumps `Project.last_saved_event_id` to
    /// the most recent applied event id.
    ///
    /// Atomicity protocol:
    /// 1. Update `self.project.last_saved_event_id` ←
    ///    `self.last_applied_event_id` (no-op when zero mutations).
    /// 2. Serialize `&self.project` to bytes.
    /// 3. `NamedTempFile::new_in(<root>)` → write bytes → `sync_data`.
    /// 4. `tempfile.persist(<root>/project.json)` — POSIX rename is
    ///    atomic when source + dest are on the same filesystem (which
    ///    they always are here because we used `new_in(<root>)`).
    /// 5. `fsync(<root>)` so the rename itself is durable.
    /// 6. Return [`SaveInfo`].
    ///
    /// # Errors
    ///
    /// - [`LifecycleError::Io`] — any IO step failed.
    pub fn save(&mut self) -> Result<SaveInfo, LifecycleError> {
        if let Some(id) = self.last_applied_event_id {
            self.project.last_saved_event_id = Some(id);
        }

        let bytes = serde_json::to_vec_pretty(&self.project).map_err(|e| {
            LifecycleError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        })?;
        let bytes_written = bytes.len() as u64;

        let mut tmp = NamedTempFile::new_in(&self.root)?;
        tmp.as_file_mut().write_all(&bytes)?;
        tmp.as_file_mut().sync_data()?;
        let target = self.root.join("project.json");
        tmp.persist(&target)
            .map_err(|e| LifecycleError::Io(e.error))?;

        // Parent-dir fsync — POSIX guarantee that the rename is durable.
        let dir = File::open(&self.root)?;
        dir.sync_data()?;

        Ok(SaveInfo {
            path: target,
            bytes_written,
        })
    }

    /// Reference to the in-memory project.
    #[must_use]
    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Reference to the on-disk root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Most recent applied event id (or `None` if zero mutations since
    /// last save).
    #[must_use]
    pub fn last_applied_event_id(&self) -> Option<EventId> {
        self.last_applied_event_id
    }

    /// Read-only handle on the §0.8 idempotency dedup index. Exposed
    /// so callers can drive [`IdempotencyIndex::evict_expired`] on a
    /// schedule and inspect the index in tests; mutation of the index
    /// happens transparently inside [`Self::mutate`].
    #[must_use]
    pub fn idempotency(&self) -> &IdempotencyIndex {
        &self.idempotency
    }
}

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

/// Open `<verbreel_dir>/events.jsonl` via [`NativeBackend`]. Maps the
/// upstream `Locked` variant to our typed [`LifecycleError::LockHeldByAnotherProcess`].
fn open_backend(verbreel_dir: &Path) -> Result<NativeBackend, LifecycleError> {
    let events_path = verbreel_dir.join("events.jsonl");
    NativeBackend::open(&events_path).map_err(|e| match e {
        BackendError::Locked => LifecycleError::LockHeldByAnotherProcess { holder_pid: None },
        other => LifecycleError::Backend(other),
    })
}

/// Read `events.jsonl` from the backend and parse line-by-line.
/// On the first parse failure, truncate the backend to the line's
/// starting offset and stop (per §0.8 torn-line recovery). Returns
/// the list of valid events.
fn parse_and_repair_log(backend: &NativeBackend) -> Result<Vec<Event>, LifecycleError> {
    let bytes = backend.read_all()?;
    let mut events: Vec<Event> = Vec::new();
    let mut pos: usize = 0;

    while pos < bytes.len() {
        if let Some(rel) = bytes[pos..].iter().position(|&b| b == b'\n') {
            let line = &bytes[pos..pos + rel];
            let line_end = pos + rel + 1; // past the \n
            if !line.is_empty() {
                match serde_json::from_slice::<Event>(line) {
                    Ok(ev) => events.push(ev),
                    Err(e) => {
                        warn!(
                            offset = pos,
                            error = %e,
                            "torn event line — truncating events.jsonl"
                        );
                        backend.truncate(pos as u64)?;
                        return Ok(events);
                    }
                }
            }
            pos = line_end;
        } else {
            // No more newlines — the remainder is either empty
            // (file already ended with \n) or a torn final line
            // without a terminator. Empty case: done.
            let tail = &bytes[pos..];
            if tail.is_empty() {
                break;
            }
            // Torn tail (no terminator). Treat as torn regardless of
            // whether it parses — the §0.8 contract is that a line is
            // "complete" only when terminated by \n.
            warn!(
                offset = pos,
                "torn final line (no newline terminator) — truncating events.jsonl"
            );
            backend.truncate(pos as u64)?;
            break;
        }
    }

    Ok(events)
}
