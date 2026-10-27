//! A recipe as it lives on disk: `recipe.yaml` plus its prompt and schema files.

use crate::recipe::{Recipe, RecipeError, Step};
use crate::resolve::Resolution;
use crate::RunRequest;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecipeSource {
    Builtin,
    User,
}

#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("{0}")]
    Recipe(#[from] RecipeError),
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{path}: {source}")]
    Json { path: PathBuf, source: serde_json::Error },
    #[error("path {0} escapes the recipe directory")]
    Escape(String),
}

#[derive(Debug, Clone)]
pub struct RecipeBundle {
    pub root: PathBuf,
    pub source: RecipeSource,
    pub recipe: Recipe,
    /// Prompt file contents keyed by their path inside the bundle.
    pub prompts: HashMap<String, String>,
}

/// What the UI needs to list and configure a recipe.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeInfo {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub source: RecipeSource,
    pub path: String,
    pub inputs: IndexMap<String, Value>,
    pub model_slots: IndexMap<String, crate::recipe::ModelSlot>,
    pub steps: Vec<StepInfo>,
    pub outputs: IndexMap<String, crate::recipe::OutputSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepInfo {
    pub id: String,
    pub kind: String,
    pub name: Option<String>,
    pub slot: Option<String>,
    pub tool: Option<String>,
}

/// Join a bundle-relative path, refusing anything absolute or containing `..`.
fn safe_join(root: &Path, rel: &str) -> Result<PathBuf, BundleError> {
    let p = Path::new(rel);
    if p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir | Component::Prefix(_))) {
        return Err(BundleError::Escape(rel.to_string()));
    }
    Ok(root.join(p))
}

fn read(root: &Path, rel: &str) -> Result<String, BundleError> {
    let path = safe_join(root, rel)?;
    std::fs::read_to_string(&path).map_err(|source| BundleError::Io { path, source })
}

impl RecipeBundle {
    /// Load `root/recipe.yaml`, every prompt a model step references, and inline any
    /// `outputSchema` given as a file path.
    pub fn load(root: impl Into<PathBuf>, source: RecipeSource) -> Result<Self, BundleError> {
        let root = root.into();
        let yaml = read(&root, "recipe.yaml")?;
        let mut recipe = Recipe::from_yaml(&yaml)?;
        let mut prompts = HashMap::new();

        for step in &mut recipe.steps {
            let schema_slot = match step {
                Step::Model(m) => {
                    if !prompts.contains_key(&m.prompt) {
                        prompts.insert(m.prompt.clone(), read(&root, &m.prompt)?);
                    }
                    &mut m.output_schema
                }
                Step::Tool(t) => &mut t.output_schema,
                _ => continue,
            };
            if let Some(Value::String(rel)) = schema_slot {
                let rel = rel.clone();
                let path = safe_join(&root, &rel)?;
                let text = read(&root, &rel)?;
                let v: Value = serde_json::from_str(&text).map_err(|source| BundleError::Json { path, source })?;
                *schema_slot = Some(v);
            }
        }
        Ok(Self { root, source, recipe, prompts })
    }

    pub fn info(&self) -> RecipeInfo {
        RecipeInfo {
            id: self.recipe.id.clone(),
            name: self.recipe.name.clone(),
            version: self.recipe.version.clone(),
            description: self.recipe.description.clone(),
            source: self.source,
            path: self.root.to_string_lossy().into_owned(),
            inputs: self.recipe.inputs.clone(),
            model_slots: self.recipe.model_slots.clone(),
            steps: self
                .recipe
                .steps
                .iter()
                .map(|s| StepInfo {
                    id: s.id().into(),
                    kind: s.kind_name().into(),
                    name: s.name().map(String::from),
                    slot: match s {
                        Step::Model(m) => Some(m.slot.clone()),
                        _ => None,
                    },
                    tool: match s {
                        Step::Tool(t) => Some(t.tool.clone()),
                        _ => None,
                    },
                })
                .collect(),
            outputs: self.recipe.outputs.clone(),
        }
    }

    pub fn request(&self, inputs: IndexMap<String, Value>, resolutions: BTreeMap<String, Resolution>) -> RunRequest {
        RunRequest { recipe: self.recipe.clone(), prompts: self.prompts.clone(), inputs, resolutions, artifact_base: None, reuse: true }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_recipes() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../recipes")
    }

    #[test]
    fn loads_summarize_file_bundle() {
        let b = RecipeBundle::load(repo_recipes().join("summarize-file"), RecipeSource::Builtin).unwrap();
        assert_eq!(b.recipe.id, "summarize-file");
        assert!(b.prompts.contains_key("prompts/summarize.md"));
        let info = b.info();
        assert_eq!(info.steps[1].slot.as_deref(), Some("writer"));
    }

    #[test]
    fn loads_learning_kit_and_inlines_schemas() {
        let b = RecipeBundle::load(repo_recipes().join("learning-kit"), RecipeSource::Builtin).unwrap();
        let extract = b.recipe.step("extract").unwrap();
        match extract {
            Step::Model(m) => assert!(m.output_schema.as_ref().unwrap().is_object(), "schema inlined"),
            _ => panic!("extract should be a model step"),
        }
        assert_eq!(b.prompts.len(), 6);
    }

    #[test]
    fn refuses_paths_that_escape() {
        assert!(safe_join(Path::new("/r"), "../x.md").is_err());
        assert!(safe_join(Path::new("/r"), "/etc/passwd").is_err());
        assert!(safe_join(Path::new("/r"), "prompts/a.md").is_ok());
    }
}
