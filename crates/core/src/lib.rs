//! Stillroom core: recipe types, validation, slot resolution, execution, traces, and storage.
//!
//! This crate knows nothing about any particular model provider or desktop shell.
//! Providers implement [`model::ModelClient`]; tools implement [`tools::Tool`].

pub mod bundle;
pub mod cache;
pub mod draft;
pub mod executor;
pub mod keep;
pub mod library;
pub mod migrate;
pub mod model;
pub mod planner;
pub mod prompts;
pub mod propagate;
pub mod recipe;
pub mod refs;
pub mod resolve;
pub mod stagelib;
pub mod store;
pub mod tools;
pub mod trace;
pub mod watch;

pub use bundle::{RecipeBundle, RecipeInfo, RecipeSource};
pub use cache::{CachedStep, MemoryCache, StepCache};
pub use draft::{Draft, DraftError};
pub use executor::{stage_fingerprint, stage_models, Cancel, Executor, ModelDefaults, RunRequest, FINGERPRINT_FORMAT, FORMAT_CHANGED_REASON};
pub use keep::{Keep, KeepError, KeepManifest, KeepRecord, KeepSummary, JournalEntry};
pub use library::RecipeLibrary;
pub use migrate::{migrate, Finding, FixId, MigrateError, Migrated, Severity, CURRENT_SCHEMA_VERSION};
pub use stagelib::{StageDef, StageLibrary};
pub use propagate::{dependents, edits_for_rerun, pins_from_run, validate_edit, EditError, StepEdit};
pub use planner::{PlanError, PlanEvent, PlanOutcome, PlanRequest, Planner};
pub use prompts::{PromptError, PromptState, SystemPrompts};
pub use model::{ChatMessage, ChatRequest, ChatResponse, InstalledModel, ModelClient, ModelError};
pub use recipe::{Recipe, RecipeError, Step, RECIPE_SCHEMA};
pub use resolve::{resolve_slots, resolve_slots_with, ResolveDefaults, ResolveError, Resolution, ResolvedVia};
pub use store::{ShelfState, Store, EXAMPLE_RECIPE_ID};
pub use trace::{Run, RunStatus, RunTrigger, StepRecord, StepStatus};
pub use watch::{FolderWatcher, Watch, WatchError};
