//! Recipe format, version 0. Mirrors `schemas/recipe.schema.json`.
//!
//! The JSON Schema is the source of truth. These types are a typed view that is
//! deliberately permissive (`serde_json::Value` in places) until the format settles.

use serde::{Deserialize, Serialize};
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
    pub inputs: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub model_slots: BTreeMap<String, ModelSlot>,
    #[serde(default)]
    pub permissions: Option<serde_json::Value>,
    #[serde(default)]
    pub skills: Vec<String>,
    pub steps: Vec<serde_json::Value>,
    pub outputs: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub budget: Option<serde_json::Value>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preference {
    Accuracy,
    Speed,
    Cost,
    Local,
    Quality,
}

impl Recipe {
    /// Parse a `recipe.yaml` string, validate it against the JSON Schema, and return typed data.
    pub fn from_yaml(yaml: &str) -> Result<Self, RecipeError> {
        let value: serde_json::Value = serde_yaml::from_str(yaml)?;
        Self::validate_value(&value)?;
        Ok(serde_json::from_value(value)?)
    }

    /// Validate an already-parsed JSON value against the recipe schema.
    pub fn validate_value(value: &serde_json::Value) -> Result<(), RecipeError> {
        let schema: serde_json::Value = serde_json::from_str(RECIPE_SCHEMA)?;
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
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
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
}
