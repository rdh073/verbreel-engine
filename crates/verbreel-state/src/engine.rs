//! [`Engine`] — the single shared verb-dispatch surface (§commands.md:3-9).
//!
//! CLI, MCP, and HTTP are all thin shells over one engine. This module
//! is that engine for the native (persisted) build: it holds a map of
//! **open** projects keyed by [`ProjectId`] (§0.12 — `project_id` is a
//! client-supplied arg naming an open project, never inferred from path
//! or process state) and routes any `<noun>.<verb>` call plus a JSON
//! args object to the right place:
//!
//! - **Lifecycle verbs** (`project.create` / `.open` / `.save` /
//!   `.close` / `.duplicate` / `.forget`) → bespoke handlers that manage
//!   the open-project map by composing the six free functions in
//!   [`crate::verbs::project_*`]. These are NOT registry verbs — they
//!   manage on-disk + open-map state, not in-graph state.
//! - **Project-less read verbs** (`list_capabilities` / `help` /
//!   `schema` / `validate_command` / `project.list`) → run the
//!   registry verb's `compute_patch` against a synthetic empty project
//!   (they ignore project state) and return `event_id: ""`.
//! - **Project-scoped verbs** → resolve `project_id` from args, look the
//!   open [`ProjectStore`] up in the map (absent ⇒ `E_PROJECT_NOT_FOUND`),
//!   and run [`ProjectStore::mutate_via_verb`] (§0.8 persist) — or, under
//!   §0.5.1 `dry_run`, [`ProjectStore::compute_via_verb`] (compute, no
//!   persist).
//!
//! Every call returns an [`Envelope`] (§0.1).
//!
//! ## §0.5 universal args
//!
//! `dry_run`, `idempotency_key`, and `exact_time` are read from the
//! top-level of the args object. On **project-scoped** verbs `dry_run`
//! computes the patch but skips persistence and returns `event_id: ""`
//! (§0.5.1) and `idempotency_key` threads to `mutate_via_verb` (ignored
//! under `dry_run` per §0.5.1). On **lifecycle** verbs `dry_run` is
//! rejected (the free functions have no compute-only path — they always
//! persist — so honoring §0.5.1's "no persistent side effect" means not
//! running them; see `dispatch_lifecycle`). `exact_time` is passed
//! through to the verb in `args` untouched — the verb owns its snap
//! semantics.

#![cfg(feature = "native")]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde_json::{Value, json};
use verbreel_types::ProjectId;

use crate::lifecycle::{LifecycleError, MutateOutcome, ProjectStore};
use crate::reconstructor::VerbRegistry;
use crate::verbs::project_list::{ProjectEntry, ProjectListData, ProjectListState};
use crate::verbs::{
    project_close, project_create, project_duplicate, project_forget, project_open, project_save,
};

/// The six lifecycle verbs the [`Engine`] handles outside the registry.
/// They manage on-disk state + the open-project map, not in-graph state.
const LIFECYCLE_VERBS: [&str; 6] = [
    "project.create",
    "project.open",
    "project.save",
    "project.close",
    "project.duplicate",
    "project.forget",
];

/// Warning code emitted when the §2.6 projects-index write fails but the
/// project itself is created/opened (`register_in_index`). Non-fatal:
/// the project is usable in-session; only cross-process resolution by id
/// is affected until a successful re-register.
const W_INDEX_WRITE_FAILED: &str = "W_INDEX_WRITE_FAILED";

/// Read verbs that span all open projects and take no `project_id`
/// (§0.12). They are registry verbs whose `compute_patch` ignores the
/// project graph, so the engine runs them against a synthetic empty
/// project and returns `event_id: ""`.
///
/// `project.list` is NOT in this set: per §2.6 it reports live engine
/// state (open vs index-discovered projects) and prunes stale index
/// entries — both of which need `home` + the open-map, which the pure
/// registry `compute_patch` cannot see. It gets its own engine handler.
const PROJECTLESS_READ_VERBS: [&str; 4] =
    ["list_capabilities", "help", "schema", "validate_command"];

/// §0.1 result envelope. Wraps every verb's `data` (the per-verb payload)
/// on success, or a `{code, message, ...}` failure shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Envelope {
    /// `ok: true` — verb succeeded.
    Ok {
        /// Verb-specific payload.
        data: Value,
        /// RFC 6902 patch describing the change (`[]` for read-only /
        /// dry-run).
        patch: Value,
        /// Non-fatal advisories.
        warnings: Vec<Value>,
        /// `UUIDv7` of the recorded event; `""` for read-only verbs and
        /// dry runs (§0.1).
        event_id: String,
    },
    /// `ok: false` — verb failed.
    Err {
        /// `E_NAMESPACED_CODE` (§0.7).
        code: String,
        /// Human-readable, one line.
        message: String,
        /// Actionable next step (§0.1).
        hint: Option<String>,
        /// Verb-specific diagnostic data (§0.1).
        details: Option<Value>,
    },
}

impl Envelope {
    /// Serialize to the exact §0.1 wire shape: `{ok:true, data, patch,
    /// warnings, event_id}` or `{ok:false, code, message, hint?,
    /// details?}`. Absent `hint` / `details` are omitted (not emitted as
    /// `null`) to match the `?`-optional fields in §0.1.
    #[must_use]
    pub fn to_json(&self) -> Value {
        match self {
            Envelope::Ok {
                data,
                patch,
                warnings,
                event_id,
            } => json!({
                "ok": true,
                "data": data,
                "patch": patch,
                "warnings": warnings,
                "event_id": event_id,
            }),
            Envelope::Err {
                code,
                message,
                hint,
                details,
            } => {
                let mut obj = serde_json::Map::new();
                obj.insert("ok".into(), Value::Bool(false));
                obj.insert("code".into(), Value::String(code.clone()));
                obj.insert("message".into(), Value::String(message.clone()));
                if let Some(hint) = hint {
                    obj.insert("hint".into(), Value::String(hint.clone()));
                }
                if let Some(details) = details {
                    obj.insert("details".into(), details.clone());
                }
                Value::Object(obj)
            }
        }
    }

    /// `true` for the [`Envelope::Ok`] variant.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        matches!(self, Envelope::Ok { .. })
    }

    /// Build an `ok:true` envelope.
    fn ok(data: Value, patch: Value, warnings: Vec<Value>, event_id: String) -> Self {
        Envelope::Ok {
            data,
            patch,
            warnings,
            event_id,
        }
    }

    /// Build an `ok:false` envelope from a code + message.
    fn err(code: impl Into<String>, message: impl Into<String>) -> Self {
        Envelope::Err {
            code: code.into(),
            message: message.into(),
            hint: None,
            details: None,
        }
    }
}

/// The shared verb-dispatch engine (native, persisted).
///
/// Holds a map of open projects keyed by [`ProjectId`], the verb
/// registry (for routing + `verb_ids` + the project-less reads), and the
/// user home used to resolve the projects index.
///
/// The §0.8 reconstructor-purity startup gate (registry + fixtures) is
/// applied by the lifecycle free functions themselves when they
/// `create` / `open` a store — they build the canonical
/// `default_registry()` + `default_fixtures()` pair internally — so the
/// engine does not hold a separate fixtures vector.
pub struct Engine {
    open: HashMap<ProjectId, ProjectStore>,
    registry: VerbRegistry,
    home: PathBuf,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("open", &self.open.keys().collect::<Vec<_>>())
            .field("home", &self.home)
            .finish_non_exhaustive()
    }
}

impl Engine {
    /// Construct a fresh engine with the canonical verb registry +
    /// fixtures and no open projects. `home` is the user directory whose
    /// `.verbreel/projects-index` resolves a `project_id` to a root.
    #[must_use]
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self {
            open: HashMap::new(),
            registry: crate::default_registry(),
            home: home.into(),
        }
    }

    /// THE single surface entry point. Route `verb_id` + `args` to the
    /// right handler and return the §0.1 [`Envelope`].
    // `args` is taken by value: this is the owned command hand-off from the
    // transport layer (CLI/MCP/HTTP), so the public boundary owns the parsed
    // command while the private routers only borrow it. needless_pass_by_value
    // is a known false-positive for this public ownership-handoff shape.
    #[allow(clippy::needless_pass_by_value)]
    pub fn dispatch(&mut self, verb_id: &str, args: Value) -> Envelope {
        // §0.5: `dry_run` is a boolean universal arg. Type-check it once here,
        // at the routing boundary, before any path acts on it. It is stripped
        // before the per-verb `deny_unknown_fields` struct deserializes and
        // dispatch runs no JSON-schema validation, so this is the only place a
        // malformed `dry_run` is caught. Reading it via `bool_arg`'s lossy
        // `as_bool().unwrap_or(false)` instead would coerce `{"dry_run":"true"}`
        // / `1` / `null` to `false` and silently run the REAL mutation under a
        // preview-intent flag (§0.5.1 violation) — destructive for both
        // `dispatch_lifecycle` (e.g. project.forget) and `dispatch_project_scoped`
        // (e.g. keyframe.remove). Rejecting a present-but-non-boolean value here
        // closes that trap for every route at the root.
        let dry_run = match dry_run_flag(&args) {
            Ok(flag) => flag,
            Err(env) => return *env,
        };
        if LIFECYCLE_VERBS.contains(&verb_id) {
            return self.dispatch_lifecycle(verb_id, &args, dry_run);
        }
        if verb_id == "project.list" {
            return self.handle_project_list();
        }
        if PROJECTLESS_READ_VERBS.contains(&verb_id) {
            return self.dispatch_projectless_read(verb_id, &args);
        }
        self.dispatch_project_scoped(verb_id, &args, dry_run)
    }

    /// All dispatchable verb ids: the registry verbs (sorted) plus the
    /// six lifecycle ids the engine handles outside the registry.
    #[must_use]
    pub fn verb_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .registry
            .verbs()
            .iter()
            .map(|v| (*v).to_string())
            .collect();
        for lc in LIFECYCLE_VERBS {
            ids.push(lc.to_string());
        }
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// JSON Schema for a verb's args. Returns the hand-curated
    /// well-known schema if one exists for `verb_id`, else the v1
    /// permissive floor (`{type:object, additionalProperties:true}` per
    /// `schema.rs`'s vacuous-schema rule).
    #[must_use]
    pub fn schema_for(&self, verb_id: &str) -> Value {
        let registry = verbreel_args::well_known::default_registry();
        if let Some(schema) = registry.get(verb_id) {
            return schema.as_value().clone();
        }
        json!({
            "type": "object",
            "additionalProperties": true,
            "title": format!("{verb_id} args"),
        })
    }

    // ---------------------------------------------------------------
    // Routing — project-scoped registry verbs
    // ---------------------------------------------------------------

    fn dispatch_project_scoped(&mut self, verb_id: &str, args: &Value, dry_run: bool) -> Envelope {
        // `dry_run` is the type-validated flag from `dispatch` (a present
        // non-boolean was already rejected there) — NOT re-read via the lossy
        // `bool_arg`, which would coerce a malformed value to `false` and run
        // the real mutation under a preview flag. `keyframe.remove --dry_run`
        // is the canonical destructive-preview case this protects.

        // Unknown verb (not a registry verb and not a lifecycle verb).
        if self.registry.get(verb_id).is_none() {
            return Envelope::err("E_UNKNOWN_VERB", format!("unknown verb: {verb_id}"));
        }

        let project_id = match project_id_from_args(args) {
            Ok(id) => id,
            Err(env) => return *env,
        };

        if !self.open.contains_key(&project_id) {
            return project_not_found(&project_id);
        }

        let idempotency_key = string_arg(args, "idempotency_key");

        // Strip the engine-consumed universal args before the verb sees
        // them: they are routing controls, not verb args (§0.5), and a
        // verb with `deny_unknown_fields` would otherwise reject them.
        let verb_args = strip_universal_args(args);

        if dry_run {
            // §0.5.1: compute the patch, skip persistence, `event_id: ""`,
            // ignore `idempotency_key`.
            let store = &self.open[&project_id];
            return match store.compute_via_verb(verb_id, &verb_args) {
                Ok((patch, data, warnings)) => {
                    match serialize_or_internal(verb_id, "patch", &patch) {
                        Ok(patch_value) => Envelope::ok(data, patch_value, warnings, String::new()),
                        Err(env) => *env,
                    }
                }
                Err(err) => lifecycle_error_to_envelope(verb_id, &err),
            };
        }

        let store = self
            .open
            .get_mut(&project_id)
            .expect("contains_key checked above");
        match store.mutate_via_verb(verb_id, verb_args, idempotency_key) {
            Ok(outcome) => mutate_outcome_to_envelope(verb_id, outcome),
            Err(err) => lifecycle_error_to_envelope(verb_id, &err),
        }
    }

    // ---------------------------------------------------------------
    // Routing — project-less read verbs
    // ---------------------------------------------------------------

    fn dispatch_projectless_read(&self, verb_id: &str, args: &Value) -> Envelope {
        let Some(verb) = self.registry.get(verb_id) else {
            // Defensive: every PROJECTLESS_READ_VERBS id is registered.
            return Envelope::err("E_UNKNOWN_VERB", format!("unknown verb: {verb_id}"));
        };
        // §0.12: these verbs take no `project_id` from the caller. The
        // kernel `*Args` structs still carry a `project_id` field for
        // `Verb`-trait uniformity (it is never read by the impl — see
        // `schema.rs` / `list_capabilities.rs`), so the engine injects a
        // synthetic valid UUID to satisfy deserialization. The caller
        // never supplies one.
        // Defensive: the project-less read arg structs do not yet carry
        // `deny_unknown_fields`, so a stray universal arg is currently
        // tolerated — but strip it anyway so the day one of them tightens to
        // deny-unknown, a keyed/dry-run call here does not start 400ing (§0.5).
        let synthetic_id = ProjectId::now();
        let args = inject_project_id(&strip_universal_args(args), synthetic_id);
        let synthetic = crate::synthetic_empty_project(synthetic_id);
        match verb.compute_patch(&synthetic, &args) {
            Ok((patch, data, warnings)) => match serialize_or_internal(verb_id, "patch", &patch) {
                Ok(patch_value) => Envelope::ok(data, patch_value, warnings, String::new()),
                Err(env) => *env,
            },
            Err(err) => Envelope::err(verb_error_code(&err), format!("{verb_id}: {err}")),
        }
    }

    // ---------------------------------------------------------------
    // Routing — lifecycle verbs (manage the open-project map)
    // ---------------------------------------------------------------

    fn dispatch_lifecycle(&mut self, verb_id: &str, args: &Value, dry_run: bool) -> Envelope {
        // §0.5.1: under `dry_run` the engine guarantees NO persistent side
        // effect. The six lifecycle free functions only ever persist
        // (create/duplicate write a project to disk, save rewrites the
        // snapshot, forget removes the on-disk root, open/close mutate the
        // flock + open-map) — none has a compute-only path. Stripping
        // `dry_run` and proceeding would silently perform the real mutation,
        // turning `project.forget --dry_run` into an irreversible delete under
        // a flag whose whole purpose is "preview, no side effects". So reject
        // a `dry_run: true` lifecycle call explicitly instead of
        // strip-and-proceed: a safe error, not a silent destructive action.
        // `dry_run` is the type-validated flag from `dispatch` (a present
        // non-boolean was already rejected there as a malformed universal arg).
        if dry_run {
            return Envelope::Err {
                code: "E_SCHEMA_VIOLATION".to_string(),
                message: format!(
                    "{verb_id}: dry_run is not supported on lifecycle verbs \
                     (they have no compute-only path; §0.5.1)"
                ),
                hint: Some("drop dry_run; lifecycle verbs always persist their effect".to_string()),
                details: Some(json!({ "verb_id": verb_id, "arg": "dry_run" })),
            };
        }
        // Strip the engine-consumed universal args once before any handler
        // deserializes into its `deny_unknown_fields` arg struct: §0.8
        // designates project.create/.duplicate as legitimate idempotency_key
        // surfaces, so a keyed lifecycle call must not 400.
        //
        // `idempotency_key` is stripped (so the keyed call doesn't 400) but
        // NOT yet threaded into the lifecycle free functions — unlike
        // dispatch_project_scoped, which passes it to mutate_via_verb for §0.8
        // dedup. So a keyed project.create/.duplicate is accepted but the key
        // is not honored for dedup: a second keyed call re-executes (and a
        // second create hits E_PROJECT_EXISTS) rather than replaying. Wiring
        // §0.8 dedup through the lifecycle path is tracked in #461; until then
        // the key is accepted-but-not-yet-honored, not silently dedup-effective.
        let args = &strip_universal_args(args);
        match verb_id {
            "project.create" => self.handle_create(args),
            "project.open" => self.handle_open(args),
            "project.save" => self.handle_save(args),
            "project.close" => self.handle_close(args),
            // duplicate does not touch the open-project map nor the index —
            // its free function writes only to disk — so it is an associated
            // function. forget reads/writes the engine's projects-index, so
            // it borrows `&self` to reach `self.home` (but never the
            // open-project map).
            "project.duplicate" => Self::handle_duplicate(args),
            "project.forget" => self.handle_forget(args),
            // Unreachable: LIFECYCLE_VERBS gates entry here.
            other => Envelope::err("E_UNKNOWN_VERB", format!("unknown verb: {other}")),
        }
    }

    fn handle_create(&mut self, args: &Value) -> Envelope {
        let typed: project_create::ProjectCreateArgs = match serde_json::from_value(args.clone()) {
            Ok(a) => a,
            Err(e) => return Envelope::err("E_SCHEMA_VIOLATION", format!("project.create: {e}")),
        };
        let data = match project_create::create(&typed) {
            Ok(d) => d,
            Err(e) => return create_error_to_envelope(&e),
        };
        // `create` writes the project on disk and drops the store. Open
        // it into the engine's map so subsequent verbs can target it.
        let open_args = project_open::ProjectOpenArgs {
            path: data.path.clone(),
            strict: false,
        };
        match project_open::open(&open_args) {
            Ok((store, _open_data)) => {
                self.open.insert(store.project().id, store);
            }
            Err(e) => return open_error_to_envelope(&e),
        }
        // §2.6: `project.create` appends the id→root entry to the
        // user-wide projects-index. Without it, a one-shot CLI process
        // (`create` then a fresh `clip.add <id>` in a new process) has an
        // empty open-map AND an empty index, so `resolve_root_for_project_id`
        // returns NotFound and every project-scoped CLI verb is inert.
        // Long-running surfaces (MCP/HTTP) hold the open-map in-session, so
        // this only ever bit the CLI — but the index is the documented
        // durable id→root source of truth, so the engine writes it here.
        let warnings = self.register_in_index(&data.project_id, &typed.name, &data.path);
        match serialize_or_internal("project.create", "data", &data) {
            Ok(data_value) => Envelope::ok(data_value, json!([]), warnings, String::new()),
            Err(env) => *env,
        }
    }

    fn handle_open(&mut self, args: &Value) -> Envelope {
        let typed: project_open::ProjectOpenArgs = match serde_json::from_value(args.clone()) {
            Ok(a) => a,
            Err(e) => return Envelope::err("E_SCHEMA_VIOLATION", format!("project.open: {e}")),
        };
        match project_open::open(&typed) {
            Ok((store, data)) => {
                let id = store.project().id;
                // Register the ABSOLUTIZED root the store resolved, NOT the
                // caller's raw `typed.path`: `project.open` accepts a relative
                // path (resolved against the caller's cwd by `to_absolute`),
                // and a relative entry in the durable index would later resolve
                // against a *different* process's cwd — defeating the
                // cross-process resolution this registration exists for.
                // `store.root()` is the absolute path the verb validated.
                let root = store.root().to_path_buf();
                let name = store.project().name.clone();
                self.open.insert(id, store);
                // §2.6: opening upserts the id→entry, refreshing
                // `last_opened_at`. Keying by id means re-registering an
                // already-indexed project overwrites its entry in place —
                // no duplicate, no compaction.
                let warnings = self.register_in_index(&id, &name, &root);
                match serialize_or_internal("project.open", "data", &data) {
                    Ok(data_value) => Envelope::ok(data_value, json!([]), warnings, String::new()),
                    Err(env) => *env,
                }
            }
            Err(e) => open_error_to_envelope(&e),
        }
    }

    fn handle_save(&mut self, args: &Value) -> Envelope {
        let typed: project_save::ProjectSaveArgs = match serde_json::from_value(args.clone()) {
            Ok(a) => a,
            Err(e) => return Envelope::err("E_SCHEMA_VIOLATION", format!("project.save: {e}")),
        };
        let Some(store) = self.open.get_mut(&typed.project_id) else {
            return project_not_found(&typed.project_id);
        };
        match project_save::save(store, &typed) {
            Ok(data) => match serialize_or_internal("project.save", "data", &data) {
                Ok(data_value) => Envelope::ok(data_value, json!([]), Vec::new(), String::new()),
                Err(env) => *env,
            },
            Err(e) => save_error_to_envelope(&e),
        }
    }

    fn handle_close(&mut self, args: &Value) -> Envelope {
        let typed: project_close::ProjectCloseArgs = match serde_json::from_value(args.clone()) {
            Ok(a) => a,
            Err(e) => return Envelope::err("E_SCHEMA_VIOLATION", format!("project.close: {e}")),
        };
        // Take the store out of the map; `close` consumes it. On the
        // Err-returns-store path, re-insert so the project stays open and
        // the caller can retry (matches §2.5 close-failure semantics).
        let Some(store) = self.open.remove(&typed.project_id) else {
            return project_not_found(&typed.project_id);
        };
        match project_close::close(store, &typed) {
            Ok(data) => match serialize_or_internal("project.close", "data", &data) {
                Ok(data_value) => Envelope::ok(data_value, json!([]), Vec::new(), String::new()),
                Err(env) => *env,
            },
            Err((store, e)) => {
                self.open.insert(store.project().id, store);
                close_error_to_envelope(&e)
            }
        }
    }

    fn handle_duplicate(args: &Value) -> Envelope {
        let typed: project_duplicate::ProjectDuplicateArgs =
            match serde_json::from_value(args.clone()) {
                Ok(a) => a,
                Err(e) => {
                    return Envelope::err("E_SCHEMA_VIOLATION", format!("project.duplicate: {e}"));
                }
            };
        match project_duplicate::duplicate(&typed) {
            Ok(data) => {
                // `duplicate` writes the new project on disk but does not
                // open it (no flock). Leave it closed — the caller chains
                // `project.open` if they want a live handle, matching the
                // free function's contract.
                match serialize_or_internal("project.duplicate", "data", &data) {
                    Ok(data_value) => {
                        Envelope::ok(data_value, json!([]), Vec::new(), String::new())
                    }
                    Err(env) => *env,
                }
            }
            Err(e) => duplicate_error_to_envelope(&e),
        }
    }

    fn handle_forget(&self, args: &Value) -> Envelope {
        let typed: project_forget::ProjectForgetArgs = match serde_json::from_value(args.clone()) {
            Ok(a) => a,
            Err(e) => return Envelope::err("E_SCHEMA_VIOLATION", format!("project.forget: {e}")),
        };
        match project_forget::forget(&self.home, &typed) {
            Ok(data) => match serialize_or_internal("project.forget", "data", &data) {
                Ok(data_value) => Envelope::ok(data_value, json!([]), Vec::new(), String::new()),
                Err(env) => *env,
            },
            Err(e) => forget_error_to_envelope(&e),
        }
    }

    /// `project.list` (§2.6) — live engine read over the projects-index.
    ///
    /// Unlike the pure registry verb (which always returns an empty list
    /// because its `compute_patch` cannot do file IO), this handler reads
    /// the real `<home>/.verbreel/projects-index`, prunes entries whose
    /// path no longer exists (emitting one `W_INDEX_STALE` per removed
    /// entry), and marks each surviving entry `open` if it is held in
    /// this engine's open-map, else `closed`.
    ///
    /// `patch` is `[]` and `event_id` is `""`: per §2.6 the prune rewrite
    /// is engine-state housekeeping, not a project-graph mutation, so it
    /// is not represented in any patch — `W_INDEX_STALE` is the only
    /// observable signal that the index file was rewritten.
    ///
    /// The read/prune is best-effort in the sense that a damaged index
    /// does not fail the verb (a corrupt file should not make "list my
    /// projects" return an error). But unlike a swallow-to-empty, a
    /// read/parse failure surfaces a `W_INDEX_UNREADABLE` envelope
    /// warning so the caller can distinguish "no projects registered"
    /// from "index unreadable" — mirroring the `register_in_index`
    /// `W_INDEX_WRITE_FAILED` convention (both log via `tracing::warn!`
    /// AND emit an envelope warning; neither is silently swallowed).
    ///
    /// Pruning exempts every project this engine currently holds open:
    /// a live project whose path is transiently unreachable must not be
    /// dropped from the durable index by a read-shaped verb (§2.6).
    fn handle_project_list(&self) -> Envelope {
        let mut warnings: Vec<Value> = Vec::new();

        let exempt: std::collections::BTreeSet<String> =
            self.open.keys().map(ToString::to_string).collect();

        // One locked acquisition reads + prunes + returns the surviving
        // map, so there is no read-after-prune race and the index is
        // touched once. A read/parse failure is non-fatal but observable.
        let index = match verbreel_storage::layout::list_and_prune(&self.home, &exempt) {
            Ok((index, removed)) => {
                for id in &removed {
                    warnings.push(json!({
                        "code": "W_INDEX_STALE",
                        "message": format!(
                            "removed stale projects-index entry for `{id}` (path no longer exists)"
                        ),
                        "details": { "project_id": id },
                    }));
                }
                index
            }
            Err(e) => {
                tracing::warn!(error = %e, "project.list: reading the projects-index failed");
                warnings.push(json!({
                    "code": "W_INDEX_UNREADABLE",
                    "message": format!(
                        "the projects-index could not be read; \
                         project.list may be incomplete: {e}"
                    ),
                    "details": { "error": e.to_string() },
                }));
                verbreel_storage::layout::ProjectsIndex::new()
            }
        };

        let mut projects: Vec<ProjectEntry> = index
            .into_values()
            .map(|entry| {
                let state = match ProjectId::from_str(&entry.project_id) {
                    Ok(id) if self.open.contains_key(&id) => ProjectListState::Open,
                    _ => ProjectListState::Closed,
                };
                ProjectEntry {
                    id: entry.project_id,
                    name: entry.name,
                    path: entry.path,
                    state,
                    last_opened_at: entry.last_opened_at,
                }
            })
            .collect();
        projects.sort_by(|a, b| a.id.cmp(&b.id));

        let data = ProjectListData { projects };
        match serialize_or_internal("project.list", "data", &data) {
            Ok(data_value) => Envelope::ok(data_value, json!([]), warnings, String::new()),
            Err(env) => *env,
        }
    }

    /// Register a `(project_id, root)` pair in the user-wide
    /// projects-index at `<home>/.verbreel/projects-index` (§2.6).
    ///
    /// This is the write side of [`Self::resolve_root`]: `create` / `open`
    /// call it so a later process can turn the client-supplied `project_id`
    /// back into a root (§0.12). The `home` is an engine-level concern — the
    /// `project_create` / `project_open` free functions deliberately do not
    /// know it, so the registration lives here, not in the verbs.
    ///
    /// **Index-write failure is non-fatal.** The project is already
    /// created/opened on disk (and, for `open`, the flock is held and the
    /// store is in `self.open`), so failing the whole call would leave the
    /// user with a real on-disk project the API claims does not exist —
    /// strictly worse than an unindexed-but-open project. We therefore do
    /// NOT propagate the error: we log it via `tracing::warn!` (operational
    /// visibility, the crate's recoverable-IO convention) AND return a
    /// `W_INDEX_WRITE_FAILED` warning so the agent observes that the durable
    /// index was not updated (next-process resolution of this id will miss
    /// until a successful re-register). The error is not swallowed silently —
    /// it surfaces on both channels — it is just not promoted to a fatal.
    ///
    /// Returns the envelope `warnings` vector: empty on success, one
    /// `W_INDEX_WRITE_FAILED` entry on an index-write IO error.
    ///
    /// `name` and `last_opened_at` populate the §2.6 index entry fields:
    /// `create`/`open`/`duplicate` set `last_opened_at` to now (RFC 3339)
    /// so `project.list` can report a meaningful "last opened" ordering.
    fn register_in_index(&self, project_id: &ProjectId, name: &str, root: &Path) -> Vec<Value> {
        let last_opened_at = verbreel_events::Timestamp::now();
        match verbreel_storage::layout::register_project(
            &self.home,
            &project_id.to_string(),
            name,
            root,
            last_opened_at.as_str(),
        ) {
            Ok(()) => Vec::new(),
            Err(e) => {
                tracing::warn!(
                    project_id = %project_id,
                    root = %root.display(),
                    error = %e,
                    "failed to write projects-index entry; project is usable in-session \
                     but will not resolve by id in a fresh process until re-registered"
                );
                vec![json!({
                    "code": W_INDEX_WRITE_FAILED,
                    "message": format!(
                        "project created/opened, but writing the projects-index entry failed: {e}"
                    ),
                    "details": {
                        "project_id": project_id.to_string(),
                        "path": root.display().to_string(),
                    },
                })]
            }
        }
    }

    /// Resolve a `project_id` to its on-disk root via the engine's
    /// `home`/`.verbreel/projects-index` (§0.12). This is the bridge the
    /// surface layers use to turn a client-supplied id into a root they
    /// can hand to `project.open` — the engine itself never infers
    /// project context from process state, only from this explicit
    /// registration index.
    ///
    /// # Errors
    ///
    /// Propagates [`verbreel_storage::layout::ResolveError`]:
    /// `NotFound` (no registration — surfaces map to
    /// `E_PROJECT_NOT_FOUND`), `Io`, or `InvalidIndex`.
    pub fn resolve_root(
        &self,
        project_id: &str,
    ) -> Result<PathBuf, verbreel_storage::layout::ResolveError> {
        verbreel_storage::layout::resolve_root_for_project_id(&self.home, project_id)
    }

    /// Read-only count of open projects. Test + introspection helper.
    #[must_use]
    pub fn open_count(&self) -> usize {
        self.open.len()
    }

    /// `true` if `project_id` names a project currently open in the
    /// engine.
    #[must_use]
    pub fn is_open(&self, project_id: &ProjectId) -> bool {
        self.open.contains_key(project_id)
    }
}

// -------------------------------------------------------------------
// Free helpers
// -------------------------------------------------------------------

/// Extract a `project_id` UUID from the args object. Missing / non-string
/// / unparseable values surface as a typed `Err` envelope so the caller
/// can return it directly.
fn project_id_from_args(args: &Value) -> Result<ProjectId, Box<Envelope>> {
    let raw = args
        .get("project_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            Box::new(Envelope::err(
                "E_SCHEMA_VIOLATION",
                "missing required `project_id` (string UUID) for project-scoped verb",
            ))
        })?;
    ProjectId::from_str(raw).map_err(|e| {
        Box::new(Envelope::err(
            "E_SCHEMA_VIOLATION",
            format!("`project_id` is not a valid UUIDv7: {e}"),
        ))
    })
}

/// Serialize a verb's typed payload to JSON, mapping a serialize failure
/// to an `E_INTERNAL` error envelope instead of silently fabricating an
/// `ok:true` envelope with `data:null` / `patch:[]`.
///
/// `what` names the field for the error message (`"data"` or `"patch"`).
/// Practically unreachable for the typed structs the handlers feed it —
/// `serde_json::to_value` only fails on a non-string map key or a custom
/// `Serialize` that errors, neither of which these types produce — but
/// the §0.7 contract is that a fault on the engine's side surfaces as
/// `E_INTERNAL`, not as a success with a hollowed-out body (consistent
/// with how `verb_error_code` maps `VerbError::Custom`).
fn serialize_or_internal<T: serde::Serialize>(
    verb_id: &str,
    what: &str,
    value: &T,
) -> Result<Value, Box<Envelope>> {
    serde_json::to_value(value).map_err(|e| {
        Box::new(Envelope::err(
            "E_INTERNAL",
            format!("{verb_id}: {what} serialization failed: {e}"),
        ))
    })
}

/// §0.12 `E_PROJECT_NOT_FOUND` envelope with `details.project_id`.
fn project_not_found(project_id: &ProjectId) -> Envelope {
    Envelope::Err {
        code: "E_PROJECT_NOT_FOUND".to_string(),
        message: format!("project `{project_id}` is not open"),
        hint: Some("open the project with project.open before targeting it".to_string()),
        details: Some(json!({ "project_id": project_id.to_string() })),
    }
}

/// Return a clone of `args` (an object, else a fresh object) with
/// `project_id` set to `id`. Used to satisfy the trait-required
/// `project_id` field on project-less read verbs (§0.12) — the value is
/// synthetic and never read by those verbs' impls.
fn inject_project_id(args: &Value, id: ProjectId) -> Value {
    let mut obj = match args {
        Value::Object(m) => m.clone(),
        _ => serde_json::Map::new(),
    };
    obj.insert("project_id".into(), Value::String(id.to_string()));
    Value::Object(obj)
}

/// Type-check + read the §0.5 `dry_run` universal arg.
///
/// Returns `Ok(false)` when absent or explicit `false`, `Ok(true)` when
/// explicit boolean `true`, and an `E_SCHEMA_VIOLATION` envelope when present
/// but not a JSON boolean (`"true"`, `1`, `null`, `{}`, …).
///
/// This deliberately does NOT coerce like `as_bool().unwrap_or(false)`: a
/// lossy coercion of a malformed `dry_run` to `false` would run the real,
/// persistent mutation under a preview-intent flag — a §0.5.1 violation that
/// is destructive on both the lifecycle and project-scoped dispatch paths.
/// `dry_run` is stripped before any `deny_unknown_fields` arg struct
/// deserializes and dispatch runs no JSON-schema validation, so this is the
/// single point where a malformed value is caught.
fn dry_run_flag(args: &Value) -> Result<bool, Box<Envelope>> {
    match args.get("dry_run") {
        None | Some(Value::Bool(false)) => Ok(false),
        Some(Value::Bool(true)) => Ok(true),
        Some(other) => Err(Box::new(Envelope::Err {
            code: "E_SCHEMA_VIOLATION".to_string(),
            message: format!("dry_run must be a boolean, got {}", json_type_name(other)),
            hint: Some("pass dry_run as a JSON boolean (true / false)".to_string()),
            details: Some(json!({ "arg": "dry_run", "value": other })),
        })),
    }
}

/// Name the JSON type of a value for an error message.
fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Read a top-level string arg, if present and non-null.
fn string_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Strip the §0.5 universal args the **engine** itself consumes
/// (`dry_run`, `idempotency_key`) from the args object before handing it
/// to a verb. These are transport-level routing controls, not verb args:
/// the engine reads `dry_run` to pick the compute-vs-persist branch and
/// `idempotency_key` to drive the §0.8 dedup index, then must not leak
/// them into the verb's typed args struct.
///
/// Two reasons this matters:
/// 1. Verbs with `#[serde(deny_unknown_fields)]` (e.g. `asset.import`)
///    reject an unknown `dry_run` / `idempotency_key` field with
///    `E_SCHEMA_VIOLATION` — so without stripping, a dry-run or keyed
///    call against such a verb fails before it ever computes a patch.
/// 2. §0.8 specifies the idempotency fingerprint excludes `dry_run` and
///    `idempotency_key`; stripping here makes the fingerprint computed in
///    the mutate path (over the same args object) match that contract.
///
/// `exact_time` is deliberately NOT stripped — per §0.5 it is passed
/// through to the verb, which owns its snap semantics.
fn strip_universal_args(args: &Value) -> Value {
    let Value::Object(map) = args else {
        return args.clone();
    };
    let mut out = map.clone();
    out.remove("dry_run");
    out.remove("idempotency_key");
    Value::Object(out)
}

/// Map a successful [`MutateOutcome`] to an §0.1 [`Envelope`]. A patch
/// serialization fault surfaces as `E_INTERNAL` rather than a hollow
/// `ok:true` with `patch:[]`.
fn mutate_outcome_to_envelope(verb_id: &str, outcome: MutateOutcome) -> Envelope {
    match outcome {
        MutateOutcome::Applied {
            event_id,
            data,
            warnings,
            patch,
        }
        | MutateOutcome::Replayed {
            event_id,
            data,
            warnings,
            patch,
        } => match serialize_or_internal(verb_id, "patch", &patch) {
            Ok(patch_value) => Envelope::ok(data, patch_value, warnings, event_id.to_string()),
            Err(env) => *env,
        },
        MutateOutcome::NoOp {
            data,
            warnings,
            patch,
        } => match serialize_or_internal(verb_id, "patch", &patch) {
            // §0.1: read-only / no-op verbs return `event_id: ""`.
            Ok(patch_value) => Envelope::ok(data, patch_value, warnings, String::new()),
            Err(env) => *env,
        },
    }
}

/// Map a [`LifecycleError`] from a project-scoped `mutate_via_verb` /
/// `compute_via_verb` call to an §0.1 [`Envelope`].
fn lifecycle_error_to_envelope(verb_id: &str, err: &LifecycleError) -> Envelope {
    match err {
        LifecycleError::UnknownVerb { verb_id } => {
            Envelope::err("E_UNKNOWN_VERB", format!("unknown verb: {verb_id}"))
        }
        LifecycleError::VerbExecutionFailed { source, .. } => {
            Envelope::err(verb_error_code(source), format!("{verb_id}: {source}"))
        }
        // not unit-testable here: E_BUSY needs an `in_progress` idempotency
        // slot, which only exists while a concurrent first-call holds the
        // key between `start` and `complete`. The single-threaded dispatch
        // path completes/aborts the slot before returning, so a follow-up
        // call never observes `InProgress`. Covered by the idempotency
        // index's own concurrency tests, not the engine surface.
        LifecycleError::IdempotencyBusy { .. } => Envelope::Err {
            code: "E_BUSY".to_string(),
            message: err.to_string(),
            hint: Some("retry after the in-flight call with this key completes".to_string()),
            details: Some(json!({ "idempotency_state": "in_progress" })),
        },
        LifecycleError::IdempotencyConflict { .. } => {
            Envelope::err("E_IDEMPOTENCY_CONFLICT", err.to_string())
        }
        // not unit-testable here: every remaining variant (IO, backend,
        // apply, replay, canonicalize) is an engine-side I/O-class fault
        // that requires sabotaging the filesystem / event log mid-call;
        // no deterministic trigger exists through the public dispatch
        // surface. Maps to E_IO.
        other => Envelope::err("E_IO", format!("{verb_id}: {other}")),
    }
}

/// Map a [`crate::reconstructor::VerbError`] to its §0.7 code. `BadArgs`
/// and `InvariantViolation` are client-facing (`E_SCHEMA_VIOLATION` —
/// malformed args vs would-violate-§0.13). `Custom` is the engine-internal
/// escape hatch (patch-construction failures, data-envelope serialization
/// faults, the unreachable raw-mutate `NoOp` guard) — mapping it to
/// `E_INTERNAL` rather than `E_SCHEMA_VIOLATION` keeps an agent from
/// wasting a retry rewriting valid args when the fault is on our side.
fn verb_error_code(err: &crate::reconstructor::VerbError) -> &'static str {
    use crate::reconstructor::VerbError;
    match err {
        VerbError::BadArgs { .. } | VerbError::InvariantViolation { .. } => "E_SCHEMA_VIOLATION",
        // not unit-testable here: VerbError::Custom is the engine-internal
        // escape hatch (patch-construction / data-envelope serialization
        // faults). For the typed verb structs no input drives it, so there
        // is no deterministic E_INTERNAL trigger via the dispatch surface.
        VerbError::Custom(_) => "E_INTERNAL",
    }
}

// --- Lifecycle free-fn error → §0.1 envelope mappers ----------------
//
// Each free function documents the spec code its variants map to in its
// rustdoc; these mappers mirror that documentation 1:1.

fn create_error_to_envelope(err: &project_create::ProjectCreateError) -> Envelope {
    use project_create::ProjectCreateError as E;
    let (code, hint): (&str, Option<&str>) = match err {
        E::NameEmpty
        | E::NameTooLong { .. }
        | E::InvalidCanvas(_)
        | E::CanvasDimOutOfRange { .. }
        | E::InvalidFps { .. }
        | E::RelativeAt { .. } => ("E_SCHEMA_VIOLATION", None),
        E::ProjectExists { .. } => (
            "E_PROJECT_EXISTS",
            Some("choose a different name or destination"),
        ),
        E::Io(_) | E::LifecycleFailed(_) => ("E_IO", None),
    };
    Envelope::Err {
        code: code.to_string(),
        message: err.to_string(),
        hint: hint.map(str::to_string),
        details: None,
    }
}

fn open_error_to_envelope(err: &project_open::ProjectOpenError) -> Envelope {
    use project_open::ProjectOpenError as E;
    let code = match err {
        E::ProjectNotFound { .. } => "E_PROJECT_NOT_FOUND",
        E::SchemaViolation { .. } => "E_SCHEMA_VIOLATION",
        // not unit-testable here: E_SCHEMA_VERSION_UNSUPPORTED needs a
        // project.json whose schema_version this build does not support;
        // the engine never writes one, so triggering it would require
        // hand-forging an out-of-band file (out of scope for an engine
        // dispatch test). E_PROJECT_LOCKED is covered above.
        E::SchemaVersionUnsupported { .. } => "E_SCHEMA_VERSION_UNSUPPORTED",
        E::ProjectLocked { .. } => "E_PROJECT_LOCKED",
        E::Io(_) | E::LifecycleFailed(_) => "E_IO",
    };
    Envelope::err(code, err.to_string())
}

fn save_error_to_envelope(err: &project_save::ProjectSaveError) -> Envelope {
    use project_save::ProjectSaveError as E;
    let code = match err {
        E::ProjectNotFound { .. } => "E_PROJECT_NOT_FOUND",
        E::Io(_) | E::LifecycleFailed(_) => "E_IO",
    };
    Envelope::err(code, err.to_string())
}

fn close_error_to_envelope(err: &project_close::ProjectCloseError) -> Envelope {
    use project_close::ProjectCloseError as E;
    let code = match err {
        E::ProjectNotFound { .. } => "E_PROJECT_NOT_FOUND",
        E::SaveIo(_) | E::SaveLifecycle(_) => "E_IO",
    };
    Envelope::err(code, err.to_string())
}

fn duplicate_error_to_envelope(err: &project_duplicate::ProjectDuplicateError) -> Envelope {
    use project_duplicate::ProjectDuplicateError as E;
    let code = match err {
        E::NameEmpty | E::NameTooLong { .. } | E::RelativeAt { .. } => "E_SCHEMA_VIOLATION",
        E::DestinationExists { .. } => "E_PROJECT_EXISTS",
        E::SourceNotFound { .. } => "E_PROJECT_NOT_FOUND",
        E::SourceCorrupt { .. } | E::Io(_) => "E_IO",
    };
    Envelope::err(code, err.to_string())
}

fn forget_error_to_envelope(err: &project_forget::ProjectForgetError) -> Envelope {
    use project_forget::ProjectForgetError as E;
    let code = match err {
        E::ArgsIncompatible { .. } => "E_ARGS_INCOMPATIBLE",
        E::BadRange { .. } => "E_BAD_RANGE",
        E::ProjectNotFound { .. } => "E_PROJECT_NOT_FOUND",
        E::Io(_) => "E_IO",
    };
    Envelope::err(code, err.to_string())
}
