//! `verbreel project ...` CLI wrappers.
//!
//! Each function here:
//!
//! 1. Looks up the matching verb in
//!    [`verbreel_state::default_registry`].
//! 2. Builds the args JSON value the verb expects.
//! 3. Calls `Verb::compute_patch(&prior, &args)` with a project-agnostic
//!    synthetic prior project (via
//!    [`verbreel_state::synthetic_empty_project`]).
//! 4. Writes the verb's `data` envelope as pretty-printed JSON.
//! 5. Returns the process exit code.

use std::io::Write;

use serde_json::json;
use verbreel_state::{ProjectId, default_registry, synthetic_empty_project};

/// `verbreel project list` — print the `project.list` data envelope.
///
/// v1 floor: the verb returns an empty `projects` array (see
/// `verbreel_state::verbs::project_list` for the spec rationale). This
/// wrapper still routes through the registry so the dispatch shape is
/// identical to every later subcommand.
///
/// # Errors
///
/// Returns an error if the `project.list` verb is missing from the
/// default registry, if the dispatcher refuses the args, or if the
/// returned `data` value cannot be serialised back to JSON.
pub fn list(out: &mut dyn Write) -> anyhow::Result<i32> {
    let registry = default_registry();
    let verb = registry
        .get("project.list")
        .ok_or_else(|| anyhow::anyhow!("project.list verb not found in default registry"))?;

    // project.list is project-agnostic at v1 — args still need a
    // project_id to clear the Verb trait's argument shape (see the
    // ProjectListArgs doc comment in verbreel-state).
    let project_id = ProjectId::now();
    let args = json!({ "project_id": project_id });
    let prior = synthetic_empty_project(project_id);

    let (_patch, data, _warnings) = verb.compute_patch(&prior, &args)?;
    let rendered = serde_json::to_string_pretty(&data)?;
    writeln!(out, "{rendered}")?;
    Ok(0)
}
