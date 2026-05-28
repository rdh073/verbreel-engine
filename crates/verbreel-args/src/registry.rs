//! [`ArgsRegistry`] — the verb-name → [`Schema`] lookup table.
//!
//! One [`ArgsRegistry`] is built at engine start-up: each verb crate
//! contributes its `Args` schema via [`ArgsRegistry::register`], and
//! the dispatcher hands the resulting registry to
//! [`crate::validate`] on every `apply` call.
//!
//! The registry is keyed by `&'static str` — verb names are
//! compile-time string literals, not runtime-generated, so we save the
//! allocation and the `Hash` cost of a `String` key. Insertion order
//! is NOT preserved (callers iterate the registry only for
//! debug/introspection, where ordering does not matter); if a future
//! caller needs ordered iteration, swap the inner map for `IndexMap`
//! per the rule in `CLAUDE.md`.

use std::collections::HashMap;

use crate::schema::Schema;

/// Verb-name → [`Schema`] registry. Backs [`crate::validate`].
///
/// Construct via [`ArgsRegistry::new`] (empty) or
/// [`ArgsRegistry::default`] (alias). Register schemas with
/// [`ArgsRegistry::register`].
///
/// # Duplicate keys
///
/// Re-registering the same verb name overwrites the previous schema
/// and returns the prior entry via the `Option` in
/// [`ArgsRegistry::register`]'s return value. This lets test code swap
/// schemas in place without juggling a side-table; production callers
/// can `assert!` the return is `None` if they want a one-shot guarantee.
#[derive(Debug, Default)]
pub struct ArgsRegistry {
    inner: HashMap<&'static str, Schema>,
}

impl ArgsRegistry {
    /// Build an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }

    /// Register `schema` under `verb`. Returns the previously-registered
    /// [`Schema`] if one existed (i.e. this call overwrote it), else
    /// `None`.
    pub fn register(&mut self, verb: &'static str, schema: Schema) -> Option<Schema> {
        self.inner.insert(verb, schema)
    }

    /// Look up the [`Schema`] registered for `verb`.
    ///
    /// Returns `None` if no schema has been registered under that
    /// name. Callers that want this turned into a typed error should
    /// use [`crate::validate`].
    #[must_use]
    pub fn get(&self, verb: &str) -> Option<&Schema> {
        self.inner.get(verb)
    }

    /// Number of registered verbs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// `true` iff no verbs have been registered yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Iterate over all `(verb, schema)` pairs.
    ///
    /// Iteration order is unspecified — see the type-level
    /// documentation for the rationale. The verb name is yielded by
    /// value (`&'static str` is `Copy`) so callers do not have to
    /// double-dereference.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &Schema)> {
        self.inner.iter().map(|(k, v)| (*k, v))
    }
}
