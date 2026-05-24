//! `project.set_metadata` (§2.12) — first real verb in the engine.
//!
//! ## Spec quote (`spec/commands/project.md` §2.12, verbatim)
//!
//! > Updates `Project.metadata`. The metadata bag is free-form
//! > (`additionalProperties: true` per schema). This verb is a
//! > **shallow merge** by default: supplied keys overwrite existing
//! > top-level keys; absent keys are left unchanged. Passing
//! > `replace: true` (CLI: `--replace`) discards the existing metadata
//! > entirely before applying the new object. Passing a key with value
//! > `null` (MCP/HTTP) or `--unset <key>` (CLI, repeatable) removes
//! > that top-level key.
//! >
//! > **Args**: `project_id: string`, `metadata?: object`,
//! > `replace?: boolean` (default `false`), `unset?: string[]`
//! > (top-level keys to remove; `maxItems: 256`). At least one of
//! > `metadata` or `unset` must be supplied (else
//! > `E_ARGS_INCOMPATIBLE`).
//! >
//! > **Returns** (`data`): `{ project_id: string; metadata: object }` —
//! > the new full metadata object after the merge/replace/unset.
//! >
//! > **Errors**: `E_ARGS_INCOMPATIBLE` (`replace: true` combined with
//! > `unset`; or both `metadata` and `unset` omitted),
//! > `E_SCHEMA_VIOLATION` (`metadata` is not an object; `unset`
//! > exceeds `maxItems: 256`; resulting `metadata` exceeds the §0.13
//! > 256-keys / 64 KiB cap).
//!
//! ## Null-value semantics
//!
//! Per §2.12 and the `spec/project-schema.json` "Null-value semantics
//! divergence" note: when `args.metadata` carries a key whose value is
//! `null`, the verb interprets that as **remove this top-level key**
//! rather than store a `null` value. The divergence is a deliberate
//! cost — a hand-edited project that legitimately stores a `null`
//! cannot round-trip through `set_metadata` without losing the entry.
//! Persist `null` only by hand-editing project.json directly or via a
//! sentinel string the agent layer interprets.
//!
//! Null-removal applies in both modes:
//!
//! - **Merge mode** (default): `null` values in `args.metadata` remove
//!   the named key from the prior metadata.
//! - **Replace mode** (`replace: true`): the prior metadata is
//!   discarded first; `null`-valued entries in `args.metadata` are
//!   simply skipped when building the new map (they cannot "remove"
//!   anything because nothing precedes them, but they MUST NOT land
//!   in the result either).
//!
//! ## Order of operations
//!
//! Given `(prior, args)`:
//!
//! 1. Seed `new_metadata`:
//!    - `replace: true` → start from an empty `Map`.
//!    - `replace: false` → clone `prior.metadata` (default merge mode).
//! 2. Apply `args.unset[]` (always allowed when present and not
//!    combined with `replace: true`): remove every named key from
//!    `new_metadata`. Removal of an absent key is a no-op.
//! 3. Apply `args.metadata` (each `(k, v)` pair in declaration order):
//!    - `v == null` → remove `k` from `new_metadata` (no-op if absent).
//!    - any other value → insert / overwrite `(k, v)`.
//! 4. Validate the §0.13 caps against `new_metadata`:
//!    - key count ≤ [`crate::invariants::METADATA_MAX_KEYS`] (256).
//!    - compact-serialized JSON byte length ≤
//!      [`crate::invariants::METADATA_MAX_BYTES`] (65 536).
//!
//!    Key count is checked first because it's cheaper (single
//!    `Map::len()`); the byte cap requires a `serde_json::to_vec` over
//!    the whole map. On a tie the cheaper error fires.
//! 5. Emit the patch as a single `replace` op on `/metadata`. Per the
//!    §0.8 record-of-truth contract the verb could emit a finer-grained
//!    per-key add/remove/replace op list, but the engine reconstructs
//!    the same final state either way and the wholesale-replace form
//!    is simpler to audit. Per-key minimization is a future
//!    micro-optimization not required for §0.8 correctness.
//!
//! ## Args-validation order
//!
//! Argument-shape errors are checked BEFORE any computation, in this
//! order:
//!
//! 1. `replace: true` combined with a non-`None` `unset` →
//!    [`ProjectSetMetadataError::ArgsIncompatibleReplaceAndUnset`].
//! 2. Both `metadata` and `unset` omitted (`None`) →
//!    [`ProjectSetMetadataError::ArgsIncompatibleNeitherMetadataNorUnset`].
//! 3. `unset.len() > 256` →
//!    [`ProjectSetMetadataError::UnsetTooLong`].
//!
//! Cap errors (key count / byte size) are computed after the merge —
//! the verb has to build the candidate `new_metadata` to know whether
//! it would exceed the caps.
//!
//! ## Reconstructor purity (§0.8)
//!
//! The envelope `data` field is `{ project_id, metadata }` where:
//!
//! - `project_id` is `args.project_id` (recorded verbatim in the
//!   event's `args` slot).
//! - `metadata` is the post-state `Project.metadata` (the value
//!   `compute_patch` wrote on the patch's `replace` op, applied by the
//!   kernel to produce the post-state).
//!
//! Both fields are derivable from `(args, post_state)` alone — no
//! `randomUUID()`, no wall-clock, no patch inspection, no
//! `warnings.details` escape hatch needed. The
//! [`ProjectSetMetadataReconstructor`] impl exercises this directly
//! and is registered against [`crate::validate_reconstructors`] in the
//! crate's test suite to lock the round-trip.
//!
//! ## Out of scope (this slice)
//!
//! - No `ProjectStore::mutate()` wiring. The verb is freestanding;
//!   integration into the kernel (event-log write → patch apply → data
//!   envelope return) lands in Slice B2/B3.
//! - No `W_TAG_RESERVED_NAMESPACE` (Appendix B) — that's a tag-
//!   validation micro-feature scoped to a future slice; the
//!   `Project.metadata` bag is free-form here.
//! - No `verbreel-args` schema crate population. Serde derive on the
//!   args struct is sufficient for this slice; the JSON-schema
//!   declaration belongs to that crate's future work.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::invariants::{METADATA_MAX_BYTES, METADATA_MAX_KEYS};
use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Maximum entries allowed in `args.unset[]`. Mirrors
/// `Project.metadata`'s [`METADATA_MAX_KEYS`] cap — the largest legal
/// `unset` array can address every top-level key. Per §2.12.
pub const UNSET_MAX_ITEMS: usize = METADATA_MAX_KEYS;

/// Args for `project.set_metadata`. Mirrors the §2.12 args list.
///
/// `metadata` is held as a `serde_json::Map<String, Value>` so the
/// declaration order of input keys is preserved (matters for the
/// shallow-merge / null-removal walk in
/// [`compute_patch`]). `unset` is `Option<Vec<String>>` so the
/// "both omitted" args-incompatible case is distinguishable from the
/// "supplied but empty" case (the latter is a degenerate no-op, the
/// former is a hard error).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSetMetadataArgs {
    /// Target project id. Strongly-typed [`ProjectId`] so a verb-args
    /// payload carrying a non-UUID-v7 string fails serde deserialize
    /// rather than reaching `compute_patch`.
    pub project_id: ProjectId,

    /// Shallow-merge payload. Optional — if `None`, `unset` MUST be
    /// supplied. `null`-valued entries are treated as "remove this
    /// top-level key" per the null-value semantics divergence (§2.12 +
    /// `spec/project-schema.json`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,

    /// Wipe `Project.metadata` to `{}` before applying `metadata`.
    /// Default `false` (merge mode). Combining `replace: true` with a
    /// non-`None` `unset` is a hard error
    /// ([`ProjectSetMetadataError::ArgsIncompatibleReplaceAndUnset`]).
    #[serde(default)]
    pub replace: bool,

    /// Top-level keys to remove before the merge. Optional — if `None`,
    /// `metadata` MUST be supplied. Per §2.12 the array is bounded to
    /// [`UNSET_MAX_ITEMS`] (256 entries).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unset: Option<Vec<String>>,
}

/// Envelope `data` shape returned by `project.set_metadata`. Per
/// §2.12: the FULL post-merge metadata bag plus the target project id.
///
/// `metadata` is `Map<String, Value>` so the serialized JSON is an
/// object (matching the spec's `metadata: object` declaration), not a
/// sentinel `null` or array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSetMetadataData {
    /// The target project id (echoed from `args.project_id`).
    pub project_id: ProjectId,
    /// The full post-merge `Project.metadata` bag (after replace,
    /// unset, and the null-removal walk).
    pub metadata: Map<String, Value>,
}

/// Verb-level errors surfaced by [`compute_patch`]. Maps onto §2.12's
/// `E_ARGS_INCOMPATIBLE` and `E_SCHEMA_VIOLATION` once wired into the
/// kernel error-translation layer (Slice B2/B3).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProjectSetMetadataError {
    /// `replace: true` was combined with a non-`None` `unset`. Per
    /// §2.12 the two are mutually exclusive — a `replace` wipes the
    /// prior metadata wholesale, so naming individual keys to remove
    /// would be redundant at best and contradictory at worst.
    /// Surfaces as `E_ARGS_INCOMPATIBLE`.
    #[error("project.set_metadata: `replace: true` cannot be combined with `unset`")]
    ArgsIncompatibleReplaceAndUnset,

    /// Both `metadata` and `unset` were omitted. Per §2.12 at least
    /// one must be supplied — a verb call with neither is a no-op
    /// request, which the spec rejects rather than silently accepting.
    /// Surfaces as `E_ARGS_INCOMPATIBLE`.
    #[error("project.set_metadata: at least one of `metadata` or `unset` must be supplied")]
    ArgsIncompatibleNeitherMetadataNorUnset,

    /// `unset` carried more than [`UNSET_MAX_ITEMS`] entries. Per
    /// §2.12 the array is bounded to 256 (matching
    /// `Project.metadata`'s `maxProperties` so the largest legal
    /// `unset` can still address every key). Surfaces as
    /// `E_SCHEMA_VIOLATION` with `details.field: "unset"`.
    #[error("project.set_metadata: `unset` has {actual} entries, exceeds cap of {cap}")]
    UnsetTooLong {
        /// Actual entry count from the input.
        actual: usize,
        /// Cap ([`UNSET_MAX_ITEMS`]).
        cap: usize,
    },

    /// `metadata` was present in the JSON args but did not deserialize
    /// to an object. Surfaces as `E_SCHEMA_VIOLATION` with
    /// `details.field: "metadata"`.
    ///
    /// **Unreachable** for callers that supply a typed
    /// [`ProjectSetMetadataArgs`] (serde already enforces the object
    /// shape at deserialize time, surfacing as a serde error not this
    /// variant). Included for completeness so callers that hand-route
    /// pre-typed JSON to the verb layer can produce the same
    /// taxonomy.
    #[error("project.set_metadata: `metadata` is not an object")]
    MetadataNotObject,

    /// Post-merge `new_metadata` would exceed the §0.13 key-count cap.
    /// Surfaces as `E_SCHEMA_VIOLATION` with `details.field:
    /// "/metadata"`, `details.cap_keys: 256`, `details.size_keys:
    /// <actual>`.
    #[error("project.set_metadata: post-merge metadata has {actual} keys, exceeds cap of {cap}")]
    KeysOverCap {
        /// Actual key count of the post-merge map.
        actual: usize,
        /// Cap ([`METADATA_MAX_KEYS`]).
        cap: usize,
    },

    /// Post-merge `new_metadata` would exceed the §0.13 serialized-byte
    /// cap. Surfaces as `E_SCHEMA_VIOLATION` with `details.field:
    /// "/metadata"`, `details.cap_bytes: 65536`, `details.size_bytes:
    /// <actual>`.
    ///
    /// Computed via `serde_json::to_vec(&new_metadata).len()` — the
    /// storage-bytes measure (NOT
    /// [`verbreel_canon::canonicalize`]; the cap is on bytes-on-the-
    /// wire / bytes-on-disk, which for ASCII keys is typically equal
    /// to the canonical form but can diverge under unicode
    /// normalization).
    #[error(
        "project.set_metadata: post-merge metadata serializes to {actual} bytes, exceeds cap of {cap}"
    )]
    BytesOverCap {
        /// Actual serialized byte length.
        actual: usize,
        /// Cap ([`METADATA_MAX_BYTES`]).
        cap: usize,
    },
}

/// Compute the RFC 6902 patch and post-merge metadata for a
/// `project.set_metadata` call.
///
/// Pure function — no I/O, no clock, no RNG. Builds the post-merge
/// `Map<String, Value>` per the order-of-operations documented at the
/// module level, validates the §0.13 caps, and returns:
///
/// - the patch as a `serde_json::Value` (an RFC 6902 op array with a
///   single `replace` op on `/metadata`), and
/// - the post-merge `Map<String, Value>` (the value of the patch's
///   `replace` op, returned separately so the caller doesn't have to
///   walk the patch to recover it).
///
/// The patch's wholesale-replace form means a downstream kernel call
/// to `Project::apply` will reach the [`crate::invariants::check_metadata_caps`]
/// post-condition with `Project.metadata == new_metadata` — and that
/// post-condition runs the same checks as this function, so the kernel
/// path is doubly-defended (rejection here is the loud verb-layer
/// surface; rejection there is the safety net against verb-author
/// bugs that bypass the helper).
///
/// # Errors
///
/// Returns [`ProjectSetMetadataError`]:
///
/// - [`ProjectSetMetadataError::ArgsIncompatibleReplaceAndUnset`] when
///   `args.replace == true` and `args.unset.is_some()`.
/// - [`ProjectSetMetadataError::ArgsIncompatibleNeitherMetadataNorUnset`]
///   when both `args.metadata` and `args.unset` are `None`.
/// - [`ProjectSetMetadataError::UnsetTooLong`] when
///   `args.unset.as_ref().map(Vec::len) > Some(UNSET_MAX_ITEMS)`.
/// - [`ProjectSetMetadataError::KeysOverCap`] when the post-merge map
///   exceeds [`METADATA_MAX_KEYS`].
/// - [`ProjectSetMetadataError::BytesOverCap`] when the post-merge map
///   exceeds [`METADATA_MAX_BYTES`] under compact serialization.
pub fn compute_patch(
    prior: &Project,
    args: &ProjectSetMetadataArgs,
) -> Result<(Value, Map<String, Value>), ProjectSetMetadataError> {
    // Args-shape validation. Cheapest checks first; no allocation
    // until we know the args are well-formed.
    if args.replace && args.unset.is_some() {
        return Err(ProjectSetMetadataError::ArgsIncompatibleReplaceAndUnset);
    }
    if args.metadata.is_none() && args.unset.is_none() {
        return Err(ProjectSetMetadataError::ArgsIncompatibleNeitherMetadataNorUnset);
    }
    if let Some(unset) = args.unset.as_ref()
        && unset.len() > UNSET_MAX_ITEMS
    {
        return Err(ProjectSetMetadataError::UnsetTooLong {
            actual: unset.len(),
            cap: UNSET_MAX_ITEMS,
        });
    }

    // Seed: empty map for `replace: true`, prior metadata for merge.
    let mut new_metadata: Map<String, Value> = if args.replace {
        Map::new()
    } else {
        prior.metadata.clone()
    };

    // Apply unset[]. Mutually exclusive with `replace: true` per the
    // earlier check, so this branch only runs in merge mode.
    if let Some(unset) = args.unset.as_ref() {
        for key in unset {
            new_metadata.shift_remove(key);
        }
    }

    // Apply metadata. `null` values remove the named key; everything
    // else overwrites/inserts. Iterates in declaration order so the
    // resulting key order is deterministic for a given input.
    if let Some(metadata) = args.metadata.as_ref() {
        for (k, v) in metadata {
            if v.is_null() {
                new_metadata.shift_remove(k);
            } else {
                new_metadata.insert(k.clone(), v.clone());
            }
        }
    }

    // §0.13 caps. Key count first (single `len()`), then bytes
    // (allocates the serialized form). On a tie the cheaper error
    // fires.
    let key_count = new_metadata.len();
    if key_count > METADATA_MAX_KEYS {
        return Err(ProjectSetMetadataError::KeysOverCap {
            actual: key_count,
            cap: METADATA_MAX_KEYS,
        });
    }
    let bytes = serde_json::to_vec(&new_metadata).map_or(usize::MAX, |v| v.len());
    if bytes > METADATA_MAX_BYTES {
        return Err(ProjectSetMetadataError::BytesOverCap {
            actual: bytes,
            cap: METADATA_MAX_BYTES,
        });
    }

    let patch = json!([
        { "op": "replace", "path": "/metadata", "value": Value::Object(new_metadata.clone()) }
    ]);

    Ok((patch, new_metadata))
}

/// Build the verb's envelope `data` from `(args, post_state)`. Pure —
/// this is the function the reconstructor exercises during replay.
///
/// Per §0.8 reconstructor purity, every field on
/// [`ProjectSetMetadataData`] is derivable from the recorded inputs:
///
/// - `project_id`: cloned from `args.project_id`.
/// - `metadata`: cloned from `post_state.metadata` (the value the
///   patch wrote during the original execution; the kernel replays
///   the patch to produce the same `post_state` before calling here).
#[must_use]
pub fn data_envelope(
    args: &ProjectSetMetadataArgs,
    post_state: &Project,
) -> ProjectSetMetadataData {
    ProjectSetMetadataData {
        project_id: args.project_id,
        metadata: post_state.metadata.clone(),
    }
}

/// Funnel [`ProjectSetMetadataError`] into the verb-layer
/// [`VerbError`] taxonomy. Argument-shape errors map to
/// [`VerbError::BadArgs`]; cap / §0.13-invariant errors map to
/// [`VerbError::InvariantViolation`]. Used by
/// [`<ProjectSetMetadataVerb as Verb>::compute_patch`] to propagate
/// typed verb errors out of the kernel routing layer.
impl From<ProjectSetMetadataError> for VerbError {
    fn from(value: ProjectSetMetadataError) -> Self {
        match value {
            ProjectSetMetadataError::ArgsIncompatibleReplaceAndUnset
            | ProjectSetMetadataError::ArgsIncompatibleNeitherMetadataNorUnset
            | ProjectSetMetadataError::MetadataNotObject => VerbError::BadArgs {
                detail: value.to_string(),
            },
            ProjectSetMetadataError::UnsetTooLong { .. }
            | ProjectSetMetadataError::KeysOverCap { .. }
            | ProjectSetMetadataError::BytesOverCap { .. } => VerbError::InvariantViolation {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `project.set_metadata`. Registered in a
/// [`crate::VerbRegistry`] so the §0.8 startup gate
/// ([`crate::validate_reconstructors`]) can exercise its `reconstruct`
/// path against a recorded fixture, and so
/// [`crate::lifecycle::ProjectStore::mutate_via_verb`] can route
/// forward calls through its `compute_patch` path.
///
/// Pure on both legs of the trait — no I/O, no clock, no RNG, no
/// patch / warnings inspection during reconstruct. The forward leg
/// (`compute_patch`) deserialises `args` into [`ProjectSetMetadataArgs`],
/// calls the freestanding [`compute_patch`] helper, and converts the
/// resulting `Value` patch into a typed [`json_patch::Patch`].
///
/// Slice B3 rename: was `ProjectSetMetadataReconstructor` in Slices B1
/// / B2. The old name is kept as a `#[deprecated]` alias for one slice
/// cycle to ease downstream migration.
#[derive(Debug, Default)]
pub struct ProjectSetMetadataVerb;

/// Deprecated alias for [`ProjectSetMetadataVerb`] — kept for one slice
/// cycle while downstream callers migrate to the new name.
#[deprecated(since = "0.0.0", note = "use `ProjectSetMetadataVerb`")]
pub use ProjectSetMetadataVerb as ProjectSetMetadataReconstructor;

impl Verb for ProjectSetMetadataVerb {
    fn verb(&self) -> &'static str {
        "project.set_metadata"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        // Deserialize the raw JSON args into the typed struct. A serde
        // failure here is a [`VerbError::BadArgs`] — the args payload
        // is malformed (wrong shape, missing required fields, wrong
        // types).
        let typed: ProjectSetMetadataArgs =
            serde_json::from_value(args.clone()).map_err(|e| VerbError::BadArgs {
                detail: format!("project.set_metadata: args deserialize failed: {e}"),
            })?;

        // Run the freestanding compute_patch helper. Its typed error
        // funnels through the From impl above.
        let (patch_value, new_metadata) = compute_patch(prior, &typed)?;

        // Convert the RFC 6902 patch from `serde_json::Value` to the
        // typed `json_patch::Patch`. A failure here is a verb-author
        // bug — `compute_patch` produces a well-formed op array by
        // construction — so it surfaces as [`VerbError::Custom`].
        let patch: json_patch::Patch = serde_json::from_value(patch_value).map_err(|e| {
            VerbError::Custom(format!(
                "project.set_metadata: patch construction failed: {e}"
            ))
        })?;

        // Build the data envelope. `data_envelope` reads only
        // `(args.project_id, post_state.metadata)`; we synthesise the
        // post-state by cloning `prior` and overwriting `metadata`
        // with the value the patch will install. This matches what
        // `Project::apply(&patch)` would produce, but does not run the
        // §0.13 post-apply checks — that's the kernel's job in
        // `apply_write_ordering`.
        let mut post_state = prior.clone();
        post_state.metadata = new_metadata;
        let envelope = data_envelope(&typed, &post_state);
        let data = serde_json::to_value(&envelope).map_err(|e| {
            VerbError::Custom(format!(
                "project.set_metadata: data envelope serialize failed: {e}"
            ))
        })?;

        Ok((patch, data, Vec::new()))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: ProjectSetMetadataArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "ProjectSetMetadataArgs",
            })?;
        let envelope = data_envelope(&typed, post_state);
        serde_json::to_value(&envelope).map_err(|e| ReconstructError::Custom(e.to_string()))
    }
}
