//! [`Plan`] / [`VerbCall`] — the ordered verb sequence an agent produces
//! from an intent, plus structural validation against the capability
//! catalog.
//!
//! A plan is the contract between *the thing that decides what to do*
//! (a [`crate::Planner`], whether an LLM or a human authoring JSON) and
//! *the thing that does it* ([`crate::Session::apply_plan`]). Keeping it
//! a plain serde type means the same plan can come from a file
//! (`verbreel edit --plan plan.json`), an HTTP request body, or a model
//! reply, and the dispatch path is identical.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::capabilities::Capabilities;
use crate::error::AgentError;

/// One step of a [`Plan`]: a verb id plus its argument object.
///
/// `args` is a free-form [`Value`] (validated to be a JSON object at
/// plan-validation time) — the same shape every kernel verb's
/// `compute_patch` consumes. `project_id` is *not* required here; the
/// [`crate::Session`] injects the open project's id when it is absent so
/// agents author plans without threading ids by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerbCall {
    /// The verb id to invoke (e.g. `"clip.trim"`).
    pub verb: String,
    /// The verb's argument object. Defaults to an empty object when a
    /// step omits it (some verbs take only the injected `project_id`).
    #[serde(default = "empty_object")]
    pub args: Value,
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

/// An ordered, validated sequence of verb calls plus an optional
/// rationale (the planner's natural-language explanation, surfaced to
/// the user but never executed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// The steps to apply, in order.
    pub steps: Vec<VerbCall>,
    /// Optional human-readable explanation of why these steps realize
    /// the intent. Informational only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

impl Plan {
    /// Construct a plan from steps with no rationale.
    #[must_use]
    pub fn new(steps: Vec<VerbCall>) -> Self {
        Self {
            steps,
            rationale: None,
        }
    }

    /// Parse a plan from a JSON document.
    ///
    /// Accepts either a full `{ "steps": [...], "rationale"?: ... }`
    /// object or a bare `[ {verb, args}, ... ]` array (the latter is the
    /// shape models most readily emit). A bare array becomes a plan with
    /// no rationale.
    ///
    /// # Errors
    ///
    /// Returns [`serde_json::Error`] when `json` is neither a plan object
    /// nor a step array.
    pub fn from_json(json: &Value) -> Result<Self, serde_json::Error> {
        if json.is_array() {
            let steps: Vec<VerbCall> = serde_json::from_value(json.clone())?;
            Ok(Self::new(steps))
        } else {
            serde_json::from_value(json.clone())
        }
    }

    /// Validate every step against `caps` *before any step runs*.
    ///
    /// Two structural checks per step, in order:
    /// 1. `args` must be a JSON object (the [`verbreel_state::Verb`]
    ///    contract) — a string/array/number step body is rejected.
    /// 2. `verb` must be a known verb id in the capability catalog.
    ///
    /// Validating up front is what makes [`crate::Session::apply_plan`]
    /// all-or-nothing at the *shape* level: a typo'd verb id in step 5
    /// fails the whole plan before step 1 mutates anything. (Per-verb
    /// arg-schema and §0.13 invariant violations still surface at apply
    /// time — those need the live project state.)
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::InvalidPlan`] naming the first offending
    /// step index.
    pub fn validate(&self, caps: &Capabilities) -> Result<(), AgentError> {
        for (index, step) in self.steps.iter().enumerate() {
            if !step.args.is_object() {
                return Err(AgentError::InvalidPlan {
                    index,
                    detail: format!(
                        "step args must be a JSON object, got {}",
                        json_type_name(&step.args)
                    ),
                });
            }
            if !caps.contains(&step.verb) {
                return Err(AgentError::InvalidPlan {
                    index,
                    detail: format!("unknown verb {:?}", step.verb),
                });
            }
        }
        Ok(())
    }

    /// Number of steps in the plan.
    #[must_use]
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Whether the plan has no steps.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// Human-readable JSON type name for error messages.
fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_full_plan_object() {
        let plan = Plan::from_json(&json!({
            "steps": [{ "verb": "clip.trim", "args": { "clip": "c1" } }],
            "rationale": "trim the intro"
        }))
        .expect("valid plan object");
        assert_eq!(plan.len(), 1);
        assert_eq!(plan.steps[0].verb, "clip.trim");
        assert_eq!(plan.rationale.as_deref(), Some("trim the intro"));
    }

    #[test]
    fn parses_bare_step_array() {
        let plan = Plan::from_json(&json!([
            { "verb": "clip.trim", "args": {} },
            { "verb": "clip.split" }
        ]))
        .expect("valid step array");
        assert_eq!(plan.len(), 2);
        // Omitted args default to an empty object.
        assert_eq!(plan.steps[1].args, json!({}));
    }

    #[test]
    fn validate_rejects_non_object_args() {
        let caps = Capabilities::current();
        let plan = Plan::new(vec![VerbCall {
            verb: "clip.trim".to_string(),
            args: json!("not an object"),
        }]);
        let err = plan.validate(&caps).expect_err("non-object args rejected");
        assert!(matches!(err, AgentError::InvalidPlan { index: 0, .. }));
    }

    #[test]
    fn validate_rejects_unknown_verb() {
        let caps = Capabilities::current();
        let plan = Plan::new(vec![
            VerbCall {
                verb: "clip.trim".to_string(),
                args: json!({}),
            },
            VerbCall {
                verb: "clip.teleport".to_string(),
                args: json!({}),
            },
        ]);
        let err = plan.validate(&caps).expect_err("unknown verb rejected");
        match err {
            AgentError::InvalidPlan { index, detail } => {
                assert_eq!(index, 1);
                assert!(detail.contains("clip.teleport"));
            }
            other => panic!("expected InvalidPlan, got {other:?}"),
        }
    }

    #[test]
    fn validate_accepts_known_verbs() {
        let caps = Capabilities::current();
        let plan = Plan::new(vec![VerbCall {
            verb: "project.info".to_string(),
            args: json!({}),
        }]);
        plan.validate(&caps).expect("known verb accepted");
    }
}
