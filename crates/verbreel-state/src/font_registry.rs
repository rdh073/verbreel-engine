//! Canonical font registry used by `font.list`, `text.add`, and `text.style`.
//!
//! Invariant: every font-family lookup must resolve against this single
//! registry before a text mutation enters state.

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use fontdb::{Database, Source};
use serde::{Deserialize, Serialize};

const BUNDLED_INTER_TTF: &[u8] = include_bytes!("../assets/fonts/Inter-Variable.ttf");

/// Registry source classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RegistrySource {
    /// Found in bundled engine assets.
    Bundled,
    /// Found from host system scan.
    System,
}

/// Canonical family row exposed by `font.list` and lookup errors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryFamily {
    /// Human-facing family name.
    pub name: String,
    /// Source bucket.
    pub source: RegistrySource,
    /// Optional source path (available for filesystem-backed faces).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Cached canonical registry snapshot.
#[derive(Debug)]
pub struct Registry {
    families: Vec<RegistryFamily>,
    by_key: HashMap<String, usize>,
    available: Vec<String>,
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();

/// Scan and cache the canonical registry.
#[must_use]
pub fn scan() -> &'static Registry {
    REGISTRY.get_or_init(build_registry)
}

/// List canonical families in deterministic order.
#[must_use]
pub fn list() -> Vec<RegistryFamily> {
    scan().families.clone()
}

/// Resolve a family name against the canonical registry.
#[must_use]
pub fn resolve(family: &str) -> Option<RegistryFamily> {
    let registry = scan();
    let key = canonical_key(family);
    registry
        .by_key
        .get(&key)
        .and_then(|idx| registry.families.get(*idx))
        .cloned()
}

/// Available family names for diagnostics.
#[must_use]
pub fn available() -> Vec<String> {
    scan().available.clone()
}

fn build_registry() -> Registry {
    let mut db = Database::new();
    db.load_font_data(BUNDLED_INTER_TTF.to_vec());
    db.load_system_fonts();

    let mut by_key = BTreeMap::<String, RegistryFamily>::new();
    for face in db.faces() {
        for (name, _) in &face.families {
            let key = canonical_key(name);
            if key.is_empty() {
                continue;
            }

            let row = RegistryFamily {
                name: name.clone(),
                source: classify_source(&face.source),
                path: None,
            };
            upsert_family(&mut by_key, key, row);
        }
    }

    let families: Vec<RegistryFamily> = by_key.into_values().collect();
    let mut by_resolve_key = HashMap::new();
    let mut available = Vec::with_capacity(families.len());
    for (idx, family) in families.iter().enumerate() {
        by_resolve_key.insert(canonical_key(&family.name), idx);
        available.push(family.name.clone());
    }

    Registry {
        families,
        by_key: by_resolve_key,
        available,
    }
}

fn upsert_family(
    rows: &mut BTreeMap<String, RegistryFamily>,
    key: String,
    candidate: RegistryFamily,
) {
    match rows.get_mut(&key) {
        None => {
            rows.insert(key, candidate);
        }
        Some(current) => {
            if current.source == RegistrySource::System
                && candidate.source == RegistrySource::Bundled
            {
                *current = candidate;
                return;
            }
            if current.path.is_none() && candidate.path.is_some() {
                current.path = candidate.path;
            }
        }
    }
}

fn classify_source(source: &Source) -> RegistrySource {
    match source {
        Source::Binary(_) => RegistrySource::Bundled,
        Source::File(_) | Source::SharedFile(_, _) => RegistrySource::System,
    }
}

fn canonical_key(name: &str) -> String {
    name.split_whitespace()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
