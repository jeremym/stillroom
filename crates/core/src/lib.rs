//! Stillroom core: recipe types, validation, slot resolution, execution, traces, and storage.
//!
//! This crate knows nothing about any particular model provider or desktop shell.
//! Providers implement [`model::ModelClient`]; tools implement [`tools::Tool`].

pub mod bundle;
pub mod cache;
pub mod draft;
pub mod executor;
pub mod library;
pub mod model;
pub mod propagate;
pub mod recipe;
pub mod refs;
pub mod resolve;
pub mod store;
pub mod tools;
pub mod trace;
pub mod watch;

pub use bundle::{RecipeBundle, RecipeInfo, RecipeSource};
pub use cache::{CachedStep, MemoryCache, StepCache};
pub use draft::{Draft, DraftError};
pub use executor::{Executor, RunRequest};
pub use library::RecipeLibrary;
pub use propagate::{dependents, edits_for_rerun, pins_from_run, validate_edit, EditError, StepEdit};
pub use model::{ChatMessage, ChatRequest, ChatResponse, InstalledModel, ModelClient, ModelError};
pub use recipe::{Recipe, RecipeError, Step, RECIPE_SCHEMA};
pub use resolve::{resolve_slots, resolve_slots_with, ResolveDefaults, ResolveError, Resolution, ResolvedVia};
pub use store::Store;
pub use trace::{Run, RunStatus, RunTrigger, StepRecord, StepStatus};
pub use watch::{FolderWatcher, Watch, WatchError};
