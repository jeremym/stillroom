//! Routes provider-qualified model ids (`ollama:qwen3:8b`) to the right provider.

use crate::provider::{Provider, ProviderStatus};
use std::sync::Arc;
use stillroom_core::model::{ChatRequest, ChatResponse, InstalledModel, ModelClient, ModelError, StreamEvent};

pub struct ModelRouter {
    providers: Vec<Arc<dyn Provider>>,
}

impl ModelRouter {
    pub fn new(providers: Vec<Arc<dyn Provider>>) -> Self {
        Self { providers }
    }

    pub fn provider(&self, id: &str) -> Option<Arc<dyn Provider>> {
        self.providers.iter().find(|p| p.id() == id).cloned()
    }

    pub async fn statuses(&self) -> Vec<ProviderStatus> {
        let mut out = Vec::new();
        for p in &self.providers {
            out.push(ProviderStatus {
                id: p.id().into(),
                label: p.label().into(),
                base_url: p.base_url().into(),
                available: p.is_available().await,
                local: p.is_local(),
            });
        }
        out
    }

    /// Every model from every reachable provider. Unreachable providers are skipped, not errors.
    pub async fn list_all(&self) -> Vec<InstalledModel> {
        let mut out = Vec::new();
        for p in &self.providers {
            if !p.is_available().await {
                continue;
            }
            match p.list_models().await {
                Ok(models) => out.extend(models),
                Err(e) => tracing::warn!(provider = p.id(), "list_models failed: {e}"),
            }
        }
        out
    }

    fn split(model_id: &str) -> Option<(&str, &str)> {
        model_id.split_once(':')
    }
}

#[async_trait::async_trait]
impl ModelClient for ModelRouter {
    async fn chat(&self, model_id: &str, req: ChatRequest) -> Result<ChatResponse, ModelError> {
        let (pid, name) = Self::split(model_id).ok_or_else(|| ModelError::UnknownProvider(model_id.into()))?;
        let p = self.provider(pid).ok_or_else(|| ModelError::UnknownProvider(model_id.into()))?;
        p.chat(name, req).await.map_err(model_error)
    }

    async fn chat_stream(&self, model_id: &str, req: ChatRequest, on_delta: &(dyn for<'a> Fn(&'a str) + Send + Sync)) -> Result<ChatResponse, ModelError> {
        let (pid, name) = Self::split(model_id).ok_or_else(|| ModelError::UnknownProvider(model_id.into()))?;
        let p = self.provider(pid).ok_or_else(|| ModelError::UnknownProvider(model_id.into()))?;
        p.chat_stream(name, req, on_delta).await.map_err(model_error)
    }

    async fn chat_events(&self, model_id: &str, req: ChatRequest, on: &(dyn for<'a> Fn(StreamEvent<'a>) + Send + Sync)) -> Result<ChatResponse, ModelError> {
        let (pid, name) = Self::split(model_id).ok_or_else(|| ModelError::UnknownProvider(model_id.into()))?;
        let p = self.provider(pid).ok_or_else(|| ModelError::UnknownProvider(model_id.into()))?;
        p.chat_events(name, req, on).await.map_err(model_error)
    }

    async fn embed(&self, model_id: &str, texts: Vec<String>) -> Result<Vec<Vec<f32>>, ModelError> {
        let (pid, name) = Self::split(model_id).ok_or_else(|| ModelError::UnknownProvider(model_id.into()))?;
        let p = self.provider(pid).ok_or_else(|| ModelError::UnknownProvider(model_id.into()))?;
        p.embed(name, texts).await.map_err(model_error)
    }
}

fn model_error(e: crate::provider::ProviderError) -> ModelError {
    match e {
        crate::provider::ProviderError::Unreachable { .. } => ModelError::Unavailable(e.to_string()),
        other => ModelError::Remote(other.to_string()),
    }
}
