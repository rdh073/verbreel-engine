//! verbreel-runtime — native application use-cases.
//!
//! The state crate owns pure project policy and event replay. The render crate
//! owns GPU/codec execution against neutral IR. This crate is the narrow
//! application boundary between them for delivery surfaces that need side
//! effects: resolving a project root, opening the store, lowering state into
//! IR, decoding source assets, running render, and atomically writing output.
//!
//! CLI/MCP/HTTP depend on this crate only behind their `native-render` feature
//! so the default v1 floor remains small and pure.

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]

use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};
use thiserror::Error;
use verbreel_ir::{AssetHash, CompositionInput, IrNodeId};
use verbreel_state::{
    Clip, LifecycleError, MutateOutcome, Project, ProjectId, ProjectStore, RenderStartArgs,
    RenderStartData, RenderVideoCodec, TrackKind, default_fixtures, default_registry,
    render_start_result_warning,
};

/// Runtime configuration for side-effecting native use-cases.
///
/// The explicit root list is primarily for tests and embedders. `from_env`
/// adds the current directory and the user-wide projects index under
/// `VERBREEL_HOME` / `HOME`.
#[derive(Debug, Clone, Default)]
pub struct RenderRuntimeConfig {
    project_roots: Vec<PathBuf>,
    homes: Vec<PathBuf>,
}

impl RenderRuntimeConfig {
    /// Construct an empty config. Add roots with [`Self::with_project_root`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct the default process config from current directory and home.
    #[must_use]
    pub fn from_env() -> Self {
        let mut config = Self::new();
        if let Ok(cwd) = std::env::current_dir() {
            config = config.with_project_root(cwd);
        }
        if let Some(home) = std::env::var_os("VERBREEL_HOME").or_else(|| std::env::var_os("HOME")) {
            config = config.with_home(home);
        }
        config
    }

    /// Add a candidate project root.
    #[must_use]
    pub fn with_project_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.project_roots.push(root.into());
        self
    }

    /// Add a home directory whose `.verbreel/projects-index` may locate roots.
    #[must_use]
    pub fn with_home(mut self, home: impl Into<PathBuf>) -> Self {
        self.homes.push(home.into());
        self
    }

    /// Resolve and run `render.start`.
    ///
    /// # Errors
    ///
    /// Returns a [`RuntimeError`] when the project cannot be resolved, the
    /// render arguments violate the published command contract, a source asset
    /// cannot be decoded, the compositor/encoder fails, or the output cannot be
    /// written atomically.
    pub fn render_start(&self, args: &RenderStartArgs) -> Result<RenderStartData, RuntimeError> {
        let root = self.resolve_project_root(args.project_id)?;
        render_start_at_root(&root, args)
    }

    fn resolve_project_root(&self, project_id: ProjectId) -> Result<PathBuf, RuntimeError> {
        for root in &self.project_roots {
            if project_id_at_root(root)?.is_some_and(|id| id == project_id) {
                return Ok(root.clone());
            }
        }

        for home in &self.homes {
            // Delegate to the storage resolver instead of reparsing the
            // index format here: §2.6 made the index a keyed JSON object,
            // and a second parser would silently break on the new shape.
            // `NotFound` means "try the next home"; a damaged index is a
            // hard `RenderFail` (so a corrupt index is not masked as
            // "project not registered").
            match verbreel_storage::layout::resolve_root_for_project_id(
                home,
                &project_id.to_string(),
            ) {
                Ok(root) => return Ok(root),
                Err(verbreel_storage::layout::ResolveError::NotFound(_)) => {}
                Err(err) => {
                    return Err(RuntimeError::RenderFail {
                        detail: format!(
                            "projects index under `{}` could not be read: {err}",
                            home.display()
                        ),
                    });
                }
            }
        }

        Err(RuntimeError::ProjectNotFound {
            project_id: project_id.to_string(),
        })
    }
}

/// Run `render.start` using the default process runtime config.
///
/// # Errors
///
/// See [`RenderRuntimeConfig::render_start`].
pub fn render_start(args: &RenderStartArgs) -> Result<RenderStartData, RuntimeError> {
    RenderRuntimeConfig::from_env().render_start(args)
}

/// Runtime failures mapped to the published `render.start` error codes.
#[derive(Debug, Error)]
pub enum RuntimeError {
    /// `E_PROJECT_NOT_FOUND`.
    #[error("render.start: E_PROJECT_NOT_FOUND — project `{project_id}` is not registered")]
    ProjectNotFound {
        /// Project id that could not be resolved.
        project_id: String,
    },

    /// `E_RENDER_PRESET_UNKNOWN`.
    #[error("render.start: E_RENDER_PRESET_UNKNOWN — unknown preset `{preset}`")]
    RenderPresetUnknown {
        /// Caller-supplied preset name.
        preset: String,
    },

    /// `E_BAD_RANGE`.
    #[error("render.start: E_BAD_RANGE — {detail}")]
    BadRange {
        /// Range failure detail.
        detail: String,
    },

    /// `E_BAD_TIME`.
    #[error("render.start: E_BAD_TIME — {detail}")]
    BadTime {
        /// Time failure detail.
        detail: String,
    },

    /// `E_ARGS_INCOMPATIBLE`.
    #[error("render.start: E_ARGS_INCOMPATIBLE — {detail}")]
    ArgsIncompatible {
        /// Argument compatibility detail.
        detail: String,
    },

    /// `E_OUT_PATH_EXISTS`.
    #[error("render.start: E_OUT_PATH_EXISTS — output path already exists: `{path}`")]
    OutPathExists {
        /// Existing output path.
        path: String,
    },

    /// `E_PATH_ESCAPE`.
    #[error("render.start: E_PATH_ESCAPE — output path escapes project root: `{path}`")]
    PathEscape {
        /// Rejected path.
        path: String,
    },

    /// `E_RENDER_EMPTY_RANGE`.
    #[error("render.start: E_RENDER_EMPTY_RANGE — render range produced no frames")]
    RenderEmptyRange,

    /// `E_IO`.
    #[error("render.start: E_IO — {detail}")]
    Io {
        /// I/O failure detail.
        detail: String,
    },

    /// `E_RENDER_FAIL`.
    #[error("render.start: E_RENDER_FAIL — {detail}")]
    RenderFail {
        /// Renderer failure detail.
        detail: String,
    },
}

impl RuntimeError {
    /// The stable §0.7 `E_*` error code for this runtime failure, shared by
    /// every surface (CLI/MCP/HTTP) so the wire `code` is uniform by
    /// construction. The match is exhaustive: a new `RuntimeError` variant
    /// is a compile error here, not a silent misclassification.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::ProjectNotFound { .. } => "E_PROJECT_NOT_FOUND",
            Self::RenderPresetUnknown { .. } => "E_RENDER_PRESET_UNKNOWN",
            Self::BadRange { .. } => "E_BAD_RANGE",
            Self::BadTime { .. } => "E_BAD_TIME",
            Self::ArgsIncompatible { .. } => "E_ARGS_INCOMPATIBLE",
            Self::OutPathExists { .. } => "E_OUT_PATH_EXISTS",
            Self::PathEscape { .. } => "E_PATH_ESCAPE",
            Self::RenderEmptyRange => "E_RENDER_EMPTY_RANGE",
            Self::Io { .. } => "E_IO",
            Self::RenderFail { .. } => "E_RENDER_FAIL",
        }
    }
}

fn render_start_at_root(
    project_root: &Path,
    args: &RenderStartArgs,
) -> Result<RenderStartData, RuntimeError> {
    validate_codec_args(args)?;

    let registry = default_registry();
    let fixtures = default_fixtures();
    let mut store =
        ProjectStore::open_with_registry(project_root, &registry, &fixtures).map_err(|err| {
            RuntimeError::RenderFail {
                detail: format!("failed to open project `{}`: {err}", project_root.display()),
            }
        })?;
    let project = store.project().clone();
    if project.id != args.project_id {
        return Err(RuntimeError::ProjectNotFound {
            project_id: args.project_id.to_string(),
        });
    }

    let preset = resolve_render_preset(args)?;
    let from_tk = args.from_tk.unwrap_or(0);
    let requested_to_tk = args.to_tk.unwrap_or_else(|| project.duration_tk.get());
    let to_tk = args
        .to_tk
        .unwrap_or_else(|| project.duration_tk.get())
        .min(project.duration_tk.get());
    validate_range(&project, from_tk, to_tk)?;

    let output_path = resolve_output_path(project_root, &args.out_path, args.overwrite)?;
    let frame_count = frames_for_range(from_tk, to_tk, project.fps_num, project.fps_den)?;
    let frames = render_plans(&project, from_tk, frame_count)?;
    let decoded = decoded_sources(project_root, &project, &frames)?;
    let spec = verbreel_render::RenderJobSpec {
        preset,
        width: project.canvas.width,
        height: project.canvas.height,
        fps_num: project.fps_num,
        fps_den: project.fps_den,
        frames,
        decoded,
    };

    let registry = verbreel_render::JobRegistry::new();
    let job_id = registry
        .start_render(&spec)
        .map_err(render_fail("renderer failed"))?;
    let verbreel_render::RenderStatus::Done {
        frame_count: rendered_frames,
        output,
    } = registry
        .status(job_id)
        .map_err(render_fail("render status failed"))?
    else {
        return Err(RuntimeError::RenderFail {
            detail: "renderer did not reach terminal Done status".to_string(),
        });
    };
    if rendered_frames != frame_count {
        return Err(RuntimeError::RenderFail {
            detail: format!("rendered {rendered_frames} frames, expected {frame_count}"),
        });
    }

    verbreel_storage::fs::atomic_write_bytes(&output_path, &output).map_err(|err| {
        RuntimeError::Io {
            detail: format!("write `{}`: {err}", output_path.display()),
        }
    })?;

    let mut data = RenderStartData {
        job_id: job_id.to_string(),
        output_path: output_path.display().to_string(),
        duration_tk: to_tk - from_tk,
        // Empty while recording: the event payload must not embed its own
        // not-yet-minted id. Set on the returned copy below, post-record.
        event_id: String::new(),
    };
    let event_id = record_render_start_event(
        &mut store,
        args,
        &data,
        requested_to_tk,
        project.duration_tk.get(),
    )?;
    data.event_id = event_id;

    Ok(data)
}

fn project_id_at_root(root: &Path) -> Result<Option<ProjectId>, RuntimeError> {
    match fs::read(root.join("project.json")) {
        Ok(bytes) => {
            let project: Project =
                serde_json::from_slice(&bytes).map_err(|err| RuntimeError::RenderFail {
                    detail: format!("invalid project.json at `{}`: {err}", root.display()),
                })?;
            Ok(Some(project.id))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(RuntimeError::Io {
            detail: format!("read `{}`: {err}", root.join("project.json").display()),
        }),
    }
}

fn record_render_start_event(
    store: &mut ProjectStore,
    args: &RenderStartArgs,
    data: &RenderStartData,
    requested_to_tk: i64,
    clamped_to_tk: i64,
) -> Result<String, RuntimeError> {
    let event_args = serde_json::to_value(args).map_err(|err| RuntimeError::RenderFail {
        detail: format!("serialize render.start event args: {err}"),
    })?;
    let mut warnings = Vec::new();
    if requested_to_tk > clamped_to_tk {
        warnings.push(range_clamped_warning(requested_to_tk, clamped_to_tk));
    }
    warnings.push(render_start_result_warning(data));

    let outcome = store
        .mutate_with_warnings(
            "render.start",
            event_args,
            &json_patch::Patch(Vec::new()),
            None,
            warnings,
        )
        .map_err(record_event_error)?;
    // Un-keyed mutate always writes an event, so the only outcome is
    // `Applied`. Match exhaustively rather than unwrap so a future outcome
    // variant is a compile error here, not a silent empty id.
    match outcome {
        MutateOutcome::Applied { event_id, .. } => Ok(event_id.to_string()),
        MutateOutcome::Replayed { .. } | MutateOutcome::NoOp { .. } => {
            Err(RuntimeError::RenderFail {
                detail: "render.start event record returned a non-Applied outcome".to_string(),
            })
        }
    }
}

fn range_clamped_warning(requested: i64, clamped_to: i64) -> Value {
    json!({
        "code": "W_RANGE_CLAMPED",
        "message": "to_tk exceeded project duration and was clamped",
        "details": {
            "field": "to_tk",
            "requested": requested,
            "clamped_to": clamped_to,
        },
    })
}

fn record_event_error(err: LifecycleError) -> RuntimeError {
    match err {
        LifecycleError::Io(err) => RuntimeError::Io {
            detail: format!("record render.start event: {err}"),
        },
        LifecycleError::Backend(err) => RuntimeError::Io {
            detail: format!("record render.start event: {err}"),
        },
        other => RuntimeError::RenderFail {
            detail: format!("record render.start event: {other}"),
        },
    }
}

fn validate_codec_args(args: &RenderStartArgs) -> Result<(), RuntimeError> {
    if args.crf.is_some() && args.bitrate_bps.is_some() {
        return Err(RuntimeError::ArgsIncompatible {
            detail: "crf and bitrate_bps are mutually exclusive".to_string(),
        });
    }
    if args.bitrate_bps.is_some_and(|value| value <= 0) {
        return Err(RuntimeError::BadRange {
            detail: "bitrate_bps must be positive when supplied".to_string(),
        });
    }
    if args.bitrate_bps.is_some_and(|value| value > 1_000_000_000) {
        return Err(RuntimeError::BadRange {
            detail: "bitrate_bps must be <= 1000000000 when supplied".to_string(),
        });
    }
    if args.crf.is_some_and(|value| !(0..=51).contains(&value)) {
        return Err(RuntimeError::BadRange {
            detail: "crf must be in the closed range 0..=51".to_string(),
        });
    }
    if args
        .video_codec
        .is_some_and(|codec| codec != RenderVideoCodec::H264)
    {
        return Err(RuntimeError::ArgsIncompatible {
            detail: "native render v1 currently supports only video_codec=h264".to_string(),
        });
    }
    if args.keep_temp {
        return Err(RuntimeError::ArgsIncompatible {
            detail: "keep_temp is reserved for the async renderer and is not available in v1"
                .to_string(),
        });
    }
    Ok(())
}

fn validate_range(project: &Project, from_tk: i64, to_tk: i64) -> Result<(), RuntimeError> {
    if from_tk < 0 {
        return Err(RuntimeError::BadTime {
            detail: format!("from_tk must be >= 0, got {from_tk}"),
        });
    }
    if to_tk < 0 {
        return Err(RuntimeError::BadTime {
            detail: format!("to_tk must be >= 0, got {to_tk}"),
        });
    }
    if to_tk <= from_tk {
        return Err(RuntimeError::RenderEmptyRange);
    }
    let duration = project.duration_tk.get();
    if to_tk > duration {
        return Err(RuntimeError::BadTime {
            detail: format!("to_tk {to_tk} exceeds project duration {duration}"),
        });
    }
    Ok(())
}

fn resolve_render_preset(
    args: &RenderStartArgs,
) -> Result<verbreel_render::RenderPreset, RuntimeError> {
    match args.preset.as_str() {
        "deterministic" => return Ok(verbreel_render::RenderPreset::Deterministic),
        "performance" => return Ok(verbreel_render::RenderPreset::Performance),
        _ => {}
    }

    if !verbreel_state::verbs::render_list_presets::bundled_presets()
        .iter()
        .any(|preset| preset.name == args.preset)
    {
        return Err(RuntimeError::RenderPresetUnknown {
            preset: args.preset.clone(),
        });
    }

    if args.deterministic {
        Ok(verbreel_render::RenderPreset::Deterministic)
    } else {
        Ok(verbreel_render::RenderPreset::Performance)
    }
}

fn resolve_output_path(
    project_root: &Path,
    out_path: &str,
    overwrite: bool,
) -> Result<PathBuf, RuntimeError> {
    if out_path.trim().is_empty() {
        return Err(RuntimeError::ArgsIncompatible {
            detail: "out_path must not be empty".to_string(),
        });
    }
    let raw = PathBuf::from(out_path);
    if raw
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(RuntimeError::PathEscape {
            path: out_path.to_string(),
        });
    }

    let root = project_root
        .canonicalize()
        .map_err(|err| RuntimeError::Io {
            detail: format!(
                "canonicalize project root `{}`: {err}",
                project_root.display()
            ),
        })?;
    let resolved = if raw.is_absolute() {
        raw
    } else {
        root.join(raw)
    };
    if !resolved.starts_with(&root) {
        return Err(RuntimeError::PathEscape {
            path: out_path.to_string(),
        });
    }

    let parent = resolved.parent().ok_or_else(|| RuntimeError::Io {
        detail: format!("output path has no parent: `{}`", resolved.display()),
    })?;
    fs::create_dir_all(parent).map_err(|err| RuntimeError::Io {
        detail: format!("create output parent `{}`: {err}", parent.display()),
    })?;
    let parent_canon = parent.canonicalize().map_err(|err| RuntimeError::Io {
        detail: format!("canonicalize output parent `{}`: {err}", parent.display()),
    })?;
    if !parent_canon.starts_with(&root) {
        return Err(RuntimeError::PathEscape {
            path: out_path.to_string(),
        });
    }
    if resolved.is_dir() {
        return Err(RuntimeError::Io {
            detail: format!("output path is a directory: `{}`", resolved.display()),
        });
    }
    if resolved.exists() && !overwrite {
        return Err(RuntimeError::OutPathExists {
            path: resolved.display().to_string(),
        });
    }
    Ok(resolved)
}

fn frames_for_range(
    from_tk: i64,
    to_tk: i64,
    fps_num: u32,
    fps_den: u32,
) -> Result<usize, RuntimeError> {
    if fps_num == 0 || fps_den == 0 {
        return Err(RuntimeError::RenderFail {
            detail: format!("project frame rate {fps_num}/{fps_den} has a zero term"),
        });
    }
    let duration = u128::try_from(to_tk - from_tk).map_err(|err| RuntimeError::BadTime {
        detail: format!("invalid render duration: {err}"),
    })?;
    let numerator = duration * u128::from(fps_num);
    let denominator = u128::from(verbreel_ir::TICK_RATE_HZ) * u128::from(fps_den);
    let frame_count =
        usize::try_from(numerator.div_ceil(denominator)).map_err(|err| RuntimeError::BadRange {
            detail: format!("frame count does not fit usize: {err}"),
        })?;
    if frame_count == 0 {
        return Err(RuntimeError::RenderEmptyRange);
    }
    Ok(frame_count)
}

fn render_plans(
    project: &Project,
    from_tk: i64,
    frame_count: usize,
) -> Result<Vec<verbreel_render::RenderPlan>, RuntimeError> {
    let mut plans = Vec::with_capacity(frame_count);
    for frame in 0..frame_count {
        let tick = from_tk
            + i64::try_from(frame).map_err(|err| RuntimeError::BadRange {
                detail: format!("frame index does not fit i64: {err}"),
            })? * ticks_per_frame(project)?;
        let graph = verbreel_ir::build(&composition_at(project, tick)?);
        let evaluator = verbreel_ir::Evaluator::new(&graph);
        let mut plan = verbreel_render::RenderPlan::from_evaluator(&graph, &evaluator);
        if plan.layers.is_empty() {
            plan.layers.push(verbreel_render::RenderLayer {
                source_asset: None,
                alpha_q16: u16::MAX,
                cache_hash: [0u8; 32],
            });
        }
        plans.push(plan);
    }
    Ok(plans)
}

fn composition_at(project: &Project, tick: i64) -> Result<CompositionInput, RuntimeError> {
    let mut tracks = Vec::new();
    for track in project
        .tracks
        .iter()
        .filter(|track| track.kind == TrackKind::Video && !track.hidden && !track.muted)
    {
        let mut clips = Vec::new();
        for clip in track.clips.iter().filter(|clip| clip_active_at(clip, tick)) {
            let clip_asset_id = clip.asset_id.id();
            let asset_hash = project
                .assets
                .iter()
                .find(|asset| Some(asset.id()) == clip_asset_id)
                .map(|asset| AssetHash::new(asset.hash().as_str()))
                .transpose()
                .map_err(render_fail("invalid asset hash"))?;
            clips.push(verbreel_ir::ClipInput {
                source_node_id: ir_node_id(&clip.id.to_string())?,
                args_hash: canonical_args_hash(&serde_json::to_value(clip).map_err(|err| {
                    RuntimeError::RenderFail {
                        detail: format!("serialize clip args: {err}"),
                    }
                })?)?,
                asset: asset_hash,
                effects: Vec::new(),
            });
        }
        tracks.push(verbreel_ir::TrackInput {
            composite_node_id: ir_node_id(&track.id.to_string())?,
            composite_args_hash: canonical_args_hash(&json!({
                "id": track.id,
                "name": &track.name,
                "volume": track.volume,
                "pan": track.pan,
            }))?,
            clips,
        });
    }

    Ok(CompositionInput {
        tracks,
        output: verbreel_ir::OutputInput {
            node_id: ir_node_id(&project.id.to_string())?,
            args_hash: canonical_args_hash(&json!({
                "width": project.canvas.width,
                "height": project.canvas.height,
                "fps_num": project.fps_num,
                "fps_den": project.fps_den,
            }))?,
        },
        tick: u64::try_from(tick).map_err(|err| RuntimeError::BadTime {
            detail: format!("tick must be non-negative: {err}"),
        })?,
    })
}

fn clip_active_at(clip: &Clip, tick: i64) -> bool {
    let start = clip.track_position_tk.get();
    let end = start + clip.source_out_tk.get() - clip.source_in_tk.get();
    start <= tick && tick < end
}

fn decoded_sources(
    project_root: &Path,
    project: &Project,
    plans: &[verbreel_render::RenderPlan],
) -> Result<HashMap<AssetHash, verbreel_render::DecodedSource>, RuntimeError> {
    let needed = needed_assets(plans);
    let mut decoded = HashMap::new();
    for asset in &project.assets {
        let key =
            AssetHash::new(asset.hash().as_str()).map_err(render_fail("invalid asset hash"))?;
        if !needed.contains(&key) {
            continue;
        }
        let path = project_root.join(asset.path().as_str());
        let frames = verbreel_render::decode_source(&path).map_err(render_fail("decode source"))?;
        if frames.is_empty() {
            return Err(RuntimeError::RenderFail {
                detail: format!("source `{}` decoded zero frames", path.display()),
            });
        }
        decoded.insert(key, verbreel_render::DecodedSource { frames });
    }
    let decoded_keys: HashSet<AssetHash> = decoded.keys().cloned().collect();
    if let Some(missing) = needed.difference(&decoded_keys).next() {
        return Err(RuntimeError::RenderFail {
            detail: format!("source asset `{missing}` not found in project registry"),
        });
    }
    Ok(decoded)
}

fn needed_assets(plans: &[verbreel_render::RenderPlan]) -> HashSet<AssetHash> {
    plans
        .iter()
        .flat_map(|plan| plan.layers.iter())
        .filter_map(|layer| layer.source_asset.clone())
        .collect()
}

fn ticks_per_frame(project: &Project) -> Result<i64, RuntimeError> {
    if project.fps_num == 0 || project.fps_den == 0 {
        return Err(RuntimeError::RenderFail {
            detail: "project frame rate must be non-zero".to_string(),
        });
    }
    let numerator = i64::from(verbreel_ir::TICK_RATE_HZ) * i64::from(project.fps_den);
    Ok(numerator / i64::from(project.fps_num))
}

fn canonical_args_hash(value: &Value) -> Result<[u8; 32], RuntimeError> {
    let hex = verbreel_canon::sha256_hex(value).map_err(|err| RuntimeError::RenderFail {
        detail: format!("canonicalize render args: {err}"),
    })?;
    hex_to_32(&hex)
}

fn hex_to_32(hex: &str) -> Result<[u8; 32], RuntimeError> {
    if hex.len() != 64 {
        return Err(RuntimeError::RenderFail {
            detail: format!("expected 64 hex chars, got {}", hex.len()),
        });
    }
    let mut out = [0u8; 32];
    for (idx, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&hex[idx * 2..idx * 2 + 2], 16).map_err(|err| {
            RuntimeError::RenderFail {
                detail: format!("invalid hex digest: {err}"),
            }
        })?;
    }
    Ok(out)
}

fn ir_node_id(raw: &str) -> Result<IrNodeId, RuntimeError> {
    raw.parse().map_err(render_fail("invalid IR node id"))
}

fn render_fail<E: Display>(context: &'static str) -> impl FnOnce(E) -> RuntimeError + Copy {
    move |err| RuntimeError::RenderFail {
        detail: format!("{context}: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::RuntimeError;

    fn detail() -> String {
        "x".to_string()
    }

    #[test]
    fn code_maps_every_variant_to_its_section_0_7_error() {
        // Exhaustive table over every RuntimeError variant. Adding a variant
        // without extending this table fails to compile in `code()`, and a
        // copy-paste slip in the mapping is caught here rather than on the wire.
        let cases: [(RuntimeError, &str); 10] = [
            (
                RuntimeError::ProjectNotFound {
                    project_id: "p".to_string(),
                },
                "E_PROJECT_NOT_FOUND",
            ),
            (
                RuntimeError::RenderPresetUnknown {
                    preset: "p".to_string(),
                },
                "E_RENDER_PRESET_UNKNOWN",
            ),
            (RuntimeError::BadRange { detail: detail() }, "E_BAD_RANGE"),
            (RuntimeError::BadTime { detail: detail() }, "E_BAD_TIME"),
            (
                RuntimeError::ArgsIncompatible { detail: detail() },
                "E_ARGS_INCOMPATIBLE",
            ),
            (
                RuntimeError::OutPathExists {
                    path: "p".to_string(),
                },
                "E_OUT_PATH_EXISTS",
            ),
            (
                RuntimeError::PathEscape {
                    path: "p".to_string(),
                },
                "E_PATH_ESCAPE",
            ),
            (RuntimeError::RenderEmptyRange, "E_RENDER_EMPTY_RANGE"),
            (RuntimeError::Io { detail: detail() }, "E_IO"),
            (
                RuntimeError::RenderFail { detail: detail() },
                "E_RENDER_FAIL",
            ),
        ];

        for (err, expected) in cases {
            assert_eq!(err.code(), expected, "code mismatch for {err:?}");
        }
    }
}
