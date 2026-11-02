//! Ollama native API adapter (`/api/tags`, `/api/show`, `/api/chat`).
//! Used in preference to Ollama's OpenAI-compatible endpoint because the native
//! API exposes model capabilities and accepts a full JSON Schema in `format`.

use crate::provider::*;
use serde::Deserialize;
use stillroom_core::model::{ChatRequest, ChatResponse, InstalledModel, ToolCall};
use stillroom_core::recipe::Capability;

pub struct Ollama {
    base_url: String,
    http: reqwest::Client,
}

impl Ollama {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::with_key(base_url, None)
    }

    /// An Ollama that may sit behind a proxy wanting `Authorization: Bearer <key>` on every call.
    /// The key is sent on every request and never logged.
    pub fn with_key(base_url: impl Into<String>, api_key: Option<String>) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
            if let Ok(mut v) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", key.trim())) {
                v.set_sensitive(true);
                headers.insert(reqwest::header::AUTHORIZATION, v);
            }
        }
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http: reqwest::Client::builder().default_headers(headers).build().unwrap_or_default(),
        }
    }

    pub fn default_local() -> Self {
        Self::new("http://localhost:11434")
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Download a model through Ollama, reporting each progress line as it arrives. Finishes when
    /// Ollama says `success` or reports an error.
    pub async fn pull(&self, name: &str, mut on_progress: impl FnMut(PullProgress)) -> Result<(), ProviderError> {
        let mut resp = self
            .http
            .post(self.url("/api/pull"))
            .json(&serde_json::json!({ "model": name, "stream": true }))
            .send()
            .await
            .map_err(|source| ProviderError::Unreachable { url: self.base_url.clone(), source })?
            .error_for_status()?;
        let mut buf = String::new();
        while let Some(chunk) = resp.chunk().await? {
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(nl) = buf.find('\n') {
                let line = buf[..nl].trim().to_string();
                buf.drain(..=nl);
                if line.is_empty() {
                    continue;
                }
                let v: serde_json::Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
                    return Err(ProviderError::Remote(e.to_string()));
                }
                let p = PullProgress {
                    status: v.get("status").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                    total: v.get("total").and_then(|t| t.as_u64()),
                    completed: v.get("completed").and_then(|t| t.as_u64()),
                };
                let done = p.status == "success";
                on_progress(p);
                if done {
                    return Ok(());
                }
            }
        }
        Ok(())
    }
}

/// One line of Ollama's pull stream.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullProgress {
    pub status: String,
    pub total: Option<u64>,
    pub completed: Option<u64>,
}

#[derive(Deserialize)]
struct TagsResponse {
    models: Vec<TagModel>,
}
#[derive(Deserialize)]
struct TagModel {
    name: String,
    #[serde(default)]
    details: Option<TagDetails>,
}
#[derive(Deserialize)]
struct TagDetails {
    #[serde(default)]
    parameter_size: Option<String>,
}
#[derive(Deserialize)]
struct ShowResponse {
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    model_info: serde_json::Map<String, serde_json::Value>,
}
#[derive(Deserialize)]
struct ChatResponseWire {
    message: WireMessage,
    #[serde(default)]
    prompt_eval_count: Option<u64>,
    #[serde(default)]
    eval_count: Option<u64>,
}
#[derive(Deserialize)]
struct WireMessage {
    #[serde(default)]
    content: String,
    #[serde(default)]
    tool_calls: Vec<WireToolCall>,
}
#[derive(Deserialize)]
struct WireToolCall {
    function: WireFunction,
}
#[derive(Deserialize)]
struct WireFunction {
    name: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

/// Ollama's message shape: `tool_calls` as `{function: {name, arguments}}`, tool results as
/// `role: tool` with `tool_name`.
fn wire_messages(req: &ChatRequest) -> Vec<serde_json::Value> {
    req.messages
        .iter()
        .map(|m| {
            let mut v = serde_json::json!({ "role": m.role, "content": m.content });
            if !m.tool_calls.is_empty() {
                v["tool_calls"] = serde_json::Value::Array(
                    m.tool_calls
                        .iter()
                        .map(|c| serde_json::json!({ "function": { "name": c.name, "arguments": c.arguments } }))
                        .collect(),
                );
            }
            if let Some(name) = &m.tool_name {
                v["tool_name"] = serde_json::Value::String(name.clone());
            }
            v
        })
        .collect()
}

fn map_capability(s: &str) -> Option<Capability> {
    match s {
        "completion" => Some(Capability::Text),
        "tools" => Some(Capability::ToolCalling),
        "thinking" => Some(Capability::Thinking),
        "vision" => Some(Capability::Vision),
        "embedding" => Some(Capability::Embedding),
        _ => None,
    }
}

#[async_trait::async_trait]
impl Provider for Ollama {
    fn id(&self) -> &str {
        "ollama"
    }

    fn label(&self) -> &str {
        "Ollama"
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    async fn embed(&self, model: &str, texts: Vec<String>) -> Result<Vec<Vec<f32>>, ProviderError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let body = serde_json::json!({ "model": model, "input": texts });
        let url = self.url("/api/embed");
        let resp = self.http.post(&url).json(&body).send().await.map_err(|source| ProviderError::Unreachable { url: url.clone(), source })?;
        let status = resp.status();
        let text = resp.text().await.map_err(|source| ProviderError::Unreachable { url: url.clone(), source })?;
        if !status.is_success() {
            return Err(ProviderError::Remote(format!("{status}: {text}")));
        }
        let v: serde_json::Value = serde_json::from_str(&text)?;
        let rows = v.get("embeddings").and_then(|e| e.as_array()).ok_or_else(|| ProviderError::Remote("no embeddings in the answer".into()))?;
        Ok(rows
            .iter()
            .map(|r| r.as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect()).unwrap_or_default())
            .collect())
    }

    async fn is_available(&self) -> bool {
        self.http
            .get(self.url("/"))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    async fn list_models(&self) -> Result<Vec<InstalledModel>, ProviderError> {
        let tags: TagsResponse = self
            .http
            .get(self.url("/api/tags"))
            .send()
            .await
            .map_err(|source| ProviderError::Unreachable { url: self.base_url.clone(), source })?
            .error_for_status()?
            .json()
            .await?;

        let mut out = Vec::with_capacity(tags.models.len());
        for m in tags.models {
            let show: ShowResponse = self
                .http
                .post(self.url("/api/show"))
                .json(&serde_json::json!({ "model": m.name }))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;

            let mut caps: Vec<Capability> = show.capabilities.iter().filter_map(|c| map_capability(c)).collect();
            // Every chat-capable Ollama model accepts a JSON Schema in `format`.
            if caps.contains(&Capability::Text) {
                caps.push(Capability::StructuredOutput);
            }
            let context_window = show
                .model_info
                .iter()
                .find(|(k, _)| k.ends_with(".context_length"))
                .and_then(|(_, v)| v.as_u64());

            out.push(InstalledModel {
                id: format!("ollama:{}", m.name),
                provider: "ollama".into(),
                name: m.name,
                capabilities: caps,
                context_window,
                parameter_size: m.details.and_then(|d| d.parameter_size),
                local: true,
            });
        }
        Ok(out)
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
                    .map(|t| {
                        serde_json::json!({
                            "type": "function",
                            "function": { "name": t.name, "description": t.description, "parameters": t.parameters }
                        })
                    })
                    .collect(),
            );
        }
        if let Some(schema) = req.json_schema {
            body["format"] = schema;
        }
        if let Some(think) = req.think {
            body["think"] = think;
        }
        let mut options = serde_json::Map::new();
        if let Some(t) = req.temperature {
            options.insert("temperature".into(), t.into());
        }
        if let Some(n) = req.max_tokens {
            options.insert("num_predict".into(), n.into());
        }
        if let Some(n) = req.context_length {
            options.insert("num_ctx".into(), n.into());
        }
        if !options.is_empty() {
            body["options"] = serde_json::Value::Object(options);
        }

        let mut resp = self
            .http
            .post(self.url("/api/chat"))
            .json(&body)
            .send()
            .await
            .map_err(|source| ProviderError::Unreachable { url: self.base_url.clone(), source })?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            // Models without a thinking mode reject the `think` field; retry without it.
            if status.as_u16() == 400 && body.get("think").is_some() && text.to_lowercase().contains("think") {
                body.as_object_mut().map(|o| o.remove("think"));
                resp = self
                    .http
                    .post(self.url("/api/chat"))
                    .json(&body)
                    .send()
                    .await
                    .map_err(|source| ProviderError::Unreachable { url: self.base_url.clone(), source })?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    return Err(ProviderError::Remote(format!("{status}: {text}")));
                }
            } else {
                return Err(ProviderError::Remote(format!("{status}: {text}")));
            }
        }
        let wire: ChatResponseWire = resp.json().await?;

        Ok(ChatResponse {
            content: wire.message.content,
            tool_calls: wire
                .message
                .tool_calls
                .into_iter()
                .map(|c| ToolCall { name: c.function.name, arguments: c.function.arguments })
                .collect(),
            prompt_tokens: wire.prompt_eval_count,
            completion_tokens: wire.eval_count,
        })
    }
}
