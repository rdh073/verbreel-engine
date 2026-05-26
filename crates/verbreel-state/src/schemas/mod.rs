//! Embedded JSON schemas. Files in this directory are verbatim copies
//! from `~/playground/verbreel-spec/spec/` and MUST be kept in sync
//! with the spec repo. The `schema` verb (§1.2) returns these.

/// Canonical project graph schema (RFC 8785 conformant, JSON Schema 2020-12).
pub const PROJECT_JSON: &str = include_str!("project.json");
