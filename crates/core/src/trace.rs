//! Run traces: what happened, step by step, with enough to reproduce and to show in the UI.

use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
    /// Finished as far as it can: a review step is waiting for the user.
    NeedsReview,
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
    /// Set when the output was reused from an earlier run instead of recomputed.
    #[serde(default)]
    pub reused_from: Option<String>,
    /// Set when the output is one the user edited (change propagation) rather than generated.
    #[serde(default)]
    pub edited: bool,
    /// For steps inside a map: the map step's id.
    #[serde(default)]
    pub parent: Option<String>,
    /// Why a skipped step was skipped: a `when` that was not set, or a pending review.
    #[serde(default)]
    pub skip_reason: Option<String>,
    /// For agent steps: every tool call the model made, in order.
    #[serde(default)]
    pub tool_calls: Vec<ToolCallRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallRecord {
    pub tool: String,
    pub inputs: Value,
    pub output: Option<Value>,
    pub error: Option<String>,
    pub duration_ms: u64,
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
    pub inputs: IndexMap<String, Value>,
    /// Slot name -> resolution, as decided before the run.
    pub resolutions: Value,
    pub steps: Vec<StepRecord>,
    pub outputs: IndexMap<String, Value>,
    pub error: Option<String>,
    /// Edits this run was asked to apply, by step id. An edit whose step record is not
    /// marked `edited` was set aside because the step's inputs had changed.
    #[serde(default)]
    pub edits: std::collections::BTreeMap<String, crate::propagate::StepEdit>,
    /// What started the run when it was not the user pressing Run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<RunTrigger>,
    /// The run this one continued (an approval, an edit, or a watch running again).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continues: Option<String>,
}

/// A run started by something other than the user: so far, a folder watch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunTrigger {
    /// `watch` for now.
    pub kind: String,
    pub watch_id: String,
    /// The watched folder.
    pub folder: String,
    /// File names whose change set the run off. Empty when the user pressed Run now.
    #[serde(default)]
    pub changed: Vec<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<RunTrigger>,
}

impl Run {
    /// Take in the records of an older run this one continues, so a continuation can still
    /// find what was done before a pause held some steps back. This run's own records stay
    /// first, so the newest result for a step wins.
    pub fn absorb(&mut self, older: &Run) {
        use crate::propagate::inputs_fingerprint;
        for r in &older.steps {
            if r.status != StepStatus::Succeeded || r.output.is_none() {
                continue;
            }
            let fp = inputs_fingerprint(&r.inputs);
            if !self.steps.iter().any(|m| m.step_id == r.step_id && inputs_fingerprint(&m.inputs) == fp) {
                self.steps.push(r.clone());
            }
        }
    }

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
            trigger: self.trigger.clone(),
        }
    }
}
