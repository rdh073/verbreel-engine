//! [`AgentError`] — the unified failure surface for the AX layer.
//!
//! The AX layer composes three lower concerns — the engine kernel
//! ([`verbreel_state`] lifecycle + verb dispatch), capability discovery,
//! and natural-language planning — so its error type folds the kernel's
//! [`LifecycleError`] plus the planner's own failure modes into one enum
//! callers (CLI / HTTP / MCP) match on once.

use thiserror::Error;
use verbreel_state::{LifecycleError, VerbError};

/// Errors surfaced by [`crate::Session`] and the planner.
#[derive(Debug, Error)]
pub enum AgentError {
    /// A [`crate::Session`] kernel operation failed (open / create /
    /// mutate / save / idempotency). Wraps the kernel's own typed error
    /// so callers can still match on the precise lifecycle cause.
    #[error("engine: {0}")]
    Lifecycle(#[from] LifecycleError),

    /// A verb id passed to [`crate::Session::run`] is not in the kernel
    /// [`verbreel_state::default_registry`].
    ///
    /// Distinct from [`LifecycleError::UnknownVerb`]: this fires on the
    /// read-only *peek* path (before the engine's forward router is even
    /// consulted) so the dispatcher can reject an unknown verb without
    /// touching the event log.
    #[error("unknown verb: {verb}")]
    UnknownVerb {
        /// The verb id that failed registry lookup.
        verb: String,
    },

    /// A verb's [`verbreel_state::Verb::compute_patch`] rejected the args
    /// on the read-only peek path (bad arg shape, would-violate-§0.13).
    #[error("verb execution failed: {verb}: {source}")]
    VerbExecution {
        /// The verb id whose `compute_patch` failed.
        verb: String,
        /// The underlying verb-layer error.
        #[source]
        source: VerbError,
    },

    /// A [`crate::Plan`] step referenced a verb the kernel does not know,
    /// or carried an args value that was not a JSON object. Caught when a
    /// plan is validated against the capability catalog before any step
    /// runs, so a malformed plan never half-applies.
    #[error("invalid plan at step {index}: {detail}")]
    InvalidPlan {
        /// Zero-based index of the offending step.
        index: usize,
        /// Human-readable description of what was wrong.
        detail: String,
    },

    /// Creating a fresh project on disk failed (§2.1 `project.create`).
    ///
    /// The kernel's create path has its own error type
    /// (`ProjectCreateError`) distinct from [`LifecycleError`]; its
    /// human-readable detail is captured here rather than widening this
    /// enum with a kernel type that only one constructor produces.
    #[error("project create failed: {0}")]
    ProjectCreate(String),

    /// The planner could not turn the intent into a plan.
    #[error("planner failed: {0}")]
    Planner(#[from] PlannerError),
}

/// Errors surfaced by a [`crate::Planner`] implementation.
///
/// Kept separate from [`AgentError`] so the planning concern can be
/// tested and reused (HTTP `/agent`, CLI `edit --intent`) without
/// dragging in the kernel error surface, then folded into [`AgentError`]
/// via `#[from]` at the dispatch boundary.
#[derive(Debug, Error)]
pub enum PlannerError {
    /// The LLM transport itself failed (network, auth, non-2xx status,
    /// missing API key). Carries a human-readable detail.
    #[error("llm transport: {detail}")]
    Transport {
        /// Description of the transport failure.
        detail: String,
    },

    /// The model replied but its text did not contain a parseable JSON
    /// plan object. Carries the offending raw text (truncated by the
    /// caller if needed) for diagnosis.
    #[error("model reply was not a parseable plan: {detail}")]
    BadReply {
        /// Why the reply could not be parsed into a [`crate::Plan`].
        detail: String,
    },

    /// The required configuration for the live planner was absent — e.g.
    /// the `ANTHROPIC_API_KEY` environment variable is unset.
    #[error("planner not configured: {detail}")]
    NotConfigured {
        /// What configuration was missing and how to supply it.
        detail: String,
    },
}
