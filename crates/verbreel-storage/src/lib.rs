//! verbreel-storage — Filesystem CAS, OPFS shim, file layout.
//!
//! Type-agnostic POSIX / CAS primitives shared by `verbreel-state`
//! (project save/load) and downstream slices (asset CAS, render-queue
//! persistence, OPFS shim).
//!
//! Per the crate dependency rule in `CLAUDE.md`:
//!
//! ```text
//! verbreel-storage → verbreel-types, verbreel-events
//! ```
//!
//! No imports from `verbreel-state` are permitted from inside this
//! crate — every primitive operates on `&Path`, `&[u8]`, or hex
//! strings. Higher-level composition (Project, Asset, …) belongs
//! upstack.
//!
//! ## Modules
//!
//! - [`fs`] — atomic file write via `NamedTempFile` + `persist`.
//! - [`flock`] — RAII exclusive `flock(2)` wrapper.
//! - [`cas`] — content-addressed asset paths and streaming SHA-256.
//! - [`layout`] — project-root directory init + projects-index helpers.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod cas;
pub mod flock;
pub mod fs;
pub mod layout;
