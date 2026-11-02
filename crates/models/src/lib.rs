//! Model adapters. One trait, several providers, one router.
//!
//! Slot resolution (pinned > recommended > capability) lives in core.
//! This crate only knows how to talk to providers and report what they offer.

pub mod codex;
pub mod ollama;
pub mod openai;
pub mod provider;
pub mod router;

pub use codex::{Codex, CodexStatus};
pub use openai::OpenAi;
pub use provider::{Provider, ProviderError, ProviderStatus};
pub use router::ModelRouter;
pub use stillroom_core::model::{ChatMessage, ChatRequest, ChatResponse, InstalledModel};
