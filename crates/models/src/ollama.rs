//! Ollama native API adapter (`/api/tags`, `/api/show`, `/api/chat`).
//! Used in preference to Ollama's OpenAI-compatible endpoint because the native
//! API exposes model capabilities and accepts a full JSON Schema in `format`.

use crate::provider::*;
use serde::Deserialize;
use stillroom_core::model::{tokens_per_second, ChatRequest, ChatResponse, InstalledModel, ToolCall};
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
    size: Option<u64>,
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
    /// How long writing the answer took, in nanoseconds (`prompt_eval_duration` is reading
    /// time, which is not a speed the table shows). With `eval_count`, the speed.
    #[serde(default)]
    eval_duration: Option<u64>,
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
                size_bytes: m.size,
                local: true,
            });
        }
        Ok(out)
    }

    async fn chat(&self, model: &str, req: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let body = chat_body(model, &req, false);
        let resp = self.send_chat(body).await?;
        let wire: ChatResponseWire = resp.json().await?;
        Ok(ChatResponse {
            content: wire.message.content,
            tool_calls: wire_tool_calls(wire.message.tool_calls),
            prompt_tokens: wire.prompt_eval_count,
            completion_tokens: wire.eval_count,
            completion_tokens_per_second: tokens_per_second(wire.eval_count, wire.eval_duration),
        })
    }

    /// `stream: true`: Ollama answers with one JSON object per line, each carrying a piece of the
    /// message; the last line (`done: true`) carries the token counts. `format` still applies.
    async fn chat_stream(&self, model: &str, req: ChatRequest, on_delta: &(dyn for<'a> Fn(&'a str) + Send + Sync)) -> Result<ChatResponse, ProviderError> {
        let body = chat_body(model, &req, true);
        let mut resp = self.send_chat(body).await?;
        let mut acc = StreamAccumulator::default();
        let mut buf = String::new();
        while let Some(chunk) = resp.chunk().await? {
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(nl) = buf.find('\n') {
                let line = buf[..nl].trim().to_string();
                buf.drain(..=nl);
                if let Some(text) = acc.take_line(&line)? {
                    on_delta(&text);
                }
            }
        }
        if let Some(text) = acc.take_line(buf.trim())? {
            on_delta(&text);
        }
        Ok(acc.finish())
    }
}

impl Ollama {
    /// Post a chat body; a model without a thinking mode rejects the `think` field, so the call
    /// is retried without it.
    async fn send_chat(&self, mut body: serde_json::Value) -> Result<reqwest::Response, ProviderError> {
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
        Ok(resp)
    }
}

fn chat_body(model: &str, req: &ChatRequest, stream: bool) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "messages": wire_messages(req),
        "stream": stream,
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
    if let Some(schema) = &req.json_schema {
        body["format"] = schema.clone();
    }
    if let Some(think) = &req.think {
        body["think"] = think.clone();
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
    body
}

fn wire_tool_calls(calls: Vec<WireToolCall>) -> Vec<ToolCall> {
    calls.into_iter().map(|c| ToolCall { name: c.function.name, arguments: c.function.arguments }).collect()
}

/// One line of a streamed `/api/chat` answer.
#[derive(Deserialize)]
struct StreamLine {
    #[serde(default)]
    message: Option<StreamMessage>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    prompt_eval_count: Option<u64>,
    #[serde(default)]
    eval_count: Option<u64>,
    /// Nanoseconds spent writing the answer; on the `done` line.
    #[serde(default)]
    eval_duration: Option<u64>,
}
#[derive(Deserialize)]
struct StreamMessage {
    #[serde(default)]
    content: String,
    /// Reasoning text, present when the model thinks before answering and `think` is on.
    #[serde(default)]
    thinking: String,
    #[serde(default)]
    tool_calls: Vec<WireToolCall>,
}

/// Builds the final `ChatResponse` from streamed lines and says what text each line adds to the
/// visible stream. Thinking text is shown too, headed `(thinking)`; when the answer proper starts
/// after it, `(answer)` marks the change. Only the answer's content goes into `ChatResponse`.
#[derive(Default)]
struct StreamAccumulator {
    content: String,
    tool_calls: Vec<ToolCall>,
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    eval_nanos: Option<u64>,
    thinking_shown: bool,
    answer_shown: bool,
}

impl StreamAccumulator {
    /// Feed one line. Returns the text to show for it, if any. A blank or unparseable line adds
    /// nothing; an `error` line is the provider's error.
    fn take_line(&mut self, line: &str) -> Result<Option<String>, ProviderError> {
        if line.is_empty() {
            return Ok(None);
        }
        let v: StreamLine = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };
        if let Some(e) = v.error {
            return Err(ProviderError::Remote(e));
        }
        if let Some(n) = v.prompt_eval_count {
            self.prompt_tokens = Some(n);
        }
        if let Some(n) = v.eval_count {
            self.completion_tokens = Some(n);
        }
        if let Some(ns) = v.eval_duration {
            self.eval_nanos = Some(ns);
        }
        let Some(m) = v.message else { return Ok(None) };
        self.tool_calls.extend(wire_tool_calls(m.tool_calls));
        let mut shown = String::new();
        if !m.thinking.is_empty() {
            if !self.thinking_shown {
                self.thinking_shown = true;
                shown.push_str("(thinking)\n");
            }
            shown.push_str(&m.thinking);
        }
        if !m.content.is_empty() {
            if self.thinking_shown && !self.answer_shown {
                shown.push_str("\n(answer)\n");
            }
            self.answer_shown = true;
            shown.push_str(&m.content);
            self.content.push_str(&m.content);
        }
        Ok(if shown.is_empty() { None } else { Some(shown) })
    }

    fn finish(self) -> ChatResponse {
        ChatResponse {
            content: self.content,
            tool_calls: self.tool_calls,
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            completion_tokens_per_second: tokens_per_second(self.completion_tokens, self.eval_nanos),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    const BODY: &str = concat!(
        r#"{"model":"t","message":{"role":"assistant","content":"","thinking":"Let me see."},"done":false}"#,
        "\n",
        r#"{"model":"t","message":{"role":"assistant","content":"{\"a\":"},"done":false}"#,
        "\n",
        r#"{"model":"t","message":{"role":"assistant","content":" 1}"},"done":false}"#,
        "\n\n",
        r#"{"model":"t","message":{"role":"assistant","content":""},"done":true,"prompt_eval_count":12,"eval_count":3,"prompt_eval_duration":200000000,"eval_duration":50000000}"#,
        "\n"
    );

    #[test]
    fn stream_lines_become_deltas_and_one_response() {
        let mut acc = StreamAccumulator::default();
        let shown: Vec<String> = BODY.lines().filter_map(|l| acc.take_line(l.trim()).unwrap()).collect();
        assert_eq!(shown, vec!["(thinking)\nLet me see.", "\n(answer)\n{\"a\":", " 1}"]);
        let resp = acc.finish();
        assert_eq!(resp.content, "{\"a\": 1}", "thinking text stays out of the answer");
        assert_eq!(resp.prompt_tokens, Some(12));
        assert_eq!(resp.completion_tokens, Some(3));
        assert_eq!(resp.completion_tokens_per_second, Some(60.0), "3 tokens in 50 ms");
        assert!(resp.tool_calls.is_empty());
    }

    #[test]
    fn a_one_shot_answer_carries_its_speed_when_ollama_says_how_long() {
        let wire: ChatResponseWire =
            serde_json::from_str(r#"{"message":{"role":"assistant","content":"pong"},"done":true,"prompt_eval_count":8,"eval_count":120,"prompt_eval_duration":100000000,"eval_duration":2000000000}"#).unwrap();
        assert_eq!(tokens_per_second(wire.eval_count, wire.eval_duration), Some(60.0));
        let old: ChatResponseWire = serde_json::from_str(r#"{"message":{"role":"assistant","content":"pong"},"done":true,"eval_count":120}"#).unwrap();
        assert_eq!(tokens_per_second(old.eval_count, old.eval_duration), None, "no duration, no speed");
    }

    #[test]
    fn an_error_line_is_the_providers_error() {
        let mut acc = StreamAccumulator::default();
        let e = acc.take_line(r#"{"error":"model not found"}"#).unwrap_err();
        assert!(e.to_string().contains("model not found"));
        assert!(acc.take_line("not json").unwrap().is_none());
    }

    #[test]
    fn the_body_asks_for_a_stream_and_keeps_the_schema() {
        let req = ChatRequest { json_schema: Some(serde_json::json!({"type": "object"})), think: Some(false.into()), ..Default::default() };
        let b = chat_body("m", &req, true);
        assert_eq!(b["stream"], true);
        assert_eq!(b["format"]["type"], "object");
        assert_eq!(b["think"], false);
        assert_eq!(chat_body("m", &req, false)["stream"], false);
    }

    /// A one-request HTTP server on a free port that answers with `BODY` in two writes, so the
    /// adapter has to join chunks across line boundaries.
    fn serve_once() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut req = [0u8; 16384];
            let mut n = 0;
            loop {
                let k = s.read(&mut req[n..]).unwrap();
                n += k;
                let head = String::from_utf8_lossy(&req[..n]).to_string();
                if let Some(end) = head.find("\r\n\r\n") {
                    let len: usize = head
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().to_string()))
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    if n >= end + 4 + len {
                        break;
                    }
                }
                if k == 0 {
                    break;
                }
            }
            let head = format!("HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", BODY.len());
            s.write_all(head.as_bytes()).unwrap();
            let split = BODY.find("\" 1}").unwrap();
            s.write_all(&BODY.as_bytes()[..split]).unwrap();
            s.flush().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(30));
            s.write_all(&BODY.as_bytes()[split..]).unwrap();
            s.flush().unwrap();
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn chat_stream_reads_ndjson_from_a_server() {
        let ollama = Ollama::new(serve_once());
        let seen = std::sync::Mutex::new(Vec::<String>::new());
        let req = ChatRequest { messages: vec![stillroom_core::model::ChatMessage::new("user", "hi")], ..Default::default() };
        let resp = ollama.chat_stream("t", req, &|d: &str| seen.lock().unwrap().push(d.to_string())).await.unwrap();
        assert_eq!(resp.content, "{\"a\": 1}");
        assert_eq!(resp.completion_tokens, Some(3));
        assert_eq!(resp.completion_tokens_per_second, Some(60.0));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.concat(), "(thinking)\nLet me see.\n(answer)\n{\"a\": 1}");
        assert!(seen.len() >= 2, "more than one delta: {seen:?}");
    }
}
