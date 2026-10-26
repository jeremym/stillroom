//! Stillroom core: recipe types, validation, slot resolution, execution, traces, and storage.
//!
//! This crate knows nothing about any particular model provider or desktop shell.
//! Providers implement [`model::ModelClient`]; tools implement [`tools::Tool`].

pub mod executor;
pub mod model;
pub mod recipe;
pub mod refs;
pub mod resolve;
pub mod store;
pub mod tools;
pub mod trace;

pub use executor::{Executor, RunRequest};
pub use model::{ChatMessage, ChatRequest, ChatResponse, InstalledModel, ModelClient, ModelError};
pub use recipe::{Recipe, RecipeError, Step, RECIPE_SCHEMA};
pub use resolve::{resolve_slots, ResolveError, Resolution, ResolvedVia};
pub use store::Store;
pub use trace::{Run, RunStatus, StepRecord, StepStatus};
