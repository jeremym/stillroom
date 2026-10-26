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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    /// If set, the provider is asked to constrain output to this JSON Schema.
    pub json_schema: Option<serde_json::Value>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    /// Requested context window. Providers that support it honor it.
    pub context_length: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub content: String,
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
