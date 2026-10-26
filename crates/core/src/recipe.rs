//! Recipe format, version 0. Mirrors `schemas/recipe.schema.json`.
//!
//! The JSON Schema is the source of truth. These types are a typed view that is
//! deliberately permissive (`serde_json::Value` in places) until the format settles.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

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
    #[serde(default)]
    pub inputs: BTreeMap<String, Value>,
    #[serde(default)]
    pub model_slots: BTreeMap<String, ModelSlot>,
    #[serde(default)]
    pub permissions: Option<Value>,
    #[serde(default)]
    pub skills: Vec<String>,
    pub steps: Vec<Step>,
    pub outputs: BTreeMap<String, OutputSpec>,
    #[serde(default)]
    pub budget: Option<Value>,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    Agent(OpaqueStep),
    Recipe(OpaqueStep),
    Branch(OpaqueStep),
    Map(OpaqueStep),
}

impl Step {
    pub fn id(&self) -> &str {
        match self {
            Step::Tool(s) => &s.id,
            Step::Model(s) => &s.id,
            Step::Agent(s) | Step::Recipe(s) | Step::Branch(s) | Step::Map(s) => &s.id,
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Step::Tool(s) => s.name.as_deref(),
            Step::Model(s) => s.name.as_deref(),
            Step::Agent(s) | Step::Recipe(s) | Step::Branch(s) | Step::Map(s) => s.name.as_deref(),
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
        }
    }

    pub fn inputs(&self) -> &BTreeMap<String, Value> {
        match self {
            Step::Tool(s) => &s.inputs,
            Step::Model(s) => &s.inputs,
            Step::Agent(s) | Step::Recipe(s) | Step::Branch(s) | Step::Map(s) => &s.inputs,
        }
    }

    pub fn depends_on(&self) -> &[String] {
        match self {
            Step::Tool(s) => &s.depends_on,
            Step::Model(s) => &s.depends_on,
            Step::Agent(s) | Step::Recipe(s) | Step::Branch(s) | Step::Map(s) => &s.depends_on,
        }
    }
}

/// Deterministic: always runs one registered capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStep {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub tool: String,
    #[serde(default)]
    pub inputs: BTreeMap<String, Value>,
    #[serde(default)]
    pub output_schema: Option<Value>,
    #[serde(default)]
    pub depends_on: Vec<String>,
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
    pub inputs: BTreeMap<String, Value>,
    #[serde(default)]
    pub output_schema: Option<Value>,
    #[serde(default)]
    pub params: Option<ModelParams>,
    #[serde(default)]
    pub depends_on: Vec<String>,
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
}

/// Step kinds the executor does not run yet. Parsed loosely so recipes stay loadable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpaqueStep {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub inputs: BTreeMap<String, Value>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(flatten)]
    pub rest: serde_json::Map<String, Value>,
}

impl Recipe {
    /// Parse a `recipe.yaml` string, validate it against the JSON Schema, and return typed data.
    pub fn from_yaml(yaml: &str) -> Result<Self, RecipeError> {
        let value: Value = serde_yaml::from_str(yaml)?;
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
