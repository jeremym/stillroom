//! Run traces: what happened, step by step, with enough to reproduce and to show in the UI.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepRecord {
    pub run_id: String,
    pub step_id: String,
    pub kind: String,
    pub name: Option<String>,
    pub position: u32,
    pub status: StepStatus,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    /// Provider-qualified model id, for model steps.
    pub model: Option<String>,
    pub prompt_path: Option<String>,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    /// Inputs after reference resolution.
    pub inputs: Value,
    pub output: Option<Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: String,
    pub recipe_id: String,
    pub recipe_name: String,
    pub recipe_version: Option<String>,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub inputs: BTreeMap<String, Value>,
    /// Slot name -> resolution, as decided before the run.
    pub resolutions: Value,
    pub steps: Vec<StepRecord>,
    pub outputs: BTreeMap<String, Value>,
    pub error: Option<String>,
}

/// Lightweight row for run lists.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub id: String,
    pub recipe_id: String,
    pub recipe_name: String,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub step_count: usize,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

impl Run {
    pub fn summary(&self) -> RunSummary {
        RunSummary {
            id: self.id.clone(),
            recipe_id: self.recipe_id.clone(),
            recipe_name: self.recipe_name.clone(),
            status: self.status,
            started_at: self.started_at,
            finished_at: self.finished_at,
            step_count: self.steps.len(),
            prompt_tokens: self.steps.iter().filter_map(|s| s.prompt_tokens).sum(),
            completion_tokens: self.steps.iter().filter_map(|s| s.completion_tokens).sum(),
        }
    }
}
