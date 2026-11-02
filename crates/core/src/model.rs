//! Provider-neutral model types and the trait the executor calls.

use crate::recipe::Capability;
use serde::{Deserialize, Serialize};

/// A model a provider has available right now, with the capabilities it can satisfy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModel {
    /// Provider-qualified id, e.g. `ollama:qwen3:32b`.
    pub id: String,
    pub provider: String,
    /// Bare model name as the provider knows it.
    pub name: String,
    pub capabilities: Vec<Capability>,
    pub context_window: Option<u64>,
    /// Provider-reported size such as "8.2B". Used as a rough quality/speed proxy.
    pub parameter_size: Option<String>,
    /// The weights on disk, in bytes, when the provider says (Ollama does). What the model needs
    /// in memory to run is about this much; see [`fits_in_memory`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    /// True when the model runs on this Mac. Auto never picks a model that is not local;
    /// a cloud model is used only through a pin or a default you set.
    #[serde(default = "yes")]
    pub local: bool,
}

fn yes() -> bool {
    true
}

/// The share of physical RAM a model's weights may take and still run comfortably. Above this,
/// the system swaps the weights in and out and generation falls off a cliff; below it, the model,
/// its context, the app, and the rest of the Mac fit together. 60 % leaves a 36 GB Mac about
/// 21 GB for weights, a 64 GB Mac about 38 GB.
pub const MEMORY_CEILING: f64 = 0.60;

/// Whether weights of `size_bytes` fit comfortably in `memory_bytes` of physical RAM (at most
/// [`MEMORY_CEILING`] of it). Either unknown means "treat as fits": a model without a reported
/// size (a cloud model) or a Mac whose RAM could not be read is never marked too large.
pub fn fits_in_memory(size_bytes: Option<u64>, memory_bytes: Option<u64>) -> bool {
    match (size_bytes, memory_bytes) {
        (Some(size), Some(memory)) => (size as f64) <= (memory as f64) * MEMORY_CEILING,
        _ => true,
    }
}

/// Physical RAM of this machine, read once by the shell at startup. On a Mac it is
/// `sysctl -n hw.memsize`; elsewhere, or if the read fails, `None` (unknown: every model fits).
pub fn physical_memory_bytes() -> Option<u64> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let out = std::process::Command::new("/usr/sbin/sysctl").args(["-n", "hw.memsize"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

impl InstalledModel {
    /// Parameter count in billions: from the provider's `parameter_size`, or as a
    /// fallback from a `27b` / `0.6b` token in the model name.
    pub fn parameter_billions(&self) -> Option<f64> {
        if let Some(n) = self.parameter_size.as_deref().and_then(parse_size) {
            return Some(n);
        }
        self.name
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '.')
            .find_map(parse_size)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    /// Tool calls the assistant made in this message (echoed back in the transcript).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// For `role: tool` messages, the tool that produced `content`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

impl ChatMessage {
    pub fn new(role: &str, content: impl Into<String>) -> Self {
        Self { role: role.into(), content: content.into(), tool_calls: Vec::new(), tool_name: None }
    }
}

/// A tool the model may call, described to the provider in JSON Schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    /// Tools the model may call. Empty means a plain completion.
    #[serde(default)]
    pub tools: Vec<ToolSpec>,
    /// If set, the provider is asked to constrain output to this JSON Schema.
    pub json_schema: Option<serde_json::Value>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    /// Requested context window. Providers that support it honor it.
    pub context_length: Option<u32>,
    /// Thinking control for reasoning models: `false` disables, `true` enables, or a
    /// provider-specific level such as "low". Passed through as-is.
    pub think: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub content: String,
    /// Tool calls the model wants made before it can continue.
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("no provider for model id {0}")]
    UnknownProvider(String),
    #[error("provider unavailable: {0}")]
    Unavailable(String),
    #[error("provider error: {0}")]
    Remote(String),
}

/// Anything that can answer a chat request for a provider-qualified model id.
#[async_trait::async_trait]
pub trait ModelClient: Send + Sync {
    async fn chat(&self, model_id: &str, req: ChatRequest) -> Result<ChatResponse, ModelError>;
    /// Like `chat`, but `on_delta` sees each piece of text as the model writes it (streaming).
    /// Clients that cannot stream call it once with the whole content.
    async fn chat_stream(&self, model_id: &str, req: ChatRequest, on_delta: &(dyn for<'a> Fn(&'a str) + Send + Sync)) -> Result<ChatResponse, ModelError> {
        let resp = self.chat(model_id, req).await?;
        if !resp.content.is_empty() {
            on_delta(&resp.content);
        }
        Ok(resp)
    }
    /// One vector per text, from an embedding model. Providers that cannot embed say so.
    async fn embed(&self, model_id: &str, _texts: Vec<String>) -> Result<Vec<Vec<f32>>, ModelError> {
        Err(ModelError::Remote(format!("{model_id} cannot make embeddings")))
    }
}

fn parse_size(s: &str) -> Option<f64> {
    let s = s.trim();
    let (num, unit) = s.split_at(s.len().checked_sub(1)?);
    let n: f64 = num.parse().ok()?;
    match unit {
        "B" | "b" => Some(n),
        "M" | "m" => Some(n / 1000.0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(name: &str, size: Option<&str>) -> InstalledModel {
        InstalledModel {
            id: format!("ollama:{name}"),
            provider: "ollama".into(),
            name: name.into(),
            capabilities: vec![],
            context_window: None,
            parameter_size: size.map(String::from),
            size_bytes: None,
            local: true,
        }
    }

    #[test]
    fn fits_in_memory_uses_the_ceiling_and_treats_unknown_as_fits() {
        let gb = 1u64 << 30;
        let mac_36 = Some(36 * gb);
        assert!(fits_in_memory(Some(20 * gb), mac_36), "20 GB on a 36 GB Mac is under 60 %");
        assert!(!fits_in_memory(Some(30 * gb), mac_36), "30 GB on a 36 GB Mac is over 60 %");
        assert!(fits_in_memory(Some(60 * gb), Some(100 * gb)), "exactly at the ceiling fits");
        assert!(fits_in_memory(None, mac_36), "a model without a size is not called too large");
        assert!(fits_in_memory(Some(30 * gb), None), "RAM unknown: treat as fits");
    }

    #[test]
    fn physical_memory_reads_on_a_mac() {
        if cfg!(target_os = "macos") {
            assert!(physical_memory_bytes().unwrap_or(0) >= 4 << 30, "a Mac has at least 4 GB");
        }
    }

    #[test]
    fn parameter_size_from_provider_or_name() {
        assert_eq!(m("x", Some("29.9B")).parameter_billions(), Some(29.9));
        assert!((m("x", Some("566.70M")).parameter_billions().unwrap() - 0.5667).abs() < 1e-9);
        assert_eq!(m("qwen3.8:27b-mlx", None).parameter_billions(), Some(27.0));
        assert_eq!(m("hf.co/unsloth/Qwen3-Coder-Next-GGUF:UD-Q4_K_XL", None).parameter_billions(), None);
    }
}
