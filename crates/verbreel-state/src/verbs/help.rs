//! `help` (§1.1) — sixty-fourth production verb in the engine.
//!
//! ## Spec quote (`spec/commands/meta.md` §1.1)
//!
//! > CLI: `verbreel help [<noun>[.<verb>]]`
//! > MCP: `help`
//! > Args: `topic?: string`
//! > Returns (`data`): `{ nouns?: string[]; verbs?: VerbDoc[]; verb?: VerbDoc }`
//! > Errors: `E_UNKNOWN_TOPIC`
//!
//! ## Self-documenting registry introspection.
//!
//! `help` is read-only and does not read or mutate project state; it
//! enumerates the verb registry returned by [`crate::verbs::default_registry`].
//! Three dispatch branches based on `topic`:
//!
//! - `None` → list distinct noun prefixes (the `<noun>` half of every
//!   `<noun>.<verb>` registered verb, plus single-word verb names).
//! - Topic without `.` → list every registered verb matching
//!   `<topic>.*` OR the bare-name verb whose id equals `topic`
//!   (`describe`, `help`, `list_capabilities`).
//! - Topic with `.` → look up the exact verb id.
//!
//! ## `VerbDoc` shape (spec gap)
//!
//! Spec §1.1 references `VerbDoc` but does not formally define it. v1
//! ships the 3-field shape `{name, summary, args_schema_id}` matching
//! [`crate::verbs::list_capabilities::VerbEntry`] exactly. `summary`
//! and `args_schema_id` are empty strings in v1 — the `Verb` trait does
//! not yet expose `fn summary() -> &'static str` or schema ids. Future
//! expansion (cli/mcp signatures, embedded args schema, returns schema,
//! errors[]) is additive.
//!
//! `VerbDoc` is kept as a separate type from `VerbEntry` so the help
//! response shape can evolve independently from `list_capabilities`.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use verbreel_types::ProjectId;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

/// Arguments for `help`.
///
/// `project_id` is required for `Verb` trait compatibility; the impl
/// ignores it (the spec defines only `topic?`). See `list_capabilities`
/// for the same accommodation pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpArgs {
    /// Required by the `Verb` trait shape; not read by the impl.
    pub project_id: ProjectId,

    /// Topic selector. `None` lists nouns; a single noun (`"clip"`)
    /// lists verbs under that noun; a full id (`"clip.add"`) returns
    /// that verb's doc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
}

/// Per-verb documentation entry. 3-field shape mirrors
/// [`crate::verbs::list_capabilities::VerbEntry`] — see module docs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerbDoc {
    /// Verb id (e.g. `"clip.add"`).
    pub name: String,
    /// Short verb summary. Empty in v1 — `Verb` trait does not yet
    /// expose `fn summary() -> &'static str`.
    pub summary: String,
    /// Args schema identifier. Empty in v1 — `Verb` trait does not yet
    /// expose schema ids.
    pub args_schema_id: String,
}

/// Envelope `data` returned by `help`.
///
/// Exactly one of the three fields is populated per call; the others
/// are `None` and skipped during serialization so the wire shape matches
/// `{ nouns?, verbs?, verb? }` from the spec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelpData {
    /// Distinct noun prefixes from the verb registry, sorted ascending.
    /// Populated only for the no-topic branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nouns: Option<Vec<String>>,

    /// Verbs matching a noun-prefix topic (or a bare-name verb topic),
    /// sorted by `name` ascending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbs: Option<Vec<VerbDoc>>,

    /// Single verb doc for a full `<noun>.<verb>` topic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verb: Option<VerbDoc>,
}

/// Verb-level failures for `help`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HelpError {
    /// `args.topic` is empty, names a noun that has no registered
    /// verbs, or names a full verb id that is not in the registry.
    /// Maps to `E_UNKNOWN_TOPIC`.
    #[error("help: unknown topic `{topic}`")]
    UnknownTopic {
        /// Original topic string.
        topic: String,
    },
}

/// Build a `VerbDoc` for a registered verb id. v1 ships `summary` and
/// `args_schema_id` as empty strings (see module docs).
fn verb_doc_for(name: &str) -> VerbDoc {
    VerbDoc {
        name: name.to_string(),
        summary: String::new(),
        args_schema_id: String::new(),
    }
}

/// Build the `help` data envelope for `args`.
fn build_data(args: &HelpArgs) -> Result<HelpData, HelpError> {
    let registry = crate::verbs::default_registry();
    match args.topic.as_deref() {
        None => {
            let mut nouns: BTreeSet<String> = BTreeSet::new();
            for (name, _) in registry.iter() {
                let prefix = name.split_once('.').map_or(name, |(p, _)| p);
                nouns.insert(prefix.to_string());
            }
            Ok(HelpData {
                nouns: Some(nouns.into_iter().collect()),
                verbs: None,
                verb: None,
            })
        }
        Some("") => Err(HelpError::UnknownTopic {
            topic: String::new(),
        }),
        Some(topic) if topic.contains('.') => match registry.get(topic) {
            Some(v) => Ok(HelpData {
                nouns: None,
                verbs: None,
                verb: Some(verb_doc_for(v.verb())),
            }),
            None => Err(HelpError::UnknownTopic {
                topic: topic.to_string(),
            }),
        },
        Some(topic) => {
            let prefix_with_dot = format!("{topic}.");
            let mut matches: Vec<VerbDoc> = registry
                .iter()
                // `*name == topic` matches single-word verbs (e.g. `describe`,
                // `help`, `list_capabilities`) when the user asks for them by
                // bare name — they have no `.` and would not match the prefix
                // filter alone.
                .filter(|(name, _)| name.starts_with(&prefix_with_dot) || *name == topic)
                .map(|(name, _)| verb_doc_for(name))
                .collect();
            if matches.is_empty() {
                return Err(HelpError::UnknownTopic {
                    topic: topic.to_string(),
                });
            }
            matches.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(HelpData {
                nouns: None,
                verbs: Some(matches),
                verb: None,
            })
        }
    }
}

/// Build the (empty) RFC-6902 patch and data envelope for `help`.
///
/// The patch is always `[]` and the warnings vec is always empty — this
/// is a read-only registry-introspection verb.
///
/// # Errors
///
/// Returns [`HelpError::UnknownTopic`] when `topic` is an empty string,
/// names a noun whose `<noun>.*` enumeration is empty AND the noun
/// does not equal any registered bare-name verb, or names a full
/// `<noun>.<verb>` id that is not in the registry.
pub fn compute_patch(
    _prior: &Project,
    args: &HelpArgs,
) -> Result<(Value, Vec<Value>, HelpData), HelpError> {
    let data = build_data(args)?;
    Ok((json!([]), Vec::new(), data))
}

/// Rebuild the data envelope from `(args, post_state)`.
///
/// `help` is project-agnostic, so the post-state is ignored — the
/// envelope depends solely on `args` and the current build's verb
/// registry.
///
/// # Errors
///
/// Returns [`ReconstructError`] when args do not deserialize or the
/// topic fails its registry lookup.
pub fn data_envelope_from_post_state(
    args: &HelpArgs,
    _post_state: &Project,
) -> Result<HelpData, ReconstructError> {
    build_data(args).map_err(|err| ReconstructError::Custom(err.to_string()))
}

impl From<HelpError> for VerbError {
    fn from(value: HelpError) -> Self {
        match value {
            HelpError::UnknownTopic { .. } => VerbError::BadArgs {
                detail: value.to_string(),
            },
        }
    }
}

/// The §0.8 verb for `help`.
#[derive(Debug, Default)]
pub struct HelpVerb;

impl Verb for HelpVerb {
    fn verb(&self) -> &'static str {
        "help"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(json_patch::Patch, Value, Vec<Value>), VerbError> {
        let typed: HelpArgs =
            serde_json::from_value(args.clone()).map_err(|err| VerbError::BadArgs {
                detail: format!("help: args deserialize failed: {err}"),
            })?;

        let (patch_value, warnings, data) = compute_patch(prior, &typed)?;
        let patch: json_patch::Patch = serde_json::from_value(patch_value)
            .map_err(|err| VerbError::Custom(format!("help: patch construction failed: {err}")))?;
        let data = serde_json::to_value(&data)
            .map_err(|err| VerbError::Custom(format!("help: data envelope failed: {err}")))?;

        Ok((patch, data, warnings))
    }

    fn reconstruct(
        &self,
        args: &Value,
        _patch: &Value,
        _warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let typed: HelpArgs =
            serde_json::from_value(args.clone()).map_err(|_| ReconstructError::TypeMismatch {
                name: "args",
                expected: "HelpArgs",
            })?;
        let envelope = data_envelope_from_post_state(&typed, post_state)?;
        serde_json::to_value(&envelope).map_err(|err| ReconstructError::Custom(err.to_string()))
    }
}
