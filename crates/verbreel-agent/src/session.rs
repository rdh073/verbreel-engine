//! [`Session`] — an editing session bound to one on-disk project.
//!
//! A `Session` wraps a [`verbreel_state::ProjectStore`] (the §0.8
//! write-ordering kernel) and is the single front door the AX surfaces
//! (CLI / HTTP / MCP) route verb calls through. It adds three things on
//! top of the raw store:
//!
//! 1. **Read-vs-write routing.** A verb whose `compute_patch` yields an
//!    empty RFC 6902 patch changed nothing, so persisting an event line
//!    for it would pollute the log. [`Session::run`] peeks the patch and
//!    routes such read-only verbs around the event log entirely
//!    ([`RunOutcome::Query`]); only state-changing verbs take the
//!    canonical [`ProjectStore::mutate_via_verb`] forward path.
//! 2. **`project_id` injection.** Every kernel verb's args carry a
//!    `project_id`; the session injects the open project's id when a
//!    caller omits it, so agents author args without threading ids.
//! 3. **Plan application + undo/redo.** [`Session::apply_plan`] runs an
//!    ordered [`Plan`] all-or-nothing (validated up front), and
//!    [`Session::undo`] / [`Session::redo`] drive the kernel's
//!    `timeline.undo` / `timeline.redo` verbs.
//!
//! The store holds an exclusive `flock` on `events.jsonl` for the
//! session's lifetime, so one `Session` per project at a time — a second
//! `open` on the same root returns
//! [`verbreel_state::LifecycleError::LockHeldByAnotherProcess`].

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use verbreel_state::{
    MutateOutcome, Project, ProjectId, ProjectStore, VerbRegistry, default_fixtures,
    default_registry,
};

use crate::capabilities::Capabilities;
use crate::error::AgentError;
use crate::plan::Plan;

/// Verbs that must always take the canonical
/// [`ProjectStore::mutate_via_verb`] path rather than the read-only peek
/// — they need store internals the generic `Verb::compute_patch` does
/// not see. `asset.import` is special-cased inside the store to resolve
/// the content-addressed destination against the project root.
const STORE_SPECIAL_VERBS: &[&str] = &["asset.import"];

/// The result of running one verb through [`Session::run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    /// A read-only verb: empty patch, no event written, state unchanged.
    /// `data` is the verb's typed envelope.
    Query {
        /// The verb's `data` envelope.
        data: Value,
        /// Warnings the verb emitted. Read-only verbs rarely warn, but
        /// the field is threaded for symmetry with [`RunOutcome::Mutated`]
        /// so an advisory warning on a query is never silently dropped.
        warnings: Vec<Value>,
    },
    /// A state-changing verb: an event was durably written (§0.8) and the
    /// patch applied to the in-memory project.
    Mutated {
        /// The id of the event the call emitted (`UUIDv7` string).
        event_id: String,
        /// The verb's typed `data` envelope.
        data: Value,
        /// Warnings the verb emitted.
        warnings: Vec<Value>,
    },
    /// An idempotent replay: the same `idempotency_key` + args was already
    /// executed, so nothing new was written or applied. `data` is
    /// reconstructed from the recorded event line; `warnings` ends with
    /// `W_REPLAY`.
    Replayed {
        /// The id of the original first call's event.
        event_id: String,
        /// The reconstructed `data` envelope.
        data: Value,
        /// Original warnings plus the appended `W_REPLAY`.
        warnings: Vec<Value>,
    },
}

impl RunOutcome {
    /// Borrow the `data` envelope regardless of outcome variant.
    #[must_use]
    pub fn data(&self) -> &Value {
        match self {
            RunOutcome::Query { data, .. }
            | RunOutcome::Mutated { data, .. }
            | RunOutcome::Replayed { data, .. } => data,
        }
    }

    /// Whether this outcome changed project state (i.e. wrote an event).
    #[must_use]
    pub fn mutated(&self) -> bool {
        matches!(self, RunOutcome::Mutated { .. })
    }
}

/// One applied step's outcome, paired with the verb that produced it.
#[derive(Debug, Clone)]
pub struct StepResult {
    /// The verb id that ran.
    pub verb: String,
    /// What running it produced.
    pub outcome: RunOutcome,
}

/// An editing session over a single on-disk project.
pub struct Session {
    store: ProjectStore,
    /// A clone of the kernel registry, used for the read-only `compute_patch`
    /// peek. (The store's own registry is private and only reachable via
    /// the forward router.)
    registry: VerbRegistry,
}

impl Session {
    /// Open an existing project rooted at `root`
    /// (`<root>/project.json` + `<root>/.verbreel/events.jsonl`).
    ///
    /// Wires the full kernel verb registry so every editing verb is
    /// dispatchable, and clears the §0.8 reconstructor-purity startup
    /// gate by construction (`default_registry` + `default_fixtures`).
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::Lifecycle`] when the project is missing,
    /// corrupt, schema-mismatched, or its event-log lock is held by
    /// another process.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, AgentError> {
        let registry = default_registry();
        let store = ProjectStore::open_with_registry(root, &registry, &default_fixtures())?;
        Ok(Self { store, registry })
    }

    /// Create a fresh project under `workspace` and open a session on it.
    ///
    /// The kernel `project.create` (§2.1) places the project at
    /// `<workspace>/<name>` — `workspace` is the parent directory, not
    /// the project root. The actual root is read back from the create
    /// envelope and returned via [`Session::root`]. `workspace` is made
    /// absolute first (`project.create` rejects a relative `at`).
    ///
    /// `canvas` is the `"<W>x<H>"` literal `project.create` expects;
    /// `fps` defaults to 30/1 when `None`. Two tracks (`Video 1`,
    /// `Audio 1`) are seeded.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::ProjectCreate`] when creation fails (a
    /// project of that name already exists under `workspace`, bad
    /// canvas/fps, IO error) and [`AgentError::Lifecycle`] if the
    /// freshly-created project cannot be re-opened.
    pub fn create(
        workspace: impl AsRef<Path>,
        name: &str,
        canvas: &str,
        fps: Option<(u32, u32)>,
    ) -> Result<Self, AgentError> {
        let workspace = std::path::absolute(workspace.as_ref())
            .map_err(|e| AgentError::ProjectCreate(format!("workspace path: {e}")))?;
        let args = verbreel_state::ProjectCreateArgs {
            name: name.to_string(),
            canvas: canvas.to_string(),
            fps_num: fps.map(|(n, _)| n),
            fps_den: fps.map(|(_, d)| d),
            at: Some(workspace),
            activate: false,
            metadata: Map::new(),
        };
        let created = verbreel_state::project_create(&args)
            .map_err(|e| AgentError::ProjectCreate(e.to_string()))?;
        Self::open(created.path)
    }

    /// Run one verb against the open project.
    ///
    /// Read-only verbs (empty patch) return [`RunOutcome::Query`] with no
    /// event written. State-changing verbs take the canonical §0.8
    /// forward path and return [`RunOutcome::Mutated`] (or
    /// [`RunOutcome::Replayed`] on an idempotent retry). The open
    /// project's id is injected into `args` when absent.
    ///
    /// This does **not** persist `project.json` — call [`Session::save`]
    /// (or [`Session::apply_plan`], which saves once at the end) to
    /// snapshot. The event log is always durable regardless.
    ///
    /// # Errors
    ///
    /// - [`AgentError::UnknownVerb`] — verb not in the kernel registry.
    /// - [`AgentError::VerbExecution`] — a read-only verb's
    ///   `compute_patch` rejected the args.
    /// - [`AgentError::Lifecycle`] — a mutation failed at the kernel
    ///   (apply, IO, idempotency).
    pub fn run(
        &mut self,
        verb_id: &str,
        args: Value,
        idempotency_key: Option<String>,
    ) -> Result<RunOutcome, AgentError> {
        let args = self.inject_project_id(args);

        // Read-only peek (skipped for store-special verbs, which must take
        // the forward router so the store can supply its internals).
        if !STORE_SPECIAL_VERBS.contains(&verb_id) {
            let verb = self
                .registry
                .get(verb_id)
                .ok_or_else(|| AgentError::UnknownVerb {
                    verb: verb_id.to_string(),
                })?;
            let (patch, data, warnings) =
                verb.compute_patch(self.store.project(), &args)
                    .map_err(|source| AgentError::VerbExecution {
                        verb: verb_id.to_string(),
                        source,
                    })?;
            if patch.0.is_empty() {
                return Ok(RunOutcome::Query { data, warnings });
            }
        }

        // State-changing verb (or store-special): canonical §0.8 forward
        // path. `mutate_via_verb` recomputes the patch internally — a tiny
        // duplicate of the pure peek above, traded for the store's
        // idempotency/replay handling and asset.import special-casing.
        match self.store.mutate_via_verb(verb_id, args, idempotency_key)? {
            MutateOutcome::Applied {
                event_id,
                data,
                warnings,
            } => Ok(RunOutcome::Mutated {
                event_id: event_id.to_string(),
                data,
                warnings,
            }),
            MutateOutcome::Replayed {
                event_id,
                data,
                warnings,
            } => Ok(RunOutcome::Replayed {
                event_id: event_id.to_string(),
                data,
                warnings,
            }),
        }
    }

    /// Validate then apply every step of `plan` in order, saving the
    /// project once at the end.
    ///
    /// The plan is structurally validated against the capability catalog
    /// up front (unknown verb / non-object args fail before step 0 runs).
    /// At apply time, a per-step failure (bad args, §0.13 invariant)
    /// stops the run and returns the error — earlier steps remain applied
    /// and their events durable (the engine has no transaction across
    /// verbs; each verb is its own §0.8 unit). The successfully-applied
    /// step results up to the failure are not lost: callers that need
    /// them should run steps individually via [`Session::run`].
    ///
    /// # Errors
    ///
    /// - [`AgentError::InvalidPlan`] — a step failed up-front validation.
    /// - Any error [`Session::run`] can return, from the first failing
    ///   step.
    /// - [`AgentError::Lifecycle`] — the final save failed.
    pub fn apply_plan(
        &mut self,
        plan: &Plan,
        caps: &Capabilities,
    ) -> Result<Vec<StepResult>, AgentError> {
        plan.validate(caps)?;
        let mut results = Vec::with_capacity(plan.steps.len());
        for step in &plan.steps {
            let outcome = self.run(&step.verb, step.args.clone(), None)?;
            results.push(StepResult {
                verb: step.verb.clone(),
                outcome,
            });
        }
        self.save()?;
        Ok(results)
    }

    /// Undo the most recent mutation (kernel `timeline.undo` verb).
    ///
    /// # Errors
    ///
    /// Returns whatever [`Session::run`] surfaces — including the verb's
    /// own "nothing to undo" rejection as [`AgentError::Lifecycle`].
    pub fn undo(&mut self) -> Result<RunOutcome, AgentError> {
        self.run("timeline.undo", Value::Object(Map::new()), None)
    }

    /// Redo the most recently undone mutation (kernel `timeline.redo`).
    ///
    /// # Errors
    ///
    /// Returns whatever [`Session::run`] surfaces.
    pub fn redo(&mut self) -> Result<RunOutcome, AgentError> {
        self.run("timeline.redo", Value::Object(Map::new()), None)
    }

    /// Persist the in-memory project to `<root>/project.json` (§2.3).
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::Lifecycle`] on IO failure.
    pub fn save(&mut self) -> Result<PathBuf, AgentError> {
        Ok(self.store.save()?.path)
    }

    /// Borrow the in-memory project snapshot.
    #[must_use]
    pub fn project(&self) -> &Project {
        self.store.project()
    }

    /// The open project's id.
    #[must_use]
    pub fn project_id(&self) -> ProjectId {
        self.store.project().id
    }

    /// The on-disk project root.
    #[must_use]
    pub fn root(&self) -> &Path {
        self.store.root()
    }

    /// Inject the open project's id into `args` when the caller omitted
    /// it. A non-object `args` is wrapped into a fresh object carrying
    /// only `project_id` — the verb's own arg deserialization then
    /// surfaces the shape error, keeping the failure in the verb layer
    /// where it belongs rather than masking it here.
    fn inject_project_id(&self, args: Value) -> Value {
        let mut map = match args {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        map.entry("project_id".to_string())
            .or_insert_with(|| serde_json::json!(self.project_id()));
        Value::Object(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_project() -> (tempfile::TempDir, Session) {
        let dir = tempfile::tempdir().expect("tempdir");
        let session =
            Session::create(dir.path(), "demo", "1920x1080", None).expect("create project");
        (dir, session)
    }

    #[test]
    fn create_then_reopen_roundtrips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (id, root) = {
            let s = Session::create(dir.path(), "demo", "1920x1080", None).expect("create");
            (s.project_id(), s.root().to_path_buf())
        };
        // Reopen the actual project root: same id replayed off disk.
        let reopened = Session::open(&root).expect("reopen");
        assert_eq!(reopened.project_id(), id);
        assert_eq!(reopened.project().name, "demo");
    }

    #[test]
    fn read_only_verb_writes_no_event() {
        let (_dir, mut session) = temp_project();
        let before = events_len(session.root());
        let outcome = session
            .run("project.info", json!({}), None)
            .expect("project.info runs");
        assert!(matches!(outcome, RunOutcome::Query { .. }));
        assert!(!outcome.mutated());
        // No event line appended for a read-only verb.
        assert_eq!(events_len(session.root()), before);
    }

    #[test]
    fn mutating_verb_writes_an_event_and_persists() {
        let (_dir, mut session) = temp_project();
        let before = events_len(session.root());
        // project.rename is a state-changing verb.
        let outcome = session
            .run("project.rename", json!({ "name": "renamed" }), None)
            .expect("project.rename runs");
        assert!(outcome.mutated(), "expected a mutation, got {outcome:?}");
        assert_eq!(session.project().name, "renamed");
        // Exactly one new event line.
        assert_eq!(events_len(session.root()), before + 1);
        // Survives save + reopen. Drop the first session first: it holds
        // the exclusive events.jsonl flock for its lifetime, so reopening
        // while it is alive would (correctly) hit LockHeldByAnotherProcess.
        session.save().expect("save");
        let root = session.root().to_path_buf();
        drop(session);
        let reopened = Session::open(&root).expect("reopen");
        assert_eq!(reopened.project().name, "renamed");
    }

    #[test]
    fn unknown_verb_is_rejected_without_touching_the_log() {
        let (_dir, mut session) = temp_project();
        let before = events_len(session.root());
        let err = session
            .run("clip.teleport", json!({}), None)
            .expect_err("unknown verb rejected");
        assert!(matches!(err, AgentError::UnknownVerb { .. }));
        assert_eq!(events_len(session.root()), before);
    }

    #[test]
    fn malformed_args_surface_as_verb_execution_without_an_event() {
        let (_dir, mut session) = temp_project();
        let before = events_len(session.root());
        // clip.trim expects `clip: String`; a number trips the verb's own
        // arg deserialization in the read-only peek, before any patch or
        // event — surfacing as VerbExecution, not a lifecycle error.
        let err = session
            .run("clip.trim", json!({ "clip": 123 }), None)
            .expect_err("malformed args must be rejected");
        assert!(
            matches!(err, AgentError::VerbExecution { .. }),
            "expected VerbExecution, got {err:?}"
        );
        // The peek wrote nothing.
        assert_eq!(events_len(session.root()), before);
    }

    #[test]
    fn apply_plan_runs_steps_in_order() {
        let (_dir, mut session) = temp_project();
        let caps = Capabilities::current();
        let plan = Plan::from_json(&json!([
            { "verb": "project.rename", "args": { "name": "step-one" } },
            { "verb": "track.add", "args": { "kind": "video", "name": "extra" } }
        ]))
        .expect("plan parses");
        let results = session.apply_plan(&plan, &caps).expect("plan applies");
        assert_eq!(results.len(), 2);
        assert_eq!(session.project().name, "step-one");
        // track.add added a third track (Video 1 + Audio 1 seeded + extra).
        assert!(session.project().tracks.len() >= 3);
    }

    #[test]
    fn apply_plan_rejects_unknown_verb_before_running() {
        let (_dir, mut session) = temp_project();
        let caps = Capabilities::current();
        let before = events_len(session.root());
        let plan = Plan::from_json(&json!([
            { "verb": "project.rename", "args": { "name": "should-not-apply" } },
            { "verb": "clip.teleport", "args": {} }
        ]))
        .expect("plan parses");
        let err = session
            .apply_plan(&plan, &caps)
            .expect_err("invalid plan rejected up front");
        assert!(matches!(err, AgentError::InvalidPlan { index: 1, .. }));
        // Up-front validation means step 0 never ran.
        assert_eq!(events_len(session.root()), before);
        assert_eq!(session.project().name, "demo");
    }

    /// Count the lines in the project's events.jsonl (one event per line).
    fn events_len(root: &Path) -> usize {
        let path = root.join(".verbreel").join("events.jsonl");
        std::fs::read_to_string(&path).map_or(0, |s| s.lines().count())
    }
}
