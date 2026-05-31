//! Native `render.start` — compiled only under `native-render`.
//!
//! `render.start` is a §11 streaming verb whose real execution (ffmpeg
//! decode → GPU composite → encode → atomic write) lives in
//! [`verbreel_runtime`], **not** the shared [`Engine`](verbreel_state::Engine).
//! The Engine routes the in-graph registry `render.start` verb but its v1
//! floor always returns `E_RENDER_FAIL` (`render_start.rs` —
//! "streaming renderer runtime unavailable in v1 floor"); the ffmpeg
//! runtime is a delivery-surface concern injected at the composition root.
//!
//! Under `native-render` the generic dispatcher therefore special-cases
//! `verb_id == "render.start"` and calls the injected runtime here instead
//! of `Engine::dispatch`, wrapping the runtime result in the §0.1 envelope
//! so the CLI's stdout shape stays uniform across every verb. This mirrors
//! the HTTP surface's `call_render_start` (`handlers.rs`) — same
//! engine-vs-runtime split, same envelope wrapping, same `E_*` mapping.
//!
//! Without the feature this module is absent and `render.start` flows
//! through `Engine::dispatch` to the `E_RENDER_FAIL` floor.

use serde_json::{Map, Value};
use verbreel_runtime::RenderRuntimeConfig;
use verbreel_state::{Envelope, RenderStartArgs};

/// Run the native `render.start` through the injected runtime and wrap
/// the result in a §0.1 envelope.
///
/// The generic flag tail has already been walked into `args` (scalar
/// coercion, `--project` injected as `project_id`), so this deserializes
/// that map into the typed [`RenderStartArgs`] the runtime expects. A
/// deserialize failure (e.g. an unknown codec string, a non-integer tick)
/// surfaces as `E_SCHEMA_VIOLATION`; a runtime failure surfaces with its
/// own `E_*` code (via [`verbreel_runtime::RuntimeError::code`]). Success carries the runtime's
/// `data` object with an empty engine patch but the truthful `event_id` the
/// native path recorded inside `verbreel-runtime` (no engine patch is produced
/// here — the runtime owns the event).
#[must_use]
pub fn dispatch(args: &Map<String, Value>, runtime: &RenderRuntimeConfig) -> Envelope {
    let typed: RenderStartArgs = match serde_json::from_value(Value::Object(args.clone())) {
        Ok(typed) => typed,
        Err(err) => {
            return Envelope::Err {
                code: "E_SCHEMA_VIOLATION".to_string(),
                message: format!("render.start: args deserialize failed: {err}"),
                hint: None,
                details: None,
            };
        }
    };
    match runtime.render_start(&typed) {
        Ok(data) => match serde_json::to_value(&data) {
            Ok(data_value) => Envelope::Ok {
                data: data_value,
                patch: Value::Array(Vec::new()),
                warnings: Vec::new(),
                event_id: data.event_id.clone(),
            },
            // A serialization fault on our own DTO is internal, not a
            // client error — surface it rather than fabricating a hollow
            // success.
            Err(err) => Envelope::Err {
                code: "E_INTERNAL".to_string(),
                message: format!("render.start: data serialize failed: {err}"),
                hint: None,
                details: None,
            },
        },
        // The runtime error's `Display` embeds its `E_*` code in the
        // message; surface the structured code via the shared
        // `RuntimeError::code()` mapper (the single tested source of truth
        // in `verbreel-runtime`) so the wire `code` is uniform across
        // CLI/MCP/HTTP by construction.
        Err(err) => Envelope::Err {
            code: err.code().to_string(),
            message: err.to_string(),
            hint: None,
            details: None,
        },
    }
}
