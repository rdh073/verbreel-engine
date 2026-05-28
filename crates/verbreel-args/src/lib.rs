//! verbreel-args — Per-verb args schemas + canonical JSON validator.
//!
//! This crate ships the **framework** that the rest of the engine uses
//! to validate verb arguments against per-verb JSON Schemas. It does
//! NOT yet ship concrete `*Args` types — those still live inline in
//! `verbreel-state::verbs::*` and migrate one batch at a time in
//! future slices.
//!
//! Per the crate dependency rule in `CLAUDE.md`:
//!
//! ```text
//! verbreel-args → verbreel-types
//! ```
//!
//! No imports from `verbreel-state` or `verbreel-events` are permitted
//! from inside this crate. Every primitive operates on
//! [`serde_json::Value`] + verb-name `&str`. Higher-level composition
//! (concrete `Args` types, dispatch through `engine::apply`) belongs
//! upstack.
//!
//! ## Modules
//!
//! - [`schema`] — [`Schema`](schema::Schema) newtype wrapping a raw
//!   JSON Schema value alongside a pre-compiled
//!   [`jsonschema::Validator`].
//! - [`registry`] — [`ArgsRegistry`](registry::ArgsRegistry), the
//!   `&'static str` → [`Schema`](schema::Schema) lookup table.
//! - [`validator`] — top-level
//!   [`validate`](validator::validate) entry point: verb-name lookup
//!   plus schema check, returning the first violation only.
//! - [`error`] — [`SchemaError`](error::SchemaError) (schema
//!   construction failures) and
//!   [`ValidationError`](error::ValidationError) (runtime args
//!   validation failures).
//! - [`well_known`] — hand-curated JSON Schemas for the simplest
//!   `*Args` shapes the engine already ships, plus a
//!   [`default_registry`](well_known::default_registry) factory.
//!   First proof of the framework end-to-end; subsequent slices add
//!   more verbs to the registry one batch at a time.
//!
//! ## Why pre-compile?
//!
//! Verb dispatch is on the hot path. Compiling a JSON Schema on every
//! `apply` call would dominate small-verb latency. The
//! [`Schema`](schema::Schema) constructor compiles once; the
//! [`registry::ArgsRegistry`] holds the compiled artifacts; the
//! [`validator::validate`] call site only runs the cheap match.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod error;
pub mod registry;
pub mod schema;
pub mod validator;
pub mod well_known;

pub use error::{SchemaError, ValidationError};
pub use registry::ArgsRegistry;
pub use schema::Schema;
pub use validator::validate;
pub use well_known::default_registry;
