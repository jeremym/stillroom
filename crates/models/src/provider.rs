use serde::{Deserialize, Serialize};
use stillroom_core::model::{CallPhase, ChatRequest, ChatResponse, InstalledModel, StreamEvent};

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("nothing answered at {url}: {source}")]
    Unreachable { url: String, source: reqwest::Error },
    #[error("provider returned an error: {0}")]
    Remote(String),
    #[error("could not decode provider response: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    /// A provider that runs as a program on this Mac (the Codex CLI) failed outside HTTP.
    #[error("{0}")]
    Process(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub id: String,
    pub label: String,
    pub base_url: String,
    pub available: bool,
    /// True when the provider's models run on this Mac.
    pub local: bool,
}

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> &str;
    fn label(&self) -> &str;
    fn base_url(&self) -> &str;
    /// Whether prompts stay on this Mac. Anything that is not local is marked amber in the app.
    fn is_local(&self) -> bool {
        true
    }
    /// True if the provider answers at its configured URL.
    async fn is_available(&self) -> bool;
    async fn list_models(&self) -> Result<Vec<InstalledModel>, ProviderError>;
    /// `model` is the bare model name as the provider knows it.
    async fn chat(&self, model: &str, req: ChatRequest) -> Result<ChatResponse, ProviderError>;
    /// Like `chat`, but `on_delta` sees each piece of text as the model writes it. Providers that
    /// cannot stream answer once, with the whole content. The returned response is the same as
    /// `chat` would give.
    async fn chat_stream(&self, model: &str, req: ChatRequest, on_delta: &(dyn for<'a> Fn(&'a str) + Send + Sync)) -> Result<ChatResponse, ProviderError> {
        let resp = self.chat(model, req).await?;
        if !resp.content.is_empty() {
            on_delta(&resp.content);
        }
        Ok(resp)
    }
    /// Like `chat_stream`, but `on` also hears the call's phase (loading, reading, writing).
    /// Providers that cannot tell say `Reading`, then `Writing` with the first text.
    async fn chat_events(&self, model: &str, req: ChatRequest, on: &(dyn for<'a> Fn(StreamEvent<'a>) + Send + Sync)) -> Result<ChatResponse, ProviderError> {
        on(StreamEvent::Phase(CallPhase::Reading));
        let writing = std::sync::atomic::AtomicBool::new(false);
        let on_delta = |text: &str| {
            if !writing.swap(true, std::sync::atomic::Ordering::SeqCst) {
                on(StreamEvent::Phase(CallPhase::Writing));
            }
            on(StreamEvent::Text(text));
        };
        self.chat_stream(model, req, &on_delta).await
    }
    /// One vector per text. Only providers with embedding models implement it.
    async fn embed(&self, model: &str, _texts: Vec<String>) -> Result<Vec<Vec<f32>>, ProviderError> {
        Err(ProviderError::Remote(format!("{model} cannot make embeddings here")))
    }
}
