//! Fixture validates against the canonical `spec/project-schema.json`.
//!
//! Schema discovery (in priority order):
//! 1. Env `VERBREEL_SPEC_DIR` → `$VERBREEL_SPEC_DIR/spec/project-schema.json`.
//! 2. Walk up from `CARGO_MANIFEST_DIR` looking for a sibling
//!    `verbreel-spec` directory.
//! 3. If neither resolves, **skip** the test with a `println!`. This
//!    avoids gating local dev on a sibling checkout; CI sets the env
//!    var explicitly.

use std::path::{Path, PathBuf};

use serde_json::Value;
use verbreel_state::Project;

const FIXTURE: &str = include_str!("fixtures/empty_project_create.json");

/// Try to locate `project-schema.json`. Returns `None` if neither the
/// env var nor the walk-up fallback finds it.
fn find_schema() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("VERBREEL_SPEC_DIR") {
        let p = PathBuf::from(dir).join("spec").join("project-schema.json");
        if p.is_file() {
            return Some(p);
        }
    }
    // Walk up from CARGO_MANIFEST_DIR (the crate dir at test time)
    // until we find a sibling `verbreel-spec` checkout.
    let start = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut cur: &Path = &start;
    loop {
        let candidate = cur
            .join("..")
            .join("verbreel-spec")
            .join("spec")
            .join("project-schema.json");
        if let Ok(canonical) = candidate.canonicalize()
            && canonical.is_file()
        {
            return Some(canonical);
        }
        match cur.parent() {
            Some(parent) => cur = parent,
            None => return None,
        }
    }
}

#[test]
fn schema_validates_empty_project() {
    let Some(schema_path) = find_schema() else {
        println!(
            "skipping schema_validates_empty_project: VERBREEL_SPEC_DIR unset and no sibling \
             verbreel-spec checkout found (this is expected on workspaces without the spec repo \
             cloned)."
        );
        return;
    };

    let schema_text = std::fs::read_to_string(&schema_path)
        .unwrap_or_else(|e| panic!("read schema at {}: {e}", schema_path.display()));
    let schema: Value =
        serde_json::from_str(&schema_text).expect("project-schema.json must parse as JSON");

    let validator =
        jsonschema::validator_for(&schema).expect("project-schema.json must compile as a schema");

    // Path 1: validate the hand-written fixture.
    let fixture_value: Value = serde_json::from_str(FIXTURE).expect("fixture must parse as JSON");
    if let Err(errors) = validator.validate(&fixture_value) {
        panic!("fixture must validate against project-schema.json: {errors}");
    }

    // Path 2: validate a freshly-`to_value`'d Project. This catches any
    // drift between the hand-written fixture and the typed shape's
    // serialize output (e.g. wrong default values for optional
    // Canvas/Track fields, lossy field renames).
    let project: Project = serde_json::from_str(FIXTURE).expect("fixture → Project");
    let typed_value =
        serde_json::to_value(&project).expect("Project must serialize to serde_json::Value");
    if let Err(errors) = validator.validate(&typed_value) {
        panic!("typed Project re-serialized must validate against project-schema.json: {errors}");
    }
}
