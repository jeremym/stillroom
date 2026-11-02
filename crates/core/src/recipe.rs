//! Recipe format, version 0. Mirrors `schemas/recipe.schema.json`.
//!
//! The JSON Schema is the source of truth. These types are a typed view that is
//! deliberately permissive (`serde_json::Value` in places) until the format settles.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The canonical recipe schema, embedded at build time.
pub static RECIPE_SCHEMA: &str = include_str!("../../../schemas/recipe.schema.json");

#[derive(Debug, thiserror::Error)]
pub enum RecipeError {
    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("schema compile error: {0}")]
    SchemaCompile(String),
    #[error("{0}")]
    Migrate(#[from] crate::migrate::MigrateError),
    #[error("recipe failed schema validation:\n{0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recipe {
    pub schema_version: String,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// A glyph name from the app's icon set, shown on cards and in the breadcrumb.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default)]
    pub inputs: IndexMap<String, Value>,
    #[serde(default)]
    pub model_slots: IndexMap<String, ModelSlot>,
    #[serde(default)]
    pub permissions: Option<Value>,
    #[serde(default)]
    pub skills: Vec<String>,
    pub steps: Vec<Step>,
    pub outputs: IndexMap<String, OutputSpec>,
    #[serde(default)]
    pub budget: Option<Value>,
    /// The strip: ordered chapters, each grouping inputs, steps, and outputs with a why.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stages: Vec<Stage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Which Stillroom versions can run this recipe. `minVersion` is stamped on save from the
    /// features the recipe uses, and checked when a bundle loads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<Runtime>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Paths to test case files, relative to the recipe root.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<String>,
    /// Experiments: every `x-` key, kept as written and never read by the engine.
    #[serde(flatten, default, skip_serializing_if = "IndexMap::is_empty")]
    pub extra: IndexMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Author {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Runtime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_version: Option<String>,
}

/// Compare two version strings the way "0.5" and "0.3.0" ask to be compared: numbers, dot by
/// dot, missing parts as zero, anything after a `-` or `+` ignored. Lenient on purpose; an
/// author writes "0.5", Cargo writes "0.5.0", and both mean the same floor.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(v: &str) -> Vec<u64> {
        let core = v.trim().trim_start_matches('v').split(['-', '+']).next().unwrap_or("");
        core.split('.').map(|p| p.trim().parse::<u64>().unwrap_or(0)).collect()
    }
    let (a, b) = (parts(a), parts(b));
    let n = a.len().max(b.len());
    for i in 0..n {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x.cmp(&y);
        }
    }
    std::cmp::Ordering::Equal
}

/// One chapter of a recipe, as the user reads it in the strip.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Stage {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<String>,
    /// A run keep: what this stage holds on to between runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep: Option<KeepSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct KeepSpec {
    #[serde(default)]
    pub refresh: Refresh,
}

/// When a stage with a keep runs again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Refresh {
    /// Every run (logs, anything that reads the world).
    Always,
    /// Only when the inputs and files the stage reads have changed (indexes, parsed material).
    #[default]
    WhenSourcesChange,
    /// Only when the user presses Build now or Rebuild.
    Manual,
}

pub const IMPLIED_STAGE: &str = "make";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSlot {
    pub requires: Vec<Capability>,
    #[serde(default)]
    pub preference: Option<Preference>,
    #[serde(default)]
    pub recommended: Option<String>,
    #[serde(default)]
    pub min_context: Option<u64>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(PartialOrd, Ord, Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Text,
    StructuredOutput,
    ToolCalling,
    Thinking,
    Vision,
    Embedding,
    TextToSpeech,
    SpeechToText,
    ImageGeneration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Preference {
    #[default]
    Accuracy,
    Speed,
    Cost,
    Local,
    Quality,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputSpec {
    pub from: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub editable: Option<bool>,
}

/// One step in a recipe, discriminated by `kind`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Step {
    Tool(ToolStep),
    Model(ModelStep),
    Agent(AgentStep),
    Recipe(OpaqueStep),
    Branch(OpaqueStep),
    Map(MapStep),
    Review(ReviewStep),
}

impl Step {
    pub fn id(&self) -> &str {
        match self {
            Step::Tool(s) => &s.id,
            Step::Model(s) => &s.id,
            Step::Agent(s) => &s.id,
            Step::Map(s) => &s.id,
            Step::Review(s) => &s.id,
            Step::Recipe(s) | Step::Branch(s) => &s.id,
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Step::Tool(s) => s.name.as_deref(),
            Step::Model(s) => s.name.as_deref(),
            Step::Agent(s) => s.name.as_deref(),
            Step::Map(s) => s.name.as_deref(),
            Step::Review(s) => s.name.as_deref(),
            Step::Recipe(s) | Step::Branch(s) => s.name.as_deref(),
        }
    }

    /// The `when` condition, a reference that must resolve truthy for the step to run.
    pub fn when(&self) -> Option<&str> {
        match self {
            Step::Tool(s) => s.when.as_deref(),
            Step::Model(s) => s.when.as_deref(),
            Step::Agent(s) => s.when.as_deref(),
            Step::Map(s) => s.when.as_deref(),
            Step::Review(s) => s.when.as_deref(),
            Step::Recipe(_) | Step::Branch(_) => None,
        }
    }

    /// Whether the output may be reused from an earlier run.
    pub fn checkpoint(&self) -> bool {
        match self {
            Step::Tool(s) => s.checkpoint,
            Step::Model(s) => s.checkpoint,
            Step::Agent(s) => s.checkpoint,
            Step::Map(_) | Step::Review(_) | Step::Recipe(_) | Step::Branch(_) => false,
        }
    }

    pub fn output_schema(&self) -> Option<&Value> {
        match self {
            Step::Tool(s) => s.output_schema.as_ref(),
            Step::Model(s) => s.output_schema.as_ref(),
            Step::Agent(s) => s.output_schema.as_ref(),
            Step::Review(_) => Some(&REVIEW_OUTPUT_SCHEMA),
            Step::Map(_) | Step::Recipe(_) | Step::Branch(_) => None,
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Step::Tool(_) => "tool",
            Step::Model(_) => "model",
            Step::Agent(_) => "agent",
            Step::Recipe(_) => "recipe",
            Step::Branch(_) => "branch",
            Step::Map(_) => "map",
            Step::Review(_) => "review",
        }
    }

    pub fn inputs(&self) -> &IndexMap<String, Value> {
        match self {
            Step::Tool(s) => &s.inputs,
            Step::Model(s) => &s.inputs,
            Step::Agent(s) => &s.inputs,
            Step::Map(s) => &s.inputs,
            Step::Review(s) => &s.inputs,
            Step::Recipe(s) | Step::Branch(s) => &s.inputs,
        }
    }

    pub fn depends_on(&self) -> &[String] {
        match self {
            Step::Tool(s) => &s.depends_on,
            Step::Model(s) => &s.depends_on,
            Step::Agent(s) => &s.depends_on,
            Step::Map(s) => &s.depends_on,
            Step::Review(s) => &s.depends_on,
            Step::Recipe(s) | Step::Branch(s) => &s.depends_on,
        }
    }
}

/// What a review step produces once the user has decided.
pub static REVIEW_OUTPUT_SCHEMA: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
    serde_json::json!({
        "type": "object",
        "required": ["decided", "approved", "rejected"],
        "properties": {
            "decided": { "type": "boolean" },
            "approved": { "type": "array" },
            "rejected": { "type": "array" },
            "items": { "type": "array" }
        }
    })
});

/// Deterministic: always runs one registered capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStep {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub tool: String,
    #[serde(default)]
    pub inputs: IndexMap<String, Value>,
    #[serde(default)]
    pub output_schema: Option<Value>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub when: Option<String>,
    /// If false, this step is never reused from a previous run.
    #[serde(default = "default_true")]
    pub checkpoint: bool,
}

fn default_true() -> bool {
    true
}

/// Deterministic model call: one prompt, one response, no tool use.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStep {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub slot: String,
    pub prompt: String,
    #[serde(default)]
    pub prompt_version: Option<String>,
    #[serde(default)]
    pub inputs: IndexMap<String, Value>,
    #[serde(default)]
    pub output_schema: Option<Value>,
    #[serde(default)]
    pub params: Option<ModelParams>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub when: Option<String>,
    /// If false, this step is never reused from a previous run.
    #[serde(default = "default_true")]
    pub checkpoint: bool,
}

/// Agentic: the model may call tools from `allowed_tools` in a loop, inside the step envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStep {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub slot: String,
    pub prompt: String,
    #[serde(default)]
    pub prompt_version: Option<String>,
    #[serde(default)]
    pub inputs: IndexMap<String, Value>,
    #[serde(default)]
    pub output_schema: Option<Value>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
    #[serde(default = "default_max_tool_calls")]
    pub max_tool_calls: u32,
    #[serde(default)]
    pub params: Option<ModelParams>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub when: Option<String>,
    #[serde(default = "default_true")]
    pub checkpoint: bool,
}

fn default_max_turns() -> u32 {
    10
}
fn default_max_tool_calls() -> u32 {
    20
}

/// Runs its steps once per element of `over`, binding the element to `${<as>}`.
/// Output: an array with one object per item, keyed by inner step id.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapStep {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub over: String,
    #[serde(default = "default_item", rename = "as")]
    pub item_name: String,
    #[serde(default = "default_concurrency")]
    pub concurrency: u32,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub inputs: IndexMap<String, Value>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub when: Option<String>,
}

fn default_item() -> String {
    "item".into()
}
fn default_concurrency() -> u32 {
    4
}

/// Human checkpoint: the run pauses with status needs_review until the user decides per item.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewStep {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub items: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub show: Vec<String>,
    #[serde(default)]
    pub inputs: IndexMap<String, Value>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub when: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelParams {
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub think: Option<Value>,
    #[serde(default)]
    pub context_length: Option<u32>,
}

/// Step kinds the executor does not run yet. Parsed loosely so recipes stay loadable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpaqueStep {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub inputs: IndexMap<String, Value>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(flatten)]
    pub rest: serde_json::Map<String, Value>,
}

impl Recipe {
    /// Parse a `recipe.yaml` string, bring it up to the current format, validate it against the
    /// JSON Schema, and return typed data.
    pub fn from_yaml(yaml: &str) -> Result<Self, RecipeError> {
        let value: Value = serde_yaml::from_str(yaml)?;
        let value = crate::migrate::migrate(value)?.yaml;
        Self::validate_value(&value)?;
        Ok(serde_json::from_value(value)?)
    }

    /// Validate an already-parsed JSON value against the recipe schema.
    pub fn validate_value(value: &Value) -> Result<(), RecipeError> {
        let schema: Value = serde_json::from_str(RECIPE_SCHEMA)?;
        let validator = jsonschema::validator_for(&schema)
            .map_err(|e| RecipeError::SchemaCompile(e.to_string()))?;
        let errors: Vec<String> = validator
            .iter_errors(value)
            .map(|e| format!("- {} at {}", e, e.instance_path))
            .collect();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(RecipeError::Invalid(errors.join("\n")))
        }
    }

    pub fn step(&self, id: &str) -> Option<&Step> {
        self.steps.iter().find(|s| s.id() == id)
    }

    /// The strip as it is drawn: the declared stages, with anything unassigned placed where it
    /// belongs (inputs in the first stage, steps and outputs in a final "Make" stage). A recipe
    /// with no stages is one stage, Make, holding everything.
    pub fn stages_or_implied(&self) -> Vec<Stage> {
        let mut stages = self.stages.clone();
        let assigned = |f: &dyn Fn(&Stage) -> &Vec<String>, name: &str| stages.iter().any(|s| f(s).iter().any(|x| x == name));
        let inputs: Vec<String> = self.inputs.keys().filter(|k| !assigned(&|s| &s.inputs, k)).cloned().collect();
        let steps: Vec<String> = self.steps.iter().map(|s| s.id().to_string()).filter(|k| !assigned(&|s| &s.steps, k)).collect();
        let outputs: Vec<String> = self.outputs.keys().filter(|k| !assigned(&|s| &s.outputs, k)).cloned().collect();
        if stages.is_empty() {
            let mut make = Stage { id: IMPLIED_STAGE.into(), name: "Make".into(), why: None, inputs, steps, outputs, keep: None };
            make.why = Some(self.plain_why(&make));
            return vec![make];
        }
        if !inputs.is_empty() {
            stages[0].inputs.extend(inputs);
        }
        if !steps.is_empty() || !outputs.is_empty() {
            match stages.iter_mut().find(|s| s.id == IMPLIED_STAGE) {
                Some(make) => {
                    make.steps.extend(steps);
                    make.outputs.extend(outputs);
                }
                None => stages.push(Stage { id: IMPLIED_STAGE.into(), name: "Make".into(), why: None, inputs: Vec::new(), steps, outputs, keep: None }),
            }
        }
        for st in stages.iter_mut() {
            if st.why.as_deref().map(str::trim).unwrap_or("").is_empty() {
                st.why = Some(self.plain_why(st));
            }
        }
        stages
    }

    /// A `why` written from what the stage holds, for a stage whose author wrote none: what it
    /// takes, which of its steps are AI and which are programs, what it gives, whether it is kept.
    pub fn plain_why(&self, st: &Stage) -> String {
        fn join(words: &[String]) -> String {
            match words {
                [] => String::new(),
                [a] => a.clone(),
                [a, b] => format!("{a} and {b}"),
                _ => format!("{}, and {}", words[..words.len() - 1].join(", "), words[words.len() - 1]),
            }
        }
        let label = |spec: &Value, name: &str| spec.get("label").and_then(Value::as_str).unwrap_or(name).to_string();
        let mut parts = Vec::new();
        let inputs: Vec<String> = st.inputs.iter().filter_map(|n| self.inputs.get(n).map(|s| label(s, n))).collect();
        if !inputs.is_empty() {
            parts.push(format!("Takes {}.", join(&inputs)));
        }
        let steps: Vec<String> = st
            .steps
            .iter()
            .filter_map(|id| self.steps.iter().find(|s| s.id() == id))
            .map(|s| {
                let what = match s.kind_name() {
                    "model" | "agent" => "AI",
                    "review" => "you approve",
                    "map" => "once per item",
                    _ => "a program",
                };
                format!("{} ({what})", s.name().unwrap_or(s.id()))
            })
            .collect();
        if !steps.is_empty() {
            parts.push(format!("Runs {}.", join(&steps)));
        }
        let outputs: Vec<String> = st.outputs.iter().filter_map(|n| self.outputs.get(n).map(|o| o.label.clone().unwrap_or_else(|| n.clone()))).collect();
        if !outputs.is_empty() {
            parts.push(format!("Gives you {}.", join(&outputs)));
        }
        if let Some(k) = &st.keep {
            parts.push(match k.refresh {
                Refresh::Always => "Remembered between runs; runs every time.".to_string(),
                Refresh::WhenSourcesChange => "Remembered between runs; built again only when what it reads changes.".to_string(),
                Refresh::Manual => "Remembered between runs until you rebuild it.".to_string(),
            });
        }
        if parts.is_empty() {
            "Nothing assigned to this stage yet.".to_string()
        } else {
            parts.join(" ")
        }
    }

    /// Declared inputs a run can start without: optional ones (`required: false`, default or
    /// null) and any input with a `default`, which a planned recipe uses to carry the links or
    /// notes the recipe was made from.
    pub fn optional_inputs(&self) -> Vec<(String, Value)> {
        self.inputs
            .iter()
            .filter(|(_, spec)| spec.get("required").and_then(Value::as_bool) == Some(false) || spec.get("default").is_some())
            .map(|(k, spec)| (k.clone(), spec.get("default").cloned().unwrap_or(Value::Null)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const MINIMAL: &str = r#"
schemaVersion: "0"
id: hello-world
name: Hello World
modelSlots:
  writer:
    requires: [text]
    preference: speed
steps:
  - id: summarize
    kind: model
    slot: writer
    prompt: prompts/summarize.md
    inputs:
      text: ${inputs.source}
inputs:
  source:
    type: text
outputs:
  summary:
    from: ${steps.summarize.output}
    type: markdown
"#;

    #[test]
    fn minimal_recipe_validates() {
        let r = Recipe::from_yaml(MINIMAL).expect("valid recipe");
        assert_eq!(r.id, "hello-world");
        assert_eq!(r.model_slots["writer"].requires, vec![Capability::Text]);
        assert!(matches!(r.steps[0], Step::Model(_)));
        assert_eq!(r.outputs["summary"].kind, "markdown");
    }

    #[test]
    fn versions_compare_by_number_not_by_text() {
        use std::cmp::Ordering::*;
        assert_eq!(compare_versions("0.5", "0.3.0"), Greater);
        assert_eq!(compare_versions("0.3", "0.3.0"), Equal);
        assert_eq!(compare_versions("0.10.0", "0.9.9"), Greater);
        assert_eq!(compare_versions("1.0.0-beta.1", "0.9"), Greater);
        assert_eq!(compare_versions("v0.2.0", "0.2.0"), Equal);
        assert_eq!(compare_versions("", "0.0.0"), Equal);
    }

    #[test]
    fn a_recipe_round_trips_every_schema_field() {
        let yaml = include_str!("../../../recipes/research-brief/recipe.yaml");
        let r = Recipe::from_yaml(yaml).unwrap();
        assert_eq!(r.author.as_ref().and_then(|a| a.name.as_deref()), Some("Stillroom"));
        assert_eq!(r.license.as_deref(), Some("MIT"));
        assert!(r.tags.contains(&"research".to_string()));
        let with_runtime = yaml.replace("license: MIT", "license: MIT\nruntime: { minVersion: \"0.3\" }\ntests: [tests/a.json]");
        let r = Recipe::from_yaml(&with_runtime).unwrap();
        assert_eq!(r.runtime.as_ref().and_then(|x| x.min_version.as_deref()), Some("0.3"));
        assert_eq!(r.tests, vec!["tests/a.json"]);
        let back: Value = serde_json::to_value(&r).unwrap();
        assert_eq!(back["runtime"]["minVersion"], "0.3");
        assert_eq!(back["author"]["name"], "Stillroom");
        assert!(back["runtime"].get("maxVersion").is_none(), "an unset field is left out, not written as null");
        assert!(back.get("tests").is_some() && back.get("license").is_some());
    }

    #[test]
    fn x_keys_ride_along_at_every_level() {
        let yaml = r#"
schemaVersion: "0"
id: x-everywhere
name: Experiments everywhere
x-shelf: { pinned: true }
inputs:
  source: { type: text, x-shelf: { pinned: true } }
modelSlots:
  writer: { requires: [text], x-shelf: { pinned: true } }
steps:
  - id: summarize
    kind: model
    slot: writer
    prompt: prompts/summarize.md
    inputs: { text: "${inputs.source}" }
    x-shelf: { pinned: true }
outputs:
  summary: { from: "${steps.summarize.output}", type: markdown, x-shelf: { pinned: true } }
stages:
  - { id: make, name: Make, steps: [summarize], outputs: [summary], x-shelf: { pinned: true } }
"#;
        let r = Recipe::from_yaml(yaml).expect("x- keys are allowed at every level");
        assert_eq!(r.extra["x-shelf"]["pinned"], true, "the top-level experiment is kept on the typed recipe");
        let bad = yaml.replace("x-shelf: { pinned: true }
inputs:", "y-shelf: { pinned: true }
inputs:");
        assert!(Recipe::from_yaml(&bad).is_err(), "only the x- prefix is open");
        let d = crate::draft::Draft { yaml: serde_yaml::from_str(yaml).unwrap(), files: [("prompts/summarize.md".to_string(), "Sum.".to_string())].into_iter().collect() };
        assert!(d.check(&["file.read".into()]).is_empty(), "{:?}", d.check(&["file.read".into()]));
        let tmp = std::env::temp_dir().join(format!("stillroom-x-{}", uuid::Uuid::now_v7()));
        d.write(&tmp, &["file.read".into()]).unwrap();
        let written = std::fs::read_to_string(tmp.join("recipe.yaml")).unwrap();
        assert_eq!(written.matches("x-shelf:").count(), 6, "every x- key survives a save:\n{written}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn bad_id_is_rejected() {
        let bad = MINIMAL.replace("id: hello-world", "id: Hello World");
        let err = Recipe::from_yaml(&bad).unwrap_err();
        assert!(matches!(err, RecipeError::Invalid(_)), "{err}");
    }

    #[test]
    fn unknown_step_kind_is_rejected() {
        let bad = MINIMAL.replace("kind: model", "kind: magic");
        assert!(Recipe::from_yaml(&bad).is_err());
    }

    #[test]
    fn builtin_summarize_recipe_loads() {
        let yaml = include_str!("../../../recipes/summarize-file/recipe.yaml");
        let r = Recipe::from_yaml(yaml).expect("built-in recipe is valid");
        assert_eq!(r.steps.len(), 2);
        assert!(matches!(r.steps[0], Step::Tool(_)));
    }
}
