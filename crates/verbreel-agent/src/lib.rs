//! verbreel-agent — the Agentic Experience (AX) layer.
//!
//! This crate turns the headless verbreel engine into something an agent
//! can drive end-to-end. It composes the engine kernel into three
//! agent-facing primitives:
//!
//! - [`Session`] — an editing session bound to one on-disk project. The
//!   single front door the CLI / HTTP / MCP surfaces route verb calls
//!   through. Wraps [`verbreel_state::ProjectStore`], routes read-only
//!   verbs around the event log, injects `project_id`, and applies whole
//!   [`Plan`]s. ([`session`])
//! - [`Capabilities`] — the agent-discovery catalog: every one of the
//!   engine's registered verbs, grouped by domain, with the per-verb JSON
//!   args schema where one is registered. Richer than the v1-floor
//!   `list_capabilities` verb (which carries empty summaries/schema ids).
//!   ([`capabilities`])
//! - [`Planner`] — turns a natural-language intent into a validated
//!   [`Plan`]. The logic is transport-agnostic and unit-tested against a
//!   mock; the live Anthropic Messages-API leg
//!   ([`anthropic::AnthropicClient`]) is behind the `claude` feature.
//!   ([`planner`])
//!
//! ## Crate dependency rule
//!
//! ```text
//! verbreel-agent → verbreel-state, verbreel-storage, verbreel-args
//!   [feature "claude"] + reqwest
//! ```
//!
//! `verbreel-agent` is a composition layer: it sits *above* the kernel
//! and *below* the surface binaries (cli / mcp / http), which depend on
//! it. It adds no cycle — every dependency is strictly lower in the tree.
//! The natural-language planner lives here (not in `verbreel-state`)
//! precisely because LLM transport is an application concern the kernel
//! must never take on, and because `verbreel-state` may not depend on a
//! network client.
//!
//! ## Pluggable planning
//!
//! The planner is an *optional* convenience. Over MCP, the connected
//! agent (e.g. Claude itself) is the planner — it discovers verbs via
//! [`Capabilities`] and drives [`Session`] directly, so no built-in
//! planner is needed. The [`Planner`] trait + [`anthropic::AnthropicClient`]
//! exist for the *standalone* path (`verbreel edit --intent "…"`), where
//! the tool itself must do the planning.

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
// Verb ids and domain names mirror the spec/engine vocabulary; the
// `Session`/`Capabilities` types live in same-named modules and that is
// intentional, matching the kernel crate's own convention.
#![allow(clippy::module_name_repetitions)]

#[cfg(feature = "claude")]
pub mod anthropic;
pub mod capabilities;
pub mod error;
pub mod plan;
pub mod planner;
pub mod session;

pub use capabilities::{Capabilities, VerbInfo};
pub use error::{AgentError, PlannerError};
pub use plan::{Plan, VerbCall};
pub use planner::{LlmClient, LlmPlanner, Planner, build_system_prompt};
pub use session::{RunOutcome, Session, StepResult};

#[cfg(feature = "claude")]
pub use anthropic::AnthropicClient;
