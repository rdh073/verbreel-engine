//! [`Planner`] — turn a natural-language intent into a [`Plan`].
//!
//! The planning concern is split from its transport so the *logic*
//! (prompt construction from the capability catalog, extracting a JSON
//! plan from a model reply, parsing it) is unit-tested against a mock and
//! the *live leg* (an Anthropic Messages-API call) is a single
//! feature-gated [`LlmClient`] impl ([`crate::anthropic::AnthropicClient`]).
//!
//! Dependency inversion: [`LlmPlanner`] depends on the [`LlmClient`]
//! abstraction, never on `reqwest`. The composition root (the CLI / HTTP
//! server) injects the concrete client.

use serde_json::Value;

use crate::capabilities::Capabilities;
use crate::error::PlannerError;
use crate::plan::Plan;

/// Produces a [`Plan`] from a natural-language `intent` and the engine
/// capability catalog.
pub trait Planner {
    /// Plan the verb sequence that realizes `intent`.
    ///
    /// # Errors
    ///
    /// Returns [`PlannerError`] when the transport fails, the reply is
    /// unparseable, or the planner is not configured.
    fn plan(&self, intent: &str, caps: &Capabilities) -> Result<Plan, PlannerError>;
}

/// A text-in / text-out large-language-model transport.
///
/// The single seam the live planner crosses. Implemented by
/// [`crate::anthropic::AnthropicClient`] (feature `claude`) for
/// production and by a mock in tests.
pub trait LlmClient {
    /// Send a system + user prompt, return the model's reply text.
    ///
    /// # Errors
    ///
    /// Returns [`PlannerError::Transport`] (network / auth / non-2xx) or
    /// [`PlannerError::NotConfigured`] (missing credentials).
    fn complete(&self, system: &str, user: &str) -> Result<String, PlannerError>;
}

/// An [`LlmClient`]-backed [`Planner`].
///
/// Generic over the transport so the same planning logic serves the live
/// Anthropic client and the test mock.
pub struct LlmPlanner<C: LlmClient> {
    client: C,
}

impl<C: LlmClient> LlmPlanner<C> {
    /// Wrap an [`LlmClient`] in a planner.
    pub fn new(client: C) -> Self {
        Self { client }
    }
}

impl<C: LlmClient> Planner for LlmPlanner<C> {
    fn plan(&self, intent: &str, caps: &Capabilities) -> Result<Plan, PlannerError> {
        let system = build_system_prompt(caps);
        let reply = self.client.complete(&system, intent)?;
        let value = extract_json(&reply).map_err(|detail| PlannerError::BadReply { detail })?;
        Plan::from_json(&value).map_err(|e| PlannerError::BadReply {
            detail: format!("reply JSON was not a plan: {e}"),
        })
    }
}

/// Build the planner system prompt from the capability catalog.
///
/// Lists every verb id grouped by domain (so the model sees the full
/// surface) and appends the JSON Schemas for the verbs that carry one.
/// Pins the output contract to a bare JSON plan so the reply is
/// machine-parseable.
#[must_use]
pub fn build_system_prompt(caps: &Capabilities) -> String {
    let mut s = String::new();
    s.push_str(
        "You are the planner for Verbreel, a verb-based video editor. Given a user's \
         editing intent, output a PLAN: an ordered list of engine verb calls that realizes \
         the intent.\n\n\
         Reply with ONLY a single JSON object, no prose, no code fences:\n\
         {\"steps\": [{\"verb\": \"<id>\", \"args\": { ... }}, ...], \"rationale\": \"<one sentence>\"}\n\n\
         Rules:\n\
         - Use ONLY verb ids from the catalog below.\n\
         - Omit `project_id` from args; the engine injects it.\n\
         - Time fields are engine ticks at 240000 Hz (1 second = 240000 ticks).\n\
         - Prefer the fewest steps that achieve the intent.\n\
         - If the intent is impossible with the available verbs, return an empty steps list \
           and explain why in `rationale`.\n\n\
         VERB CATALOG (by domain):\n",
    );
    for (domain, ids) in caps.by_domain() {
        s.push_str("- ");
        s.push_str(&domain);
        s.push_str(": ");
        s.push_str(&ids.join(", "));
        s.push('\n');
    }
    s.push_str("\nARGS SCHEMAS (verbs not listed take a free-form args object):\n");
    for verb in &caps.verbs {
        if let Some(schema) = &verb.args_schema {
            s.push_str(&verb.id);
            s.push_str(": ");
            s.push_str(&schema.to_string());
            s.push('\n');
        }
    }
    s
}

/// Extract the first JSON value (object or array) from a model reply.
///
/// Tolerates the common wrappers models add: leading/trailing prose,
/// ` ```json ` code fences. Tries a direct parse first, then falls back
/// to slicing from the first `{`/`[` to its matching close.
fn extract_json(reply: &str) -> Result<Value, String> {
    let trimmed = strip_code_fence(reply.trim());

    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        return Ok(v);
    }

    // Fallback: find the first JSON container and parse the balanced span.
    let (open, close) = match trimmed.find(['{', '[']) {
        Some(i) if trimmed.as_bytes()[i] == b'{' => (i, '}'),
        Some(i) => (i, ']'),
        None => return Err("no JSON object or array in reply".to_string()),
    };
    let end = trimmed
        .rfind(close)
        .ok_or_else(|| format!("unbalanced JSON: no closing '{close}'"))?;
    if end < open {
        return Err("unbalanced JSON: close before open".to_string());
    }
    serde_json::from_str::<Value>(&trimmed[open..=end])
        .map_err(|e| format!("could not parse JSON span: {e}"))
}

/// Strip a leading/trailing Markdown code fence (```json … ```), if any.
fn strip_code_fence(s: &str) -> &str {
    let Some(rest) = s.strip_prefix("```") else {
        return s;
    };
    // Drop the optional language tag on the opening fence line.
    let rest = rest.split_once('\n').map_or(rest, |(_, body)| body);
    rest.strip_suffix("```").unwrap_or(rest).trim()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Records the prompts it was given and returns a canned reply.
    struct MockLlm {
        reply: String,
        seen_system: RefCell<String>,
    }

    impl MockLlm {
        fn new(reply: impl Into<String>) -> Self {
            Self {
                reply: reply.into(),
                seen_system: RefCell::new(String::new()),
            }
        }
    }

    impl LlmClient for MockLlm {
        fn complete(&self, system: &str, _user: &str) -> Result<String, PlannerError> {
            *self.seen_system.borrow_mut() = system.to_string();
            Ok(self.reply.clone())
        }
    }

    #[test]
    fn system_prompt_lists_the_full_verb_surface() {
        let caps = Capabilities::current();
        let prompt = build_system_prompt(&caps);
        assert!(prompt.contains("clip.trim"));
        assert!(prompt.contains("render.queue.add"));
        // Schemas section carries the well-known schemas.
        assert!(prompt.contains("ARGS SCHEMAS"));
        assert!(prompt.contains("clip.list"));
    }

    #[test]
    fn plans_from_a_clean_json_reply() {
        let caps = Capabilities::current();
        let planner = LlmPlanner::new(MockLlm::new(
            r#"{"steps":[{"verb":"project.rename","args":{"name":"x"}}],"rationale":"rename"}"#,
        ));
        let plan = planner
            .plan("rename the project to x", &caps)
            .expect("plans");
        assert_eq!(plan.len(), 1);
        assert_eq!(plan.steps[0].verb, "project.rename");
        assert_eq!(plan.rationale.as_deref(), Some("rename"));
    }

    #[test]
    fn tolerates_code_fences_and_prose() {
        let caps = Capabilities::current();
        let reply = "Here is the plan:\n```json\n{\"steps\":[{\"verb\":\"clip.split\",\"args\":{}}]}\n```\nDone.";
        let planner = LlmPlanner::new(MockLlm::new(reply));
        let plan = planner.plan("split the clip", &caps).expect("plans");
        assert_eq!(plan.steps[0].verb, "clip.split");
    }

    #[test]
    fn tolerates_bare_array_reply() {
        let caps = Capabilities::current();
        let planner = LlmPlanner::new(MockLlm::new(
            r#"[{"verb":"clip.trim","args":{"clip":"c1"}}]"#,
        ));
        let plan = planner.plan("trim c1", &caps).expect("plans");
        assert_eq!(plan.len(), 1);
    }

    #[test]
    fn unparseable_reply_is_a_bad_reply_error() {
        let caps = Capabilities::current();
        let planner = LlmPlanner::new(MockLlm::new("I cannot help with that."));
        let err = planner.plan("do something", &caps).expect_err("bad reply");
        assert!(matches!(err, PlannerError::BadReply { .. }));
    }

    #[test]
    fn extract_json_handles_plain_object() {
        let v = extract_json(r#"{"a":1}"#).expect("parses");
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn extract_json_handles_surrounding_text() {
        let v = extract_json("blah blah {\"a\":1} trailing").expect("parses");
        assert_eq!(v["a"], 1);
    }
}
