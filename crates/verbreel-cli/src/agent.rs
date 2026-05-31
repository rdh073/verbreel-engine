//! Agentic-Experience CLI wrappers: `caps`, `run`, and `edit`.
//!
//! These route through [`verbreel_agent`] (the AX layer) rather than the
//! synthetic-prior pattern in [`crate::project`]: `run` and `edit` open a
//! *real* on-disk project [`Session`] and apply verbs through the §0.8
//! write-ordering kernel, so edits persist and undo.
//!
//! - [`caps`] — print the capability catalog (agent discovery).
//! - [`run`] — apply one verb to a project.
//! - [`edit`] — turn an intent (or a plan file) into a verb sequence and
//!   apply it: the headline agentic flow.

use std::io::Write;
use std::path::Path;

use serde_json::{Value, json};
use verbreel_agent::{Capabilities, Plan, RunOutcome, Session};

/// `verbreel caps [--by-domain]` — print the engine capability catalog.
///
/// Default output is the full JSON catalog (engine version, tick rate,
/// every verb with its args schema where known) — the surface an agent
/// reads to plan. `--by-domain` prints a compact grouped human view.
///
/// # Errors
///
/// Returns an error only if the catalog cannot be serialised to JSON.
pub fn caps(out: &mut dyn Write, by_domain: bool) -> anyhow::Result<i32> {
    let caps = Capabilities::current();
    if by_domain {
        let grouped = caps.by_domain();
        let domains = grouped.len();
        for (domain, ids) in &grouped {
            writeln!(out, "{} ({})", domain, ids.len())?;
            for id in ids {
                writeln!(out, "  {id}")?;
            }
        }
        writeln!(
            out,
            "\n{} verbs across {domains} domains",
            caps.verb_count()
        )?;
    } else {
        writeln!(out, "{}", serde_json::to_string_pretty(&caps)?)?;
    }
    Ok(0)
}

/// `verbreel run <root> <verb> [--args <json>] [--key <key>]` — apply one
/// verb to the project at `root` and print its `data` envelope.
///
/// Read-only verbs return their data with no event written; mutations
/// take the §0.8 forward path and the project snapshot is saved.
///
/// # Errors
///
/// - the project cannot be opened (missing / locked / corrupt);
/// - `--args` is not valid JSON;
/// - the verb is unknown or rejects the args;
/// - the save fails.
pub fn run(
    out: &mut dyn Write,
    root: &Path,
    verb: &str,
    args: Option<&str>,
    key: Option<String>,
) -> anyhow::Result<i32> {
    let args: Value = match args {
        Some(raw) => serde_json::from_str(raw)
            .map_err(|e| anyhow::anyhow!("--args is not valid JSON: {e}"))?,
        None => json!({}),
    };

    let mut session = Session::open(root)?;
    let outcome = session.run(verb, args, key)?;
    if outcome.mutated() {
        session.save()?;
    }
    writeln!(out, "{}", serde_json::to_string_pretty(outcome.data())?)?;
    Ok(0)
}

/// `verbreel edit <root> (--intent "…" | --plan <file>) [--dry-run]` —
/// the headline agentic flow: obtain a verb plan, then apply it.
///
/// The plan comes from one of two sources:
/// - `--plan <file>`: a pre-authored JSON plan (works in every build —
///   no LLM, fully deterministic);
/// - `--intent "<goal>"`: the Claude planner turns natural language into
///   a plan (requires the `claude` build feature + `ANTHROPIC_API_KEY`).
///
/// `--dry-run` prints the plan without applying it.
///
/// # Errors
///
/// - neither (or both) of `--intent` / `--plan` supplied;
/// - the plan file is missing or not valid JSON / not a plan;
/// - the planner transport fails (intent path);
/// - applying a step fails at the kernel.
pub fn edit(
    out: &mut dyn Write,
    root: &Path,
    intent: Option<&str>,
    plan_path: Option<&Path>,
    dry_run: bool,
) -> anyhow::Result<i32> {
    let caps = Capabilities::current();
    let plan = resolve_plan(intent, plan_path, &caps)?;

    writeln!(out, "plan: {} step(s)", plan.len())?;
    for (i, step) in plan.steps.iter().enumerate() {
        writeln!(
            out,
            "  {}. {} {}",
            i + 1,
            step.verb,
            serde_json::to_string(&step.args).unwrap_or_default()
        )?;
    }
    if let Some(rationale) = &plan.rationale {
        writeln!(out, "rationale: {rationale}")?;
    }

    if dry_run {
        writeln!(out, "(dry-run: no changes applied)")?;
        return Ok(0);
    }

    let mut session = Session::open(root)?;
    let results = session.apply_plan(&plan, &caps)?;
    for result in &results {
        writeln!(
            out,
            "  applied {} -> {}",
            result.verb,
            outcome_summary(&result.outcome)
        )?;
    }
    writeln!(
        out,
        "done: {} step(s) applied, saved to {}",
        results.len(),
        session.root().display()
    )?;
    Ok(0)
}

/// Resolve the plan from exactly one of `intent` / `plan_path`.
fn resolve_plan(
    intent: Option<&str>,
    plan_path: Option<&Path>,
    caps: &Capabilities,
) -> anyhow::Result<Plan> {
    match (intent, plan_path) {
        (Some(_), Some(_)) => {
            anyhow::bail!("pass only one of --intent or --plan")
        }
        (None, None) => {
            anyhow::bail!("provide --intent \"<goal>\" or --plan <file.json>")
        }
        (None, Some(path)) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| anyhow::anyhow!("reading plan {}: {e}", path.display()))?;
            let value: Value = serde_json::from_str(&text)
                .map_err(|e| anyhow::anyhow!("plan {} is not valid JSON: {e}", path.display()))?;
            Ok(Plan::from_json(&value)?)
        }
        (Some(intent), None) => plan_from_intent(intent, caps),
    }
}

/// Turn an intent into a plan via the Claude planner (feature `claude`).
#[cfg(feature = "claude")]
fn plan_from_intent(intent: &str, caps: &Capabilities) -> anyhow::Result<Plan> {
    use verbreel_agent::{AnthropicClient, LlmPlanner, Planner};
    let client = AnthropicClient::from_env()?;
    let planner = LlmPlanner::new(client);
    Ok(planner.plan(intent, caps)?)
}

/// Feature-off stub: `--intent` needs the Claude planner.
#[cfg(not(feature = "claude"))]
fn plan_from_intent(_intent: &str, _caps: &Capabilities) -> anyhow::Result<Plan> {
    anyhow::bail!(
        "--intent needs the Claude planner: rebuild with `--features claude` and set \
         ANTHROPIC_API_KEY, or supply a ready-made plan with `--plan <file.json>`"
    )
}

/// One-line summary of a step's outcome for the apply log.
fn outcome_summary(outcome: &RunOutcome) -> String {
    match outcome {
        RunOutcome::Query { .. } => "query (no change)".to_string(),
        RunOutcome::Mutated { event_id, .. } => format!("applied (event {event_id})"),
        RunOutcome::Replayed { event_id, .. } => format!("replayed (event {event_id})"),
    }
}
