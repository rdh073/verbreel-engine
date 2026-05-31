//! [`Capabilities`] — the agent-discovery catalog.
//!
//! The engine ships a v1-floor `list_capabilities` verb, but its
//! `verbs[]` entries carry empty `summary` / `args_schema_id` strings
//! (the [`verbreel_state::Verb`] trait does not yet expose a summary or
//! schema id). For an agent to *plan*, it needs to know which verbs
//! exist and, ideally, the JSON Schema of each verb's args.
//!
//! This catalog is the AX layer's value-add: it cross-joins the kernel
//! verb registry ([`verbreel_state::default_registry`], the authoritative
//! 116-verb set) with the per-verb JSON Schemas
//! ([`verbreel_args::default_registry`]) and groups by domain. Verbs
//! whose schema has not yet migrated into `verbreel-args` are still
//! listed — with `args_schema: None` — so the catalog is the *complete*
//! verb surface, never a silently-truncated subset.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

/// One verb's discovery metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerbInfo {
    /// Verb id, e.g. `"clip.trim"`.
    pub id: String,
    /// Domain prefix, e.g. `"clip"` (the `<noun>` of `<noun>.<verb>`, or
    /// the whole id for bare verbs like `"help"`).
    pub domain: String,
    /// The verb's args JSON Schema, when one is registered in
    /// `verbreel-args`. `None` means no schema has migrated yet — the
    /// args are still a free-form object the engine validates at apply
    /// time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args_schema: Option<Value>,
}

/// The full engine capability catalog: every registered verb, with its
/// args schema where known, grouped by domain.
///
/// Construct via [`Capabilities::current`]. Cheap to build (one registry
/// walk) but not free — callers that serve it per request should build
/// once and cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Capabilities {
    /// Engine version (`CARGO_PKG_VERSION` of `verbreel-agent`).
    pub engine_version: String,
    /// Engine tick rate in Hz (§0.2 — always `240_000` in v1).
    pub tick_rate_hz: u64,
    /// Every registered verb, sorted by id.
    pub verbs: Vec<VerbInfo>,
}

impl Capabilities {
    /// Build the catalog from the live kernel + args registries.
    #[must_use]
    pub fn current() -> Self {
        let args_registry = verbreel_args::default_registry();
        let mut verbs: Vec<VerbInfo> = verbreel_state::default_registry()
            .verbs()
            .into_iter()
            .map(|id| VerbInfo {
                id: id.to_string(),
                domain: domain_of(id).to_string(),
                args_schema: args_registry.get(id).map(|s| s.as_value().clone()),
            })
            .collect();
        verbs.sort_by(|a, b| a.id.cmp(&b.id));
        Self {
            engine_version: env!("CARGO_PKG_VERSION").to_string(),
            tick_rate_hz: u64::from(verbreel_state::TICK_RATE_HZ),
            verbs,
        }
    }

    /// Whether `verb` is a known verb id in the catalog.
    #[must_use]
    pub fn contains(&self, verb: &str) -> bool {
        self.verbs.iter().any(|v| v.id == verb)
    }

    /// Look up one verb's discovery metadata.
    #[must_use]
    pub fn get(&self, verb: &str) -> Option<&VerbInfo> {
        self.verbs.iter().find(|v| v.id == verb)
    }

    /// Total number of registered verbs.
    #[must_use]
    pub fn verb_count(&self) -> usize {
        self.verbs.len()
    }

    /// Verb ids grouped by domain, each list sorted. Drives the
    /// `verbreel caps` human view and the planner's domain-organized
    /// prompt.
    #[must_use]
    pub fn by_domain(&self) -> BTreeMap<String, Vec<String>> {
        let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for v in &self.verbs {
            grouped
                .entry(v.domain.clone())
                .or_default()
                .push(v.id.clone());
        }
        for ids in grouped.values_mut() {
            ids.sort();
        }
        grouped
    }
}

/// Extract the domain prefix from a verb id: the substring before the
/// first `.`, or the whole id for bare verbs (`help`, `describe`,
/// `list_capabilities`, `schema`, `validate_command`).
fn domain_of(verb: &str) -> &str {
    verb.split_once('.').map_or(verb, |(noun, _)| noun)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_the_full_registry() {
        let caps = Capabilities::current();
        // Mirrors the kernel registry exactly — never a truncated subset.
        assert_eq!(
            caps.verb_count(),
            verbreel_state::default_registry().verbs().len()
        );
        // The 116-verb floor mapped earlier — assert we are at least at
        // that scale so a registry regression is loud.
        assert!(caps.verb_count() >= 116, "got {}", caps.verb_count());
    }

    #[test]
    fn known_editing_verbs_are_present() {
        let caps = Capabilities::current();
        for verb in [
            "clip.trim",
            "clip.split",
            "caption.auto_generate",
            "render.queue.add",
            "timeline.undo",
            "project.info",
        ] {
            assert!(caps.contains(verb), "missing {verb}");
        }
    }

    #[test]
    fn domain_is_the_noun_prefix() {
        assert_eq!(domain_of("clip.trim"), "clip");
        assert_eq!(domain_of("preview.session.create"), "preview");
        assert_eq!(domain_of("help"), "help");
    }

    #[test]
    fn well_known_verbs_carry_a_schema() {
        let caps = Capabilities::current();
        // clip.list has a hand-curated schema in verbreel-args.
        let clip_list = caps.get("clip.list").expect("clip.list present");
        assert!(
            clip_list.args_schema.is_some(),
            "clip.list should expose its args schema"
        );
    }

    #[test]
    fn grouping_by_domain_is_sorted_and_complete() {
        let caps = Capabilities::current();
        let grouped = caps.by_domain();
        let total: usize = grouped.values().map(Vec::len).sum();
        assert_eq!(total, caps.verb_count());
        // The clip domain holds the bulk of editing verbs.
        let clip = grouped.get("clip").expect("clip domain present");
        assert!(clip.contains(&"clip.trim".to_string()));
        // Each domain's ids are sorted.
        for ids in grouped.values() {
            let mut sorted = ids.clone();
            sorted.sort();
            assert_eq!(*ids, sorted);
        }
    }
}
