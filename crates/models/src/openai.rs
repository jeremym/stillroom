//! Any OpenAI-compatible chat service by API key: OpenAI itself, or a local server such as
//! LM Studio, llama-server, or Ollama's `/v1`. Chat Completions only, because that is the
//! shape every one of them implements.
//!
//! Local or cloud is decided from the address: a loopback host is local, anything else
//! leaves this Mac and is marked amber. The key is sent as `Authorization: Bearer …`.
//!
//! What maps: messages (tool calls get sequential ids; a tool result is matched to the
//! next unanswered call, which is the order the executor appends them), tools,
//! `json_schema` as `response_format`, `think` as `reasoning_effort`, temperature,
//! max tokens. `context_length` has no field here and is ignored.

use crate::provider::{Provider, ProviderError};
use serde::Deserialize;
use stillroom_core::model::{ChatRequest, ChatResponse, InstalledModel, ToolCall};
use stillroom_core::recipe::Capability;

pub const PROVIDER_ID: &str = "openai";
pub const DEFAULT_URL: &str = "https://api.openai.com/v1";

pub struct OpenAi {
    base_url: String,
    http: reqwest::Client,
    /// Model names the user listed. Empty means ask the service (`/models`).
    models: Vec<String>,
}

impl OpenAi {
    pub fn new(base_url: impl Into<String>, api_key: Option<String>, models: Vec<String>) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
            if let Ok(mut v) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", key.trim())) {
                v.set_sensitive(true);
                headers.insert(reqwest::header::AUTHORIZATION, v);
            }
        }
        let http = reqwest::Client::builder().default_headers(headers).build().unwrap_or_default();
        let base_url = base_url.into().trim().trim_end_matches('/').to_string();
        let models = models.into_iter().map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).collect();
        Self { base_url, http, models }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// OpenAI's own API rejects `max_tokens` on newer models; everything else expects it.
    fn is_openai_com(&self) -> bool {
        self.base_url.contains("api.openai.com")
    }
}

/// A loopback address means the models run on this Mac.
pub fn is_local_url(url: &str) -> bool {
    let host = url.trim().trim_start_matches("http://").trim_start_matches("https://");
    let host = host.split(['/', ':']).next().unwrap_or("");
    matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1" | "0.0.0.0")
}

/// Chat Completions message shape, with tool-call ids made up in order.
pub fn wire_messages(req: &ChatRequest) -> Vec<serde_json::Value> {
    let mut pending: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let mut n = 0usize;
    req.messages
        .iter()
        .map(|m| {
            if m.role == "tool" {
                let id = pending.pop_front().unwrap_or_else(|| "call_0".into());
                return serde_json::json!({ "role": "tool", "tool_call_id": id, "content": m.content });
            }
            let mut v = serde_json::json!({ "role": m.role, "content": m.content });
            if !m.tool_calls.is_empty() {
                v["tool_calls"] = serde_json::Value::Array(
                    m.tool_calls
                        .iter()
                        .map(|c| {
                            n += 1;
                            let id = format!("call_{n}");
                            pending.push_back(id.clone());
                            serde_json::json!({ "id": id, "type": "function", "function": { "name": c.name, "arguments": c.arguments.to_string() } })
                        })
                        .collect(),
                );
                if m.content.is_empty() {
                    v["content"] = serde_json::Value::Null;
                }
            }
            v
        })
        .collect()
}

pub fn reasoning_effort(think: &Option<serde_json::Value>) -> Option<String> {
    match think {
        Some(serde_json::Value::Bool(false)) => Some("low".into()),
        Some(serde_json::Value::Bool(true)) => Some("high".into()),
        Some(serde_json::Value::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

#[derive(Deserialize)]
struct ModelsWire {
    data: Vec<ModelWire>,
}
#[derive(Deserialize)]
struct ModelWire {
    id: String,
}

#[derive(Deserialize)]
struct CompletionWire {
    choices: Vec<ChoiceWire>,
    #[serde(default)]
    usage: Option<UsageWire>,
}
#[derive(Deserialize)]
struct ChoiceWire {
    message: MessageWire,
}
#[derive(Deserialize)]
struct MessageWire {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCallWire>,
}
#[derive(Deserialize)]
struct ToolCallWire {
    function: FunctionWire,
}
#[derive(Deserialize)]
struct FunctionWire {
    name: String,
    #[serde(default)]
    arguments: String,
}
#[derive(Deserialize)]
struct UsageWire {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
}

/// Tool arguments arrive as a JSON string; a model that sends them malformed gets an empty object.
pub(crate) fn parse_arguments(s: &str) -> serde_json::Value {
    serde_json::from_str(s).unwrap_or_else(|_| serde_json::json!({}))
}

#[async_trait::async_trait]
impl Provider for OpenAi {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn label(&self) -> &str {
        if self.is_openai_com() { "OpenAI" } else { "OpenAI-compatible" }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn is_local(&self) -> bool {
        is_local_url(&self.base_url)
    }

    async fn is_available(&self) -> bool {
        self.http
            .get(self.url("/models"))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    async fn list_models(&self) -> Result<Vec<InstalledModel>, ProviderError> {
        let names: Vec<String> = if self.models.is_empty() {
            let resp = self
                .http
                .get(self.url("/models"))
                .send()
                .await
                .map_err(|source| ProviderError::Unreachable { url: self.base_url.clone(), source })?;
            if !resp.status().is_success() {
                return Err(ProviderError::Remote(format!("{}: {}", resp.status(), resp.text().await.unwrap_or_default())));
            }
            let wire: ModelsWire = resp.json().await?;
            wire.data.into_iter().map(|m| m.id).collect()
        } else {
            self.models.clone()
        };
        let local = self.is_local();
        Ok(names
            .into_iter()
            .map(|name| InstalledModel {
                id: format!("{PROVIDER_ID}:{name}"),
                provider: PROVIDER_ID.into(),
                name,
                // The service does not say what a model can do; these are what a chat model is expected to do.
                capabilities: vec![Capability::Text, Capability::StructuredOutput, Capability::ToolCalling, Capability::Thinking],
                context_window: None,
                parameter_size: None,
                size_bytes: None,
                local,
            })
            .collect())
    }

    async fn chat(&self, model: &str, req: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let mut body = serde_json::json!({
            "model": model,
            "messages": wire_messages(&req),
            "stream": false,
        });
        if !req.tools.is_empty() {
            body["tools"] = serde_json::Value::Array(
                req.tools
                    .iter()
                    .map(|t| serde_json::json!({ "type": "function", "function": { "name": t.name, "description": t.description, "parameters": t.parameters } }))
                    .collect(),
            );
        }
        if let Some(schema) = &req.json_schema {
            body["response_format"] = serde_json::json!({ "type": "json_schema", "json_schema": { "name": "output", "schema": schema } });
        }
        if let Some(effort) = reasoning_effort(&req.think) {
            body["reasoning_effort"] = serde_json::Value::String(effort);
        }
        if let Some(t) = req.temperature {
            body["temperature"] = t.into();
        }
        if let Some(n) = req.max_tokens {
            body[if self.is_openai_com() { "max_completion_tokens" } else { "max_tokens" }] = n.into();
        }
        let send = |body: &serde_json::Value| self.http.post(self.url("/chat/completions")).json(body).send();
        let mut resp = send(&body).await.map_err(|source| ProviderError::Unreachable { url: self.base_url.clone(), source })?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            // A service that does not know reasoning_effort says so with a 400; send the call again without it.
            if status.as_u16() == 400 && body.get("reasoning_effort").is_some() && text.to_lowercase().contains("reasoning") {
                body.as_object_mut().map(|o| o.remove("reasoning_effort"));
                resp = send(&body).await.map_err(|source| ProviderError::Unreachable { url: self.base_url.clone(), source })?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    return Err(ProviderError::Remote(format!("{status}: {}", resp.text().await.unwrap_or_default())));
                }
            } else {
                return Err(ProviderError::Remote(format!("{status}: {text}")));
            }
        }
        let wire: CompletionWire = resp.json().await?;
        let choice = wire.choices.into_iter().next().ok_or_else(|| ProviderError::Remote("the service answered with no choices".into()))?;
        Ok(ChatResponse {
            content: choice.message.content.unwrap_or_default(),
            tool_calls: choice
                .message
                .tool_calls
                .into_iter()
                .map(|c| ToolCall { name: c.function.name, arguments: parse_arguments(&c.function.arguments) })
                .collect(),
            prompt_tokens: wire.usage.as_ref().and_then(|u| u.prompt_tokens),
            completion_tokens: wire.usage.as_ref().and_then(|u| u.completion_tokens),
            completion_tokens_per_second: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stillroom_core::model::ChatMessage;

    #[test]
    fn loopback_is_local() {
        assert!(is_local_url("http://localhost:11434/v1"));
        assert!(is_local_url("http://127.0.0.1:1234/v1"));
        assert!(!is_local_url("https://api.openai.com/v1"));
        assert!(!is_local_url("http://192.168.1.20:11434/v1"));
    }

    #[test]
    fn tool_results_follow_their_calls_in_order() {
        let mut assistant = ChatMessage::new("assistant", "");
        assistant.tool_calls = vec![
            ToolCall { name: "a".into(), arguments: serde_json::json!({"x": 1}) },
            ToolCall { name: "b".into(), arguments: serde_json::json!({}) },
        ];
        let mut ra = ChatMessage::new("tool", "ra");
        ra.tool_name = Some("a".into());
        let mut rb = ChatMessage::new("tool", "rb");
        rb.tool_name = Some("b".into());
        let req = ChatRequest { messages: vec![ChatMessage::new("user", "go"), assistant, ra, rb], ..Default::default() };
        let w = wire_messages(&req);
        assert_eq!(w[1]["tool_calls"][0]["id"], "call_1");
        assert_eq!(w[1]["tool_calls"][0]["function"]["arguments"], "{\"x\":1}");
        assert!(w[1]["content"].is_null());
        assert_eq!(w[2]["tool_call_id"], "call_1");
        assert_eq!(w[3]["tool_call_id"], "call_2");
        assert_eq!(parse_arguments("{\"k\":2}"), serde_json::json!({"k": 2}));
        assert_eq!(parse_arguments("nonsense"), serde_json::json!({}));
    }
}
