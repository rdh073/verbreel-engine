//! verbreel-render — composition pipeline.
//!
//! v0 ships the type surface that the eventual wgpu + WGSL render
//! path will satisfy. Every method body returns
//! [`RenderError::NotYetImplemented`] until Spike S1 lands the real
//! pipeline (Research 01 §11).
//!
//! ## Why ship types now
//!
//! - Closes the workspace exit-condition counter (was a 1-line stub).
//! - Lets downstream callers (`verbreel-codec-native`, `verbreel-cli`,
//!   `verbreel-mcp`) hold a stable Rust surface to call into;
//!   behaviour arrives at S1 by body replacement, not API change.
//! - The two-preset model ([`RenderPreset::Deterministic`] vs
//!   [`RenderPreset::Performance`]) is pinned at the type level so
//!   future spike work cannot silently regress the §11.1 / §11.2
//!   determinism vs throughput split.
//!
//! ## Crate dependency rule
//!
//! ```text
//! verbreel-render → verbreel-ir
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]

pub mod error;
pub mod hwaccel;
pub mod pipeline;
pub mod preset;

pub use error::RenderError;
pub use hwaccel::{HwAccelKind, V1_HWACCEL_PRIORITY, hwaccel_priority_for_preset};
pub use pipeline::{Pipeline, run};
pub use preset::RenderPreset;
