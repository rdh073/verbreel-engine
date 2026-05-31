//! [`AnthropicClient`] — the live [`LlmClient`] leg (feature `claude`).
//!
//! A thin blocking wrapper over the Anthropic Messages API
//! (`POST /v1/messages`). Blocking (not async) keeps the planner
//! synchronous and matches the engine's existing out-of-process transport
//! style (`verbreel-ai`'s `std::process` sidecar). This module is the
//! *only* place `reqwest` is touched; the planner logic in
//! [`crate::planner`] never sees it (dependency inversion).
//!
//! Configuration comes from the environment so no secret is ever
//! compiled in:
//!
//! | Variable                | Purpose                         | Default                        |
//! |-------------------------|---------------------------------|--------------------------------|
//! | `ANTHROPIC_API_KEY`     | Auth (required)                 | — (else [`PlannerError::NotConfigured`]) |
//! | `VERBREEL_PLANNER_MODEL`| Model id                        | `claude-sonnet-4-6`            |
//! | `ANTHROPIC_BASE_URL`    | API base (proxies / testing)    | `https://api.anthropic.com`    |

use serde_json::{Value, json};

use crate::error::PlannerError;
use crate::planner::LlmClient;

/// Anthropic Messages-API version pin (the `anthropic-version` header).
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Default planner model when `VERBREEL_PLANNER_MODEL` is unset.
const DEFAULT_MODEL: &str = "claude-sonnet-4-6";
/// Default API base when `ANTHROPIC_BASE_URL` is unset.
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
/// Generous ceiling for a plan reply.
const MAX_TOKENS: u32 = 4096;

/// A blocking Anthropic Messages-API client.
pub struct AnthropicClient {
    http: reqwest::blocking::Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl AnthropicClient {
    /// Construct from the environment.
    ///
    /// # Errors
    ///
    /// Returns [`PlannerError::NotConfigured`] when `ANTHROPIC_API_KEY`
    /// is unset, and [`PlannerError::Transport`] if the HTTP client cannot
    /// be built.
    pub fn from_env() -> Result<Self, PlannerError> {
        let api_key =
            std::env::var("ANTHROPIC_API_KEY").map_err(|_| PlannerError::NotConfigured {
                detail:
                    "set ANTHROPIC_API_KEY to use the Claude planner, or pass a plan file with \
                     `--plan`"
                        .to_string(),
            })?;
        let model =
            std::env::var("VERBREEL_PLANNER_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        let base_url =
            std::env::var("ANTHROPIC_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        let http =
            reqwest::blocking::Client::builder()
                .build()
                .map_err(|e| PlannerError::Transport {
                    detail: format!("could not build HTTP client: {e}"),
                })?;
        Ok(Self {
            http,
            api_key,
            model,
            base_url,
        })
    }

    /// The model id this client will call.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }
}

impl LlmClient for AnthropicClient {
    fn complete(&self, system: &str, user: &str) -> Result<String, PlannerError> {
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let body = json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "system": system,
            "messages": [{ "role": "user", "content": user }],
        });

        let resp = self
            .http
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .map_err(|e| PlannerError::Transport {
                detail: format!("request to {url} failed: {e}"),
            })?;

        let status = resp.status();
        let text = resp.text().map_err(|e| PlannerError::Transport {
            detail: format!("reading response body failed: {e}"),
        })?;
        if !status.is_success() {
            return Err(PlannerError::Transport {
                detail: format!("Anthropic API returned {status}: {text}"),
            });
        }

        let value: Value = serde_json::from_str(&text).map_err(|e| PlannerError::Transport {
            detail: format!("response was not JSON: {e}"),
        })?;
        extract_text(&value)
    }
}

/// Concatenate the `text` of every text block in a Messages-API
/// response `content[]`.
fn extract_text(value: &Value) -> Result<String, PlannerError> {
    let blocks = value["content"]
        .as_array()
        .ok_or_else(|| PlannerError::Transport {
            detail: "response had no `content` array".to_string(),
        })?;
    let text: String = blocks
        .iter()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect();
    if text.is_empty() {
        return Err(PlannerError::Transport {
            detail: "response `content` carried no text block".to_string(),
        });
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_text_concatenates_text_blocks() {
        let resp = json!({
            "content": [
                { "type": "text", "text": "{\"steps\":" },
                { "type": "text", "text": "[]}" }
            ]
        });
        assert_eq!(extract_text(&resp).expect("text"), "{\"steps\":[]}");
    }

    #[test]
    fn extract_text_errors_without_content() {
        let resp = json!({ "id": "x" });
        assert!(matches!(
            extract_text(&resp),
            Err(PlannerError::Transport { .. })
        ));
    }

    #[test]
    fn extract_text_errors_on_empty_text() {
        let resp = json!({ "content": [{ "type": "tool_use", "name": "x" }] });
        assert!(matches!(
            extract_text(&resp),
            Err(PlannerError::Transport { .. })
        ));
    }
}
