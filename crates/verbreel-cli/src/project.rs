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
use verbreel_agent::Session;
use verbreel_state::{ProjectId, default_registry, synthetic_empty_project};

use crate::ProjectCreateCmd;

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

/// `verbreel project create <workspace> --name <n> [--canvas WxH] [--fps N/D]`
/// — create a fresh project on disk under `workspace` and report it.
///
/// Routes through [`verbreel_agent::Session::create`] (the §2.1
/// `project.create` lifecycle), which places the project at
/// `<workspace>/<name>`.
///
/// # Errors
///
/// - `--fps` is not `<num>/<den>`;
/// - creation fails (a project of that name already exists under
///   `workspace`, bad canvas, IO error).
pub fn create(out: &mut dyn Write, cmd: &ProjectCreateCmd) -> anyhow::Result<i32> {
    let fps = cmd.fps.as_deref().map(parse_fps).transpose()?;
    let session = Session::create(&cmd.workspace, &cmd.name, &cmd.canvas, fps)?;
    let canvas = &session.project().canvas;
    writeln!(
        out,
        "created project {:?} at {}",
        cmd.name,
        session.root().display()
    )?;
    writeln!(out, "  id:     {}", session.project_id())?;
    writeln!(out, "  canvas: {}x{}", canvas.width, canvas.height)?;
    Ok(0)
}

/// Parse a `<num>/<den>` frame-rate literal.
fn parse_fps(s: &str) -> anyhow::Result<(u32, u32)> {
    let (num, den) = s
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("--fps must be <num>/<den>, e.g. 30/1"))?;
    let num = num
        .trim()
        .parse()
        .map_err(|e| anyhow::anyhow!("fps numerator: {e}"))?;
    let den = den
        .trim()
        .parse()
        .map_err(|e| anyhow::anyhow!("fps denominator: {e}"))?;
    Ok((num, den))
}
