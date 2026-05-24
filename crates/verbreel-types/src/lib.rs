//! verbreel-types — shared serde types (no logic, no IO).
//!
//! Every other Verbreel crate imports from here. This crate is a structural
//! foundation; it MUST stay logic-free and IO-free. Adding business logic
//! to it would break the §0.13 invariant that the types module is a pure
//! data shape.
//!
//! Spec references:
//! - §0.2 time and ticks → [`Tick`], [`TickRate`], [`TICK_RATE_HZ`]
//! - §0.3 IDs → [`UuidV7`], [`ProjectId`], [`ClipId`], [`TrackId`], [`AssetId`], [`EventId`], [`MarkerId`], [`TrackerId`], [`LinkGroupId`], [`EffectId`], [`KeyframeId`]
//! - §3.1 asset content addressing → [`AssetHash`]

#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)] // names match spec §IDs intentionally

pub mod hash;
pub mod id;
pub mod tick;

pub use hash::AssetHash;
pub use id::{
    AssetId, ClipId, EffectId, EventId, KeyframeId, LinkGroupId, MarkerId, ProjectId, TextTrackId,
    TrackId, TrackerId, UuidV7,
};
pub use tick::{TICK_RATE_HZ, Tick, TickRate};
