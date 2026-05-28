//! `project.create` (§2.1) — eighty-fourth production verb in the
//! engine. First slice of the project arc that mints a fresh on-disk
//! project from scratch.
//!
//! ## Spec quote (`spec/commands/project.md` §2.1, abbreviated)
//!
//! > Creates a new project folder on disk. Empty timeline pre-seeded
//! > with one default video track and one default audio track.
//! >
//! > CLI: `verbreel project create --name <str> --canvas <WxH>
//! >       [--fps_num <n>] [--fps_den <n>] [--at <path>] [--activate]`
//! > MCP: `project.create`
//! > Args: `name: string`, `canvas: "<W>x<H>"`, `fps_num?: i32`
//! >       (default 30), `fps_den?: i32` (default 1), `at?: string`,
//! >       `activate?: bool` (default false), `metadata?: object`.
//! > Returns (`data`): `{ project_id: string; path: string;
//! >                       seeded_track_ids: { video: string; audio: string } }`.
//! > Errors: `E_PROJECT_EXISTS`, `E_IO`, `E_IDEMPOTENCY_CONFLICT`.
//!
//! ## Why this verb is NOT a [`Verb`](crate::reconstructor::Verb) trait impl
//!
//! Mirrors `project.duplicate` (§2.7) / `project.save` (§2.3) /
//! `project.close` (§2.5): there is no `prior` [`Project`] at call
//! time — the project does not exist yet. The pure
//! `compute_patch(&Project, &args)` trait contract cannot model a
//! verb that conjures a fresh root from arg data. The verb is also
//! the bootstrap that **creates** `events.jsonl`, so it can't itself
//! be recorded as an event line (the log doesn't exist when the call
//! starts). §2.1 makes idempotency persisted globally for the same
//! reason.
//!
//! ## Scope (v1 floor)
//!
//! - Parse and validate args: `canvas` regex, positive `fps_*`,
//!   non-empty `name` per [`PROJECT_NAME_MIN`] / [`PROJECT_NAME_MAX`].
//! - Mint `project_id` and two `track_id`s via [`ProjectId::now`] /
//!   [`TrackId::now`] (`UUIDv7`).
//! - Build the initial [`Project`] graph: zero-length timeline, two
//!   pre-seeded tracks (one [`TrackKind::Video`] named "Video 1",
//!   one [`TrackKind::Audio`] named "Audio 1"), empty
//!   `assets` / `markers` / `trackers`, caller-supplied `metadata`
//!   (defaulted to `{}`).
//! - Delegate to [`ProjectStore::create_with_registry`] for the
//!   on-disk write — that primitive already does the
//!   `<root>/project.json` (canonical JSON via [`verbreel_canon`])
//!   plus `<root>/.verbreel/events.jsonl` (empty, flock'd) layout per
//!   §2.1 and §0.5.2 of the spec.
//! - On success, drop the live store so the flock releases (the
//!   spec contract is "create a project on disk", not "open it");
//!   callers that want to keep the project loaded chain a
//!   `project.open` themselves.
//! - If `args.activate == true` and the host-config layer landed,
//!   write `~/.verbreel/active-project`. **v1 floor**: that file is
//!   not yet plumbed (same deferral as `project.duplicate` for the
//!   projects-index — see [`project_duplicate`]); the flag is
//!   accepted at the surface but has no observable effect. Document
//!   this gap rather than silently dropping the flag.
//!
//! ## Deferred (out of v1 scope)
//!
//! - **`~/.verbreel/projects-index` registration**: the host-wide
//!   index is not yet implemented. The same slice that wires
//!   `project.list` (§2.6) and `project.duplicate` (§2.7) read/write
//!   will land the writer for this verb too — until then a project
//!   created on disk is invisible to `project.list`. Document the
//!   gap rather than ship a half-wired index.
//! - **`activate: true` write of `~/.verbreel/active-project`**: same
//!   gap. The arg is accepted to keep the surface stable; the upgrade
//!   slice will land the write.
//! - **`E_IDEMPOTENCY_CONFLICT`**: §2.1 ties this to a global
//!   idempotency index persisted at `~/.verbreel/idempotency.json`
//!   per §0.8. The per-project idempotency index already runs inside
//!   [`ProjectStore`], but the global index needed for project-less
//!   verbs is not yet plumbed. Same deferral as `project.duplicate`.
//!
//! ## Universal-implicit errors
//!
//! `spec/commands/project.md` §2.1 lists `E_PROJECT_EXISTS`, `E_IO`,
//! and `E_IDEMPOTENCY_CONFLICT`. The args also have shape constraints
//! that surface as `E_SCHEMA_VIOLATION` per the §0.12 universal-
//! implicit rule (`appendix-a-errors.md` line 23): malformed `canvas`,
//! zero-dimension canvas, non-positive `fps_*`, empty `name`, name
//! exceeding [`PROJECT_NAME_MAX`]. These are reachable through the
//! arg shape, so the verb surfaces them as discrete typed variants
//! rather than burying them inside a generic `BadArgs`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use verbreel_events::timestamp_rfc3339_now;
use verbreel_types::{ProjectId, TICK_RATE_HZ, Tick, TrackId};

use crate::canvas::Canvas;
use crate::lifecycle::{LifecycleError, ProjectStore};
use crate::newtypes::Color;
use crate::project::{Project, SCHEMA_VERSION};
use crate::track::{Track, TrackKind};
use crate::verbs::project_rename::{PROJECT_NAME_MAX, PROJECT_NAME_MIN};
use crate::verbs::project_set_canvas::{CANVAS_MAX_DIM, CANVAS_MIN_DIM};

/// Default frames-per-second numerator when `args.fps_num` is omitted
/// (§2.1 spec table — "30").
pub const DEFAULT_FPS_NUM: u32 = 30;

/// Default frames-per-second denominator when `args.fps_den` is
/// omitted (§2.1 spec table — "1").
pub const DEFAULT_FPS_DEN: u32 = 1;

/// Arguments for [`create`].
///
/// `deny_unknown_fields` enforces the §2.1 published surface: stray
/// MCP/HTTP payload keys surface as arg-shape errors rather than
/// being silently accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectCreateArgs {
    /// Project display name. Doubles as the on-disk folder name per
    /// §2.1. Must satisfy the schema's `Project.name` constraints
    /// (1..=[`PROJECT_NAME_MAX`] UTF-8 chars).
    pub name: String,

    /// Canvas geometry in `"<W>x<H>"` form (e.g. `"1080x1920"`).
    /// Parsed via [`parse_canvas`]; both axes must be in
    /// `[CANVAS_MIN_DIM, CANVAS_MAX_DIM]` (shared with
    /// [`project.set_canvas`](crate::verbs::project_set_canvas)).
    pub canvas: String,

    /// Frame-rate numerator. Defaults to [`DEFAULT_FPS_NUM`] when
    /// omitted. Must be `>= 1`.
    #[serde(default)]
    pub fps_num: Option<u32>,

    /// Frame-rate denominator. Defaults to [`DEFAULT_FPS_DEN`] when
    /// omitted. Must be `>= 1`. Pass `(fps_num: 30000, fps_den: 1001)`
    /// for NTSC 29.97 per §2.1.
    #[serde(default)]
    pub fps_den: Option<u32>,

    /// Parent directory under which `<name>/` lands. When `None`, the
    /// caller's current working directory is used. Must be absolute
    /// when supplied — relative paths are rejected with
    /// [`ProjectCreateError::RelativeAt`] because the returned
    /// envelope's `path` field is contractually absolute and a
    /// process-cwd-relative resolution would make destination
    /// placement depend on runtime cwd. The `None` path goes through
    /// [`std::env::current_dir`] and is canonicalised by joining
    /// `name`.
    #[serde(default)]
    pub at: Option<PathBuf>,

    /// CLI-only ergonomics flag per §2.1. When `true`, the CLI layer
    /// writes `project_id` to `~/.verbreel/active-project` so
    /// subsequent CLI invocations infer the project without
    /// `--project`. **v1 floor**: not yet plumbed at the storage
    /// layer — see module docs.
    #[serde(default)]
    pub activate: bool,

    /// Free-form caller metadata, copied verbatim into
    /// [`Project::metadata`]. Defaults to `{}` per §2.1.
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

/// Envelope returned by a successful [`create`].
///
/// Mirrors the §2.1 `data` shape exactly: `project_id` is the freshly
/// minted [`ProjectId`], `path` is the absolute path of the new
/// project root (`<resolved_at>/<name>`), `seeded_track_ids` carries
/// the IDs of the two auto-created tracks so the caller can
/// immediately `clip.add` against them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectCreateData {
    /// Freshly minted [`ProjectId`].
    pub project_id: ProjectId,
    /// Absolute path of the new project's root folder.
    pub path: PathBuf,
    /// IDs of the two pre-seeded tracks.
    pub seeded_track_ids: SeededTrackIds,
}

/// IDs of the two tracks the engine pre-seeds on every fresh project
/// per §2.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeededTrackIds {
    /// ID of the seeded video track (`kind: "video"`, name "Video 1").
    pub video: TrackId,
    /// ID of the seeded audio track (`kind: "audio"`, name "Audio 1").
    pub audio: TrackId,
}

/// Errors returned by [`create`].
#[derive(Debug, thiserror::Error)]
pub enum ProjectCreateError {
    /// `args.name` is empty (`Project.name` `minLength: 1`). Maps to
    /// `E_SCHEMA_VIOLATION` per §0.12 universal-implicit (reachable
    /// via the `name` arg shape).
    #[error("project.create: `name` cannot be empty")]
    NameEmpty,

    /// `args.name` exceeds [`PROJECT_NAME_MAX`] UTF-8 chars. Maps to
    /// `E_SCHEMA_VIOLATION`.
    #[error("project.create: `name` has {actual} chars, exceeds max of {max}")]
    NameTooLong {
        /// Actual character count.
        actual: usize,
        /// Maximum allowed characters.
        max: usize,
    },

    /// `args.canvas` did not match the `"<W>x<H>"` shape (digits,
    /// `x`, digits) per §2.1. Maps to `E_SCHEMA_VIOLATION`.
    #[error("project.create: `canvas` must be \"<W>x<H>\" (e.g. \"1080x1920\"), got: {0:?}")]
    InvalidCanvas(String),

    /// `args.canvas` parsed but a dimension is outside
    /// [[`CANVAS_MIN_DIM`](crate::verbs::project_set_canvas::CANVAS_MIN_DIM),
    /// [`CANVAS_MAX_DIM`](crate::verbs::project_set_canvas::CANVAS_MAX_DIM)].
    /// Maps to `E_SCHEMA_VIOLATION`.
    #[error(
        "project.create: canvas dim out of range: width={width}, height={height} \
         (range {min}..={max})"
    )]
    CanvasDimOutOfRange {
        /// Parsed width.
        width: u32,
        /// Parsed height.
        height: u32,
        /// Minimum allowed dim
        /// ([`CANVAS_MIN_DIM`](crate::verbs::project_set_canvas::CANVAS_MIN_DIM)).
        min: u32,
        /// Maximum allowed dim
        /// ([`CANVAS_MAX_DIM`](crate::verbs::project_set_canvas::CANVAS_MAX_DIM)).
        max: u32,
    },

    /// `args.fps_num` or `args.fps_den` is zero. Maps to
    /// `E_SCHEMA_VIOLATION` — a zero fps numerator means zero frames
    /// per second; a zero denominator divides by zero in
    /// `fps_num / fps_den`. Per §0.2 both must be `>= 1`.
    #[error("project.create: fps must have both numerator and denominator >= 1; got {num}/{den}")]
    InvalidFps {
        /// Supplied `fps_num`.
        num: u32,
        /// Supplied `fps_den`.
        den: u32,
    },

    /// `args.at` was supplied as a relative path. Maps to
    /// `E_SCHEMA_VIOLATION` (universal-implicit per §0.12; reachable
    /// via the `at` arg shape). Relative paths are rejected because
    /// the returned envelope contractually carries the absolute
    /// destination, and process-cwd-relative resolution would couple
    /// destination placement to the caller's runtime cwd.
    #[error("project.create: `at` must be absolute, got: {path}")]
    RelativeAt {
        /// The relative path the caller supplied.
        path: PathBuf,
    },

    /// Destination root already exists (`<at>/<name>` is a file or
    /// directory). Maps to `E_PROJECT_EXISTS`. Surfaces BEFORE any
    /// writes so the create is transactional.
    #[error("project.create: destination already exists: {path}")]
    ProjectExists {
        /// The destination path that exists.
        path: PathBuf,
    },

    /// Filesystem failure during the on-disk write
    /// (`create_dir_all`, canonical-JSON write, fsync, etc.). Maps to
    /// `E_IO`.
    #[error("project.create: I/O failure: {0}")]
    Io(#[from] std::io::Error),

    /// Underlying [`ProjectStore::create_with_registry`] failed for a
    /// reason that does not map to one of the spec-aligned variants
    /// above (e.g. reconstructor-gate failure or backend lock
    /// contention). Maps to `E_IO` at the envelope layer but is
    /// preserved as a distinct variant so callers can match on it.
    #[error("project.create: lifecycle failure: {0}")]
    LifecycleFailed(#[from] LifecycleError),
}

/// Create a fresh project on disk per §2.1.
///
/// Algorithm:
/// 1. Validate `args.name`, `args.canvas`, `args.fps_num`,
///    `args.fps_den`, `args.at`.
/// 2. Resolve `dest = <at-or-cwd>/<name>` and refuse if it already
///    exists (`E_PROJECT_EXISTS`) before any writes.
/// 3. Mint `project_id` and the two seeded `track_id`s.
/// 4. Build the [`Project`] graph (canvas, fps, two pre-seeded tracks
///    named "Video 1" / "Audio 1", caller metadata).
/// 5. Delegate to [`ProjectStore::create_with_registry`] with the
///    engine's `default_registry` + `default_fixtures` so the §0.8
///    reconstructor-purity gate passes on the fresh store. The
///    primitive writes `<dest>/project.json` (canonical JSON via
///    `verbreel_canon::canonicalize` is enforced inside the
///    `ProjectStore::save` callsite — v1 floor uses the
///    `to_vec_pretty` path until the canon-write slice lands; see
///    [`crate::lifecycle::ProjectStore::save`] rustdoc) and
///    `<dest>/.verbreel/events.jsonl` (empty, fsync'd, flock'd).
/// 6. Drop the live store so the flock releases. The spec contract
///    is "create a project on disk", not "open it"; callers that
///    want the loaded handle chain a `project.open` themselves.
/// 7. Return the §2.1 envelope.
///
/// # Errors
///
/// See variants of [`ProjectCreateError`].
///
/// # Panics
///
/// Panics only if the compile-time-constant default background color
/// literal `"#000000ff"` fails the [`crate::Color`] regex — which is
/// an upstream bug in [`crate::Color`], not a runtime concern.
pub fn create(args: &ProjectCreateArgs) -> Result<ProjectCreateData, ProjectCreateError> {
    // Step 1a: name validation (mirror project_rename / project_duplicate).
    let char_count = args.name.chars().count();
    if char_count < PROJECT_NAME_MIN {
        return Err(ProjectCreateError::NameEmpty);
    }
    if char_count > PROJECT_NAME_MAX {
        return Err(ProjectCreateError::NameTooLong {
            actual: char_count,
            max: PROJECT_NAME_MAX,
        });
    }

    // Step 1b: canvas parse + range check.
    let (width, height) = parse_canvas(&args.canvas)?;
    if !(CANVAS_MIN_DIM..=CANVAS_MAX_DIM).contains(&width)
        || !(CANVAS_MIN_DIM..=CANVAS_MAX_DIM).contains(&height)
    {
        return Err(ProjectCreateError::CanvasDimOutOfRange {
            width,
            height,
            min: CANVAS_MIN_DIM,
            max: CANVAS_MAX_DIM,
        });
    }

    // Step 1c: fps validation. Defaults are positive by construction;
    // only the caller-supplied path can hit zero.
    let fps_num = args.fps_num.unwrap_or(DEFAULT_FPS_NUM);
    let fps_den = args.fps_den.unwrap_or(DEFAULT_FPS_DEN);
    if fps_num == 0 || fps_den == 0 {
        return Err(ProjectCreateError::InvalidFps {
            num: fps_num,
            den: fps_den,
        });
    }

    // Step 1d: `at` absoluteness check (only when supplied — the
    // `None` branch resolves against cwd in step 2).
    if let Some(at) = args.at.as_deref()
        && !at.is_absolute()
    {
        return Err(ProjectCreateError::RelativeAt {
            path: at.to_path_buf(),
        });
    }

    // Step 2: resolve dest.
    let dest = resolve_destination(args.at.as_deref(), &args.name)?;
    // `symlink_metadata` avoids following dangling symlinks into a
    // misleading "doesn't exist" verdict (mirrors project_duplicate).
    if dest.symlink_metadata().is_ok() {
        return Err(ProjectCreateError::ProjectExists { path: dest });
    }

    // Step 3: mint ids.
    let project_id = ProjectId::now();
    let video_track_id = TrackId::now();
    let audio_track_id = TrackId::now();

    // Step 4: build the initial graph.
    let now = timestamp_rfc3339_now();
    let project = Project {
        id: project_id,
        schema_version: SCHEMA_VERSION.to_string(),
        tick_rate_hz: TICK_RATE_HZ,
        name: args.name.clone(),
        created_at: now.clone(),
        updated_at: now,
        canvas: Canvas {
            width,
            height,
            // §0.5 default — schema-mandated background color.
            background: Color::new("#000000ff".to_string())
                .expect("compile-time-valid color literal"),
            pixel_aspect_num: 1,
            pixel_aspect_den: 1,
        },
        fps_num,
        fps_den,
        duration_tk: Tick::new(0),
        tracks: vec![
            seeded_track(video_track_id, TrackKind::Video, "Video 1"),
            seeded_track(audio_track_id, TrackKind::Audio, "Audio 1"),
        ],
        assets: Vec::new(),
        markers: Vec::new(),
        metadata: args.metadata.clone(),
        last_saved_event_id: None,
        trackers: Vec::new(),
    };

    // Step 5: delegate to the lifecycle primitive. Use the engine's
    // canonical registry + fixtures so the §0.8 reconstructor-purity
    // gate passes by construction on the fresh store. A
    // `ProjectAlreadyExists` here would only fire if a racer beat us
    // between the step-2 check and this call — surface it as the
    // spec-aligned `ProjectExists` variant rather than the generic
    // lifecycle wrapper.
    //
    // Build the registry/fixtures inline (one-shot at call time)
    // rather than holding a long-lived static — the call is rare
    // (project creation), and the pair is cheap to materialize.
    let registry = crate::verbs::default_registry();
    let fixtures = crate::verbs::default_fixtures();
    let store =
        ProjectStore::create_with_registry(&dest, project, &registry, &fixtures).map_err(|e| {
            match e {
                LifecycleError::ProjectAlreadyExists => {
                    ProjectCreateError::ProjectExists { path: dest.clone() }
                }
                LifecycleError::Io(io) => ProjectCreateError::Io(io),
                other => ProjectCreateError::LifecycleFailed(other),
            }
        })?;

    // Step 6: drop the store so the flock releases. The §2.1 contract
    // is "create a project on disk", not "open it"; callers that want
    // a live handle chain `project.open` themselves. Explicit `drop`
    // here is load-bearing — letting the binding go out of scope
    // would do the same thing, but the explicit drop documents intent
    // and matches `project.close`'s pattern.
    drop(store);

    Ok(ProjectCreateData {
        project_id,
        path: dest,
        seeded_track_ids: SeededTrackIds {
            video: video_track_id,
            audio: audio_track_id,
        },
    })
}

/// Parse `"<W>x<H>"` into `(width, height)`. Accepts only ASCII
/// decimal digits on both sides of a single lowercase `x`. Rejects
/// empty halves, leading `+`/`-`, embedded whitespace, scientific
/// notation, and uppercase `X` — the §2.1 example uses lowercase `x`
/// and the spec table fixes the form `"<W>x<H>"`.
fn parse_canvas(s: &str) -> Result<(u32, u32), ProjectCreateError> {
    // One and only one 'x' separator. `splitn(2, 'x')` collects into
    // exactly two halves on success; any other case (zero or two+
    // separators, embedded uppercase 'X', whitespace) trips parse.
    let mut parts = s.splitn(2, 'x');
    let w_str = parts.next().unwrap_or("");
    let h_str = parts.next().unwrap_or("");
    // Reject anything that isn't pure ASCII digits — parse::<u32> on
    // its own would accept `+` and the inner `+` of `1080x+1920` would
    // sneak past as just the second half. Belt-and-braces validation
    // before delegating to parse.
    if w_str.is_empty()
        || h_str.is_empty()
        || !w_str.bytes().all(|b| b.is_ascii_digit())
        || !h_str.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(ProjectCreateError::InvalidCanvas(s.to_string()));
    }
    let w: u32 = w_str
        .parse()
        .map_err(|_| ProjectCreateError::InvalidCanvas(s.to_string()))?;
    let h: u32 = h_str
        .parse()
        .map_err(|_| ProjectCreateError::InvalidCanvas(s.to_string()))?;
    Ok((w, h))
}

/// Resolve the destination root per §2.1. `at` is absolute (validated
/// upstream); `None` falls back to [`std::env::current_dir`].
fn resolve_destination(at: Option<&Path>, name: &str) -> Result<PathBuf, ProjectCreateError> {
    let parent = match at {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir().map_err(ProjectCreateError::Io)?,
    };
    Ok(parent.join(name))
}

/// Build one of the two seeded tracks. Default field values match
/// the schema defaults so the resulting `Project` round-trips
/// through canonical JSON without reading any `serde(default)`
/// fallbacks.
fn seeded_track(id: TrackId, kind: TrackKind, name: &str) -> Track {
    Track {
        id,
        kind,
        name: name.to_string(),
        clips: Vec::new(),
        muted: false,
        solo: false,
        locked: false,
        hidden: false,
        volume: 1.0,
        pan: 0.0,
        effects: Vec::new(),
    }
}
