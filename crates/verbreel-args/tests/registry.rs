//! Tests for [`verbreel_args::ArgsRegistry`].
//!
//! Surface under test:
//!
//! - [`ArgsRegistry::new`] / [`ArgsRegistry::default`] — empty start.
//! - [`ArgsRegistry::register`] — insert + return prior entry on
//!   duplicate.
//! - [`ArgsRegistry::get`] — lookup by `&str`.
//! - [`ArgsRegistry::len`] / [`ArgsRegistry::is_empty`].
//! - [`ArgsRegistry::iter`] — all-pair iteration.

use std::collections::HashSet;

use serde_json::json;
use verbreel_args::{ArgsRegistry, Schema};

fn schema(raw: serde_json::Value) -> Schema {
    Schema::from_value(raw).expect("test schema must compile")
}

// --- empty registry ------------------------------------------------------

#[test]
fn new_registry_is_empty() {
    let r = ArgsRegistry::new();
    assert!(r.is_empty());
    assert_eq!(r.len(), 0);
}

#[test]
fn default_registry_is_empty() {
    let r = ArgsRegistry::default();
    assert!(r.is_empty());
    assert_eq!(r.len(), 0);
}

#[test]
fn get_on_empty_registry_returns_none() {
    let r = ArgsRegistry::new();
    assert!(r.get("anything").is_none());
}

#[test]
fn iter_on_empty_registry_yields_no_items() {
    let r = ArgsRegistry::new();
    assert_eq!(r.iter().count(), 0);
}

// --- single-entry registry ----------------------------------------------

#[test]
fn register_new_verb_returns_none() {
    let mut r = ArgsRegistry::new();
    let prev = r.register("project.create", schema(json!({"type": "object"})));
    assert!(
        prev.is_none(),
        "first registration must not displace anything"
    );
}

#[test]
fn after_register_len_is_one() {
    let mut r = ArgsRegistry::new();
    r.register("project.create", schema(json!({"type": "object"})));
    assert_eq!(r.len(), 1);
    assert!(!r.is_empty());
}

#[test]
fn get_returns_some_after_register() {
    let mut r = ArgsRegistry::new();
    r.register("project.create", schema(json!({"type": "object"})));
    assert!(r.get("project.create").is_some());
}

#[test]
fn get_uses_str_key_not_static_str() {
    // Lookup must work with a `&str` of any lifetime, not require a
    // `&'static str`. Construct the key at runtime to be sure.
    let mut r = ArgsRegistry::new();
    r.register("clip.add", schema(json!({"type": "object"})));

    let dyn_key: String = "clip.add".to_owned();
    assert!(r.get(dyn_key.as_str()).is_some());
}

// --- multi-entry registry ----------------------------------------------

#[test]
fn register_multiple_distinct_verbs_grows_len() {
    let mut r = ArgsRegistry::new();
    r.register("a", schema(json!({"type": "object"})));
    r.register("b", schema(json!({"type": "object"})));
    r.register("c", schema(json!({"type": "object"})));
    assert_eq!(r.len(), 3);
}

#[test]
fn get_returns_distinct_schemas_per_verb() {
    let mut r = ArgsRegistry::new();
    r.register("verb.a", schema(json!({"type": "string"})));
    r.register("verb.b", schema(json!({"type": "integer"})));

    let a = r.get("verb.a").unwrap();
    let b = r.get("verb.b").unwrap();
    assert_eq!(a.as_value(), &json!({"type": "string"}));
    assert_eq!(b.as_value(), &json!({"type": "integer"}));
}

#[test]
fn iter_yields_every_registered_pair() {
    let mut r = ArgsRegistry::new();
    r.register("alpha", schema(json!({"type": "object"})));
    r.register("beta", schema(json!({"type": "object"})));
    r.register("gamma", schema(json!({"type": "object"})));

    let seen: HashSet<&'static str> = r.iter().map(|(k, _)| k).collect();
    let expected: HashSet<&'static str> = ["alpha", "beta", "gamma"].into_iter().collect();
    assert_eq!(seen, expected);
}

#[test]
fn get_with_unknown_verb_returns_none_even_when_others_present() {
    let mut r = ArgsRegistry::new();
    r.register("known", schema(json!({"type": "object"})));
    assert!(r.get("not-known").is_none());
}

// --- duplicate registration --------------------------------------------

#[test]
fn duplicate_register_returns_previous_schema() {
    let mut r = ArgsRegistry::new();
    let prev = r.register("dup", schema(json!({"type": "string"})));
    assert!(prev.is_none());

    let prev2 = r.register("dup", schema(json!({"type": "integer"})));
    let prev2 = prev2.expect("second register must hand back first schema");
    assert_eq!(prev2.as_value(), &json!({"type": "string"}));
}

#[test]
fn duplicate_register_overwrites_in_place() {
    let mut r = ArgsRegistry::new();
    r.register("dup", schema(json!({"type": "string"})));
    r.register("dup", schema(json!({"type": "integer"})));

    let now = r.get("dup").unwrap();
    assert_eq!(now.as_value(), &json!({"type": "integer"}));
}

#[test]
fn duplicate_register_does_not_grow_len() {
    let mut r = ArgsRegistry::new();
    r.register("x", schema(json!({"type": "string"})));
    r.register("x", schema(json!({"type": "integer"})));
    r.register("x", schema(json!({"type": "boolean"})));
    assert_eq!(r.len(), 1);
}
