//! Verb-id reconstruction and §0.12 project-context resolution.
//!
//! This is the routing brain between the parsed flags and
//! [`Engine::dispatch`]. It owns three decisions, all derived from data
//! the engine already exposes (no hardcoded verb lists):
//!
//! 1. **Verb-id reconstruction** — `<noun> <verb>` → `noun.verb`, except
//!    the flat meta verbs (`help`/`schema`/`describe`/`validate_command`/
//!    `list_capabilities`) which are single-token ids. The flat-vs-dotted
//!    decision is read from [`Engine::verb_ids`] (the ids with no `.`),
//!    not a duplicated whitelist.
//! 2. **Project context** (§0.12) — `--project` wins; else the
//!    active-project file; else unset. The resolved id is injected into
//!    the args as `project_id`.
//! 3. **Pre-open** — the engine's open-project map starts empty in the
//!    one-shot CLI, so a project-scoped verb needs its target project
//!    opened first. The CLI resolves `project_id → root` via
//!    [`verbreel_storage::layout::resolve_root_for_project_id`] and
//!    dispatches `project.open {path}` before the target verb. A
//!    resolution failure surfaces as `E_PROJECT_NOT_FOUND`.

use serde_json::{Map, Value};
use verbreel_state::{Engine, Envelope};

/// The five flat meta verbs are *project-less reads*; `project.create` /
/// `project.open` / `project.list` are lifecycle/global verbs that take
/// no caller `project_id`. None of these get a project context injected.
///
/// Derived membership: a verb is "flat" iff its id contains no `.` (read
/// from the engine). The dotted globals are pinned here because the
/// engine does not otherwise distinguish "needs an open project" from
/// "global lifecycle verb" — and §0.12 names these three explicitly as
/// the dotted verbs that take no `project_id`.
const DOTTED_GLOBAL_VERBS: [&str; 3] = ["project.create", "project.open", "project.list"];

/// Reconstruct the verb id from the leading positionals.
///
/// `noun` plus an optional `verb`. If `verb` is absent and `noun` is a
/// flat id the engine knows, the id is `noun` itself; otherwise it is
/// `format!("{noun}.{verb}")` (an absent `verb` yields `noun.` which the
/// engine rejects as `E_UNKNOWN_VERB` — the correct "bare noun" outcome).
#[must_use]
pub fn reconstruct_verb_id(engine: &Engine, noun: &str, verb: Option<&str>) -> String {
    match verb {
        Some(v) => format!("{noun}.{v}"),
        None => {
            if is_flat_verb(engine, noun) {
                noun.to_string()
            } else {
                // A lone non-flat token (`project`) is a bare noun with no
                // verb. Produce a dotted id with an empty verb so dispatch
                // returns E_UNKNOWN_VERB rather than silently matching a
                // flat id that does not exist.
                format!("{noun}.")
            }
        }
    }
}

/// `true` if `id` is a flat (dotless) verb the engine exposes.
fn is_flat_verb(engine: &Engine, id: &str) -> bool {
    !id.contains('.') && engine.verb_ids().iter().any(|known| known == id)
}

/// `true` if `verb_id` needs a project context injected and a pre-open
/// step: it is a dotted verb that is neither a flat meta verb nor one of
/// the dotted globals (§0.12).
#[must_use]
pub fn is_project_scoped(verb_id: &str) -> bool {
    verb_id.contains('.') && !DOTTED_GLOBAL_VERBS.contains(&verb_id)
}

/// Resolve the project id for a project-scoped verb, open its project
/// into the engine, and inject `project_id` into `args`.
///
/// Returns `Ok(())` on success (args mutated in place, project opened),
/// or `Err(Envelope)` carrying the §0.1 failure to print:
///
/// - no `--project` and no active-project file → `E_PROJECT_NOT_FOUND`
///   (this is also the §0.3 guard: a UUID prefix has no resolution scope
///   without a project, so the CLI fails here before any prefix attempt).
/// - resolver `NotFound` / I/O / invalid-index → `E_PROJECT_NOT_FOUND` /
///   `E_IO`.
/// - `project.open` itself fails → its own envelope.
///
/// # Errors
///
/// See above — every non-success path is a boxed [`Envelope`] for the
/// caller to print on stdout with exit 1.
pub fn ensure_project_context(
    engine: &mut Engine,
    home: Option<&std::path::Path>,
    explicit_project: Option<&str>,
    active_project: Option<String>,
    args: &mut Map<String, Value>,
) -> Result<(), Box<Envelope>> {
    let project_id = explicit_project
        .map(str::to_string)
        .or(active_project)
        .ok_or_else(|| {
            Box::new(project_not_found_envelope(
                "no project context: pass --project <id> or set an active project",
                None,
            ))
        })?;

    let Some(home) = home else {
        return Err(Box::new(project_not_found_envelope(
            "cannot resolve project root: no VERBREEL_HOME / HOME set",
            Some(&project_id),
        )));
    };

    let root = match verbreel_storage::layout::resolve_root_for_project_id(home, &project_id) {
        Ok(root) => root,
        Err(err) => return Err(Box::new(resolve_error_to_envelope(&err, &project_id))),
    };

    // Open the resolved project into the engine's map so the subsequent
    // project-scoped dispatch finds it (§0.8 persisted store).
    let open_args = serde_json::json!({ "path": root, "strict": false });
    let open_env = engine.dispatch("project.open", open_args);
    if !open_env.is_ok() {
        return Err(Box::new(open_env));
    }

    args.insert("project_id".to_string(), Value::String(project_id));
    Ok(())
}

/// Build an `E_PROJECT_NOT_FOUND` envelope, optionally with
/// `details.project_id`.
fn project_not_found_envelope(message: &str, project_id: Option<&str>) -> Envelope {
    Envelope::Err {
        code: "E_PROJECT_NOT_FOUND".to_string(),
        message: message.to_string(),
        hint: Some("create or open the project, or pass --project <id>".to_string()),
        details: project_id.map(|id| serde_json::json!({ "project_id": id })),
    }
}

/// Map a [`verbreel_storage::layout::ResolveError`] to a §0.1 envelope.
/// `NotFound` → `E_PROJECT_NOT_FOUND` (§0.12); the rest are I/O-class.
fn resolve_error_to_envelope(
    err: &verbreel_storage::layout::ResolveError,
    project_id: &str,
) -> Envelope {
    use verbreel_storage::layout::ResolveError;
    match err {
        ResolveError::NotFound(_) => project_not_found_envelope(
            &format!("project `{project_id}` is not registered"),
            Some(project_id),
        ),
        ResolveError::Io(_) | ResolveError::InvalidIndex { .. } => Envelope::Err {
            code: "E_IO".to_string(),
            message: format!("failed to resolve project `{project_id}`: {err}"),
            hint: None,
            details: None,
        },
    }
}
