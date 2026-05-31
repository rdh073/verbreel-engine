//! `project.forget` (§2.8) — eighty-third production verb. Eighth
//! slice of the project arc; engine-scoped (does not touch any open
//! project on disk).
//!
//! ## Spec quote (`spec/commands/project.md` §2.8, abbreviated)
//!
//! > Removes a project's entry from `~/.verbreel/projects-index`
//! > without modifying the project on disk. Used to clean up the
//! > "recent projects" list. The project remains fully usable —
//! > `project.open` will re-add the entry.
//! >
//! > CLI: `verbreel project forget (--path <path> | --project_id <id>)`
//! > MCP: `project.forget`
//! > Args: `path?: string, project_id?: string` — exactly one must be
//! > supplied (`oneOf` in the arg schema). The path form is for
//! > projects never opened in this engine instance; the id form is
//! > for projects discovered via `project.list`.
//! > Returns (`data`): `{ removed_path: string; was_in_index: boolean }`.
//! > Errors: `E_ARGS_INCOMPATIBLE`, `E_BAD_RANGE`,
//! > `E_PROJECT_NOT_FOUND`, `E_IO`.
//!
//! ## Why this verb is NOT a [`Verb`](crate::reconstructor::Verb) trait impl
//!
//! `project.forget` is engine-scoped — the spec explicitly states
//! "does not modify the project on disk". The verb mutates
//! `~/.verbreel/projects-index` (a host-wide file), reads/writes
//! nothing in any project's `events.jsonl`, and has no
//! dispatcher-style `project_id` argument (`project_id` here is one
//! of two **mutually exclusive** lookup keys, not a dispatch target).
//!
//! The [`crate::reconstructor::Verb`] trait requires
//! `compute_patch(&Project, &args) → (Patch, data, warnings)`, which
//! presupposes a single open project. Mirror the established free-
//! function precedent (`project.save` §2.3, `project.duplicate` §2.7).
//!
//! ## Index removal (§2.6 keyed-object map)
//!
//! [`forget`] takes the engine's `home` and removes the matching entry
//! from `<home>/.verbreel/projects-index` under the shared RMW `flock`
//! (the same lock `register_project` / `project.list` use). The two
//! lookup forms are not symmetric:
//!
//! - **Id form** resolves through the index: a hit yields the recorded
//!   `path` (echoed as `removed_path`) and removes the entry; a miss is
//!   [`ProjectForgetError::ProjectNotFound`] (`E_PROJECT_NOT_FOUND`).
//! - **Path form** removes by recorded path and reports the truthful
//!   `was_in_index` — `false` when no entry matched (never an error,
//!   per §2.8: a path never opened in this engine is simply not indexed).

use std::path::Path;

use serde::{Deserialize, Serialize};

use verbreel_storage::layout;

/// Arguments for [`forget`].
///
/// `deny_unknown_fields` enforces the §2.8 published surface: stray
/// MCP/HTTP payload keys surface as arg-shape errors rather than
/// being silently accepted.
///
/// Note: §2.8 specifies `path` XOR `project_id` (`oneOf`) — exactly
/// one must be supplied. The struct accepts both as `Option<String>`
/// so deserialization succeeds for any combination; [`forget`]
/// enforces the mutual exclusion and surfaces
/// [`ProjectForgetError::ArgsIncompatible`] for `(None, None)` or
/// `(Some, Some)`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectForgetArgs {
    /// Absolute or relative on-disk path of the project entry to
    /// forget. Path form is for projects never opened in this engine
    /// instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    /// `UUIDv7` (string form) of the project entry to forget. Id
    /// form is for projects discovered via `project.list`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

/// Envelope returned by a successful [`forget`].
///
/// Mirrors the §2.8 `data` shape exactly: two keys. `removed_path`
/// echoes the resolved on-disk path even when `was_in_index: false`,
/// so callers can correlate the request to a path without keeping
/// their own bookkeeping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectForgetData {
    /// The on-disk path resolved from the input args (echoed
    /// verbatim from `path` in the path form; in a future slice,
    /// resolved from the index for the id form).
    pub removed_path: String,

    /// Whether the entry was actually present in the index before
    /// the call. The id form only returns `data` when it removed an
    /// entry, so it is always `true`; the path form reports the truth
    /// of the match.
    pub was_in_index: bool,
}

/// Errors returned by [`forget`].
#[derive(Debug, thiserror::Error)]
pub enum ProjectForgetError {
    /// Both `path` and `project_id` are set, or neither is. Maps to
    /// `E_ARGS_INCOMPATIBLE`.
    #[error("project.forget: args incompatible: {detail}")]
    ArgsIncompatible {
        /// Static hint identifying which form of incompatibility was
        /// detected (both vs neither).
        detail: &'static str,
    },

    /// `path` is empty or contains a `NUL` byte. Maps to
    /// `E_BAD_RANGE`. Platform-specific path-validation failures
    /// (Windows reserved names, `PATH_MAX` overruns) are deferred to
    /// the follow-up slice that performs the real file probe — at
    /// that point those surface as [`ProjectForgetError::Io`], which
    /// is the spec-correct path.
    #[error("project.forget: bad range: {detail}")]
    BadRange {
        /// What was wrong with the input path.
        detail: String,
    },

    /// Id-form lookup did not resolve to any index entry. Maps to
    /// `E_PROJECT_NOT_FOUND`.
    #[error("project.forget: no index entry for {lookup}")]
    ProjectNotFound {
        /// The id the caller passed.
        lookup: String,
    },

    /// Filesystem failure during the index read or write (a corrupt
    /// index, lock contention, or an underlying IO error). Maps to
    /// `E_IO`.
    #[error("project.forget: I/O failure: {0}")]
    Io(#[from] std::io::Error),
}

/// Remove a project entry from `<home>/.verbreel/projects-index` per §2.8.
///
/// Validation order:
/// 1. Mutual exclusion: `(None, None)` and `(Some, Some)` both
///    surface as [`ProjectForgetError::ArgsIncompatible`].
/// 2. Path form: validate (non-empty, no NUL) → [`ProjectForgetError::
///    BadRange`] on malformed input; else remove every entry whose
///    recorded path matches and return `{removed_path: <input>,
///    was_in_index: <matched>}` — a path that was never indexed is not
///    an error (`was_in_index: false`).
/// 3. Id form: resolve the entry from the index; a hit removes it and
///    returns `{removed_path: <recorded path>, was_in_index: true}`, a
///    miss is [`ProjectForgetError::ProjectNotFound`].
///
/// `home` is the engine's user-wide root (the same path
/// `register_project` / `project.list` operate on); the verb is a free
/// function so it takes it as an argument rather than reaching for any
/// process state.
///
/// # Errors
///
/// See variants of [`ProjectForgetError`]. A corrupt index, lock
/// contention, or any other IO failure during the index read-modify-write
/// surfaces as [`ProjectForgetError::Io`].
pub fn forget(
    home: &Path,
    args: &ProjectForgetArgs,
) -> Result<ProjectForgetData, ProjectForgetError> {
    match (args.path.as_deref(), args.project_id.as_deref()) {
        (None, None) => Err(ProjectForgetError::ArgsIncompatible {
            detail: "exactly one of `path` or `project_id` is required",
        }),
        (Some(_), Some(_)) => Err(ProjectForgetError::ArgsIncompatible {
            detail: "`path` and `project_id` are mutually exclusive",
        }),
        (Some(path), None) => {
            validate_path(path)?;
            let was_in_index = layout::deregister_project_by_path(home, path)?;
            Ok(ProjectForgetData {
                removed_path: path.to_string(),
                was_in_index,
            })
        }
        (None, Some(id)) => match layout::deregister_project_by_id(home, id)? {
            Some(removed_path) => Ok(ProjectForgetData {
                removed_path,
                was_in_index: true,
            }),
            None => Err(ProjectForgetError::ProjectNotFound {
                lookup: id.to_string(),
            }),
        },
    }
}

/// Cross-platform-portable path validation: empty string and embedded
/// NUL byte are rejected. Platform-specific cases (Windows reserved
/// names, max-length, etc.) are deferred to the slice that wires real
/// file I/O — at that point the OS itself surfaces those errors via
/// `Io`.
fn validate_path(path: &str) -> Result<(), ProjectForgetError> {
    if path.is_empty() {
        return Err(ProjectForgetError::BadRange {
            detail: "`path` cannot be empty".to_string(),
        });
    }
    if path.contains('\0') {
        return Err(ProjectForgetError::BadRange {
            detail: "`path` contains a NUL byte".to_string(),
        });
    }
    Ok(())
}
