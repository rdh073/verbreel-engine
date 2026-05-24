//! [`Effect`] and its supporting types ([`EffectKind`], [`EffectWindow`]).
//! Mirrors `spec/project-schema.json` `$defs/Effect`.
//!
//! ## Why a separate [`EffectWindow`] struct
//!
//! Schema `$defs/Effect.dependentRequired` says `in_tk` and `out_tk`
//! must appear together — both present, or both absent. A naïve
//! mapping would carry two `Option<Tick>` fields on [`Effect`] and
//! enforce the pairing at the engine layer; that leaves the
//! invalid `Some(in) + None(out)` state representable at the type
//! level. Instead we lift the pairing into [`EffectWindow`] and
//! carry `Option<EffectWindow>` — the type system makes the invalid
//! state unrepresentable. The on-disk JSON shape is unchanged
//! (`in_tk` + `out_tk` at [`Effect`] level, not nested) because the
//! adapter [`EffectSerde`] flattens on serialize and pairs on
//! deserialize.
//!
//! ## Why `EffectKind(String)` and not an enum
//!
//! Schema `$defs/Effect.kind` is `{type: string, minLength: 1}` with
//! no enum constraint. The set of effect kinds is open and
//! runtime-registered — `effect.list_available` is a verb (§6.5) and
//! the type system must not foreclose on which strings are valid.
//! [`EffectKind`] enforces only the `minLength: 1` constraint.
//!
//! ### Managed kinds
//!
//! Three string values have engine-layer special semantics (§6.2 of
//! `spec/commands/effect.md`):
//!
//! - `time_stretch` — owned by `clip.set_speed --preserve_pitch`
//!   (§5.7, §5.15).
//! - `burned_caption` — owned by `caption.burn_in` / `caption.burn_off`
//!   (§10.4, §10.5).
//! - `denoise` — owned by `audio.denoise` (§9.4).
//!
//! Raw `effect.remove` / `effect.set_param` on any of these returns
//! `E_EFFECT_MANAGED`. The enforcement lives at the verb layer — NOT
//! at the type layer. [`EffectKind`] accepts these strings as-is.
//!
//! ## Why `Effect.params` is opaque
//!
//! Schema `$defs/Effect.params` is `additionalProperties: true` with
//! a `maxProperties: 64` cap and a 16 KiB serialized-size cap. The
//! per-kind shape is validated by registered effect schemas, fetched
//! via `effect.list_available` / `schema --target effect --name <kind>`
//! (§1.2, §6.5). At the type layer we carry the bag as
//! `serde_json::Map<String, Value>` and defer the caps to the engine.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;
use verbreel_types::{EffectId, Tick};

// ---------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------

/// Validation error for [`EffectKind`] construction.
#[derive(Debug, Error)]
pub enum EffectNewtypeError {
    /// Empty-string [`EffectKind`] rejected (schema `minLength: 1`).
    #[error("EffectKind requires minLength: 1 — empty string rejected")]
    EmptyKind,
}

// ---------------------------------------------------------------------
// EffectKind
// ---------------------------------------------------------------------

/// Registered effect kind identifier. Schema `$defs/Effect.kind`
/// (`{type: string, minLength: 1}`). See module docs for why this is
/// a newtype-over-String rather than an enum.
///
/// Serialized as a bare JSON string (transparent over the inner
/// [`String`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EffectKind(String);

impl EffectKind {
    /// Construct from a [`String`], rejecting empty input.
    ///
    /// # Errors
    ///
    /// Returns [`EffectNewtypeError::EmptyKind`] if `s` is empty.
    pub fn new(s: String) -> Result<Self, EffectNewtypeError> {
        if s.is_empty() {
            Err(EffectNewtypeError::EmptyKind)
        } else {
            Ok(EffectKind(s))
        }
    }

    /// Borrow the inner string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for EffectKind {
    type Error = EffectNewtypeError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        EffectKind::new(value)
    }
}

impl From<EffectKind> for String {
    fn from(value: EffectKind) -> Self {
        value.0
    }
}

impl std::fmt::Display for EffectKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------
// EffectWindow
// ---------------------------------------------------------------------

/// A paired `(in_tk, out_tk)` time window — both fields present
/// together, neither alone. Lifts schema
/// `$defs/Effect.dependentRequired` into the type system.
///
/// Engine-layer constraints (NOT enforced here):
/// - `in_tk >= 0`
/// - `out_tk > in_tk`
/// - For clip-attached effects: window expressed in parent-relative
///   ticks; for track-attached effects: absolute project time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectWindow {
    /// Effect start tick. Schema `minimum: 0`. Bound checked at
    /// engine layer.
    pub in_tk: Tick,
    /// Effect end tick (exclusive). Schema `minimum: 1`. Engine
    /// invariant `out_tk > in_tk` checked at engine layer.
    pub out_tk: Tick,
}

// ---------------------------------------------------------------------
// Effect (serde adapter pattern)
// ---------------------------------------------------------------------

/// An effect record. Mirrors `spec/project-schema.json` `$defs/Effect`.
///
/// Required fields: `id`, `kind`, `params`. Optional: `enabled`
/// (default `true`), and the paired `window` ([`EffectWindow`]).
///
/// On-disk JSON shape (canonical form — all defaults explicit):
/// ```json
/// {
///   "id": "0190...",
///   "kind": "blur",
///   "enabled": true,
///   "params": {"radius_px": 8},
///   "in_tk": 240000,
///   "out_tk": 720000
/// }
/// ```
///
/// The schema does NOT nest `in_tk` / `out_tk` under a `"window"`
/// object — they appear at the effect level. The serde adapter
/// [`EffectSerde`] handles the flatten on serialize and the
/// `dependentRequired` enforcement on deserialize (one without the
/// other is a serde error mapped to `E_SCHEMA_VIOLATION` at the
/// engine layer).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "EffectSerde", into = "EffectSerde")]
pub struct Effect {
    /// Strongly-typed effect ID.
    pub id: EffectId,
    /// Registered effect kind (schema `minLength: 1`).
    pub kind: EffectKind,
    /// Whether the effect contributes to its parent's output. Default
    /// `true` per schema. Always serialized (canonical form includes
    /// defaults explicitly).
    pub enabled: bool,
    /// Opaque per-kind parameters bag. Validated by registered effect
    /// schemas at the engine layer (`E_EFFECT_BAD_PARAMS`); type layer
    /// keeps it as `serde_json::Map<String, Value>`. Always
    /// serialized (canonical form).
    pub params: Map<String, Value>,
    /// Paired `(in_tk, out_tk)` time window. `None` means the effect
    /// applies to the full duration of its parent.
    pub window: Option<EffectWindow>,
}

impl Effect {
    /// Construct a minimal [`Effect`] — id + kind only, defaults for
    /// the rest (`enabled = true`, empty `params`, no `window`).
    #[must_use]
    pub fn new(id: EffectId, kind: EffectKind) -> Self {
        Self {
            id,
            kind,
            enabled: true,
            params: Map::new(),
            window: None,
        }
    }
}

/// Internal serde adapter shape. Flattens the optional
/// [`EffectWindow`] into the top-level `in_tk` / `out_tk` fields so
/// the on-disk JSON matches the schema (no nested `"window"` object).
///
/// Carries the `dependentRequired` enforcement: parsing fails if
/// exactly one of `in_tk` / `out_tk` is present.
///
/// Not part of the public API. The `try_from` / `into` derive
/// attribute on [`Effect`] is what wires this in.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectSerde {
    id: EffectId,
    kind: EffectKind,
    #[serde(default = "default_enabled")]
    enabled: bool,
    #[serde(default)]
    params: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    in_tk: Option<Tick>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    out_tk: Option<Tick>,
}

fn default_enabled() -> bool {
    true
}

/// Error returned when [`EffectSerde`] parses a record with exactly
/// one of `in_tk` / `out_tk` present.
#[derive(Debug, Error)]
#[error(
    "Effect window violates schema dependentRequired: in_tk and out_tk must both be present or both absent (got in_tk={in_tk_present}, out_tk={out_tk_present})"
)]
pub struct EffectWindowDependencyError {
    in_tk_present: bool,
    out_tk_present: bool,
}

impl TryFrom<EffectSerde> for Effect {
    type Error = EffectWindowDependencyError;
    fn try_from(s: EffectSerde) -> Result<Self, Self::Error> {
        let window = match (s.in_tk, s.out_tk) {
            (Some(in_tk), Some(out_tk)) => Some(EffectWindow { in_tk, out_tk }),
            (None, None) => None,
            (in_tk, out_tk) => {
                return Err(EffectWindowDependencyError {
                    in_tk_present: in_tk.is_some(),
                    out_tk_present: out_tk.is_some(),
                });
            }
        };
        Ok(Effect {
            id: s.id,
            kind: s.kind,
            enabled: s.enabled,
            params: s.params,
            window,
        })
    }
}

impl From<Effect> for EffectSerde {
    fn from(e: Effect) -> Self {
        let (in_tk, out_tk) = match e.window {
            Some(w) => (Some(w.in_tk), Some(w.out_tk)),
            None => (None, None),
        };
        EffectSerde {
            id: e.id,
            kind: e.kind,
            enabled: e.enabled,
            params: e.params,
            in_tk,
            out_tk,
        }
    }
}
