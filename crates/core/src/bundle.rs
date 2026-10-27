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
    pub icon: Option<String>,
    pub source: RecipeSource,
    pub path: String,
    pub inputs: IndexMap<String, Value>,
    pub model_slots: IndexMap<String, crate::recipe::ModelSlot>,
    pub permissions: Option<Value>,
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
    /// Step ids this step reads from.
    pub depends_on: Vec<String>,
    /// Recipe inputs this step reads (for a map, including its inner steps).
    #[serde(default)]
    pub uses_inputs: Vec<String>,
    /// The condition that must be set for the step to run, if any.
    #[serde(default)]
    pub when: Option<String>,
    /// For map steps: the inner steps.
    #[serde(default)]
    pub steps: Vec<StepInfo>,
    /// For map steps: the name each item is bound to (`as:`).
    #[serde(default)]
    pub item_name: Option<String>,
    /// For agent steps: the tools it may call.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// For review steps: what to show per item.
    #[serde(default)]
    pub show: Vec<String>,
    #[serde(default)]
    pub title: Option<String>,
}

fn step_info(s: &Step) -> StepInfo {
    StepInfo {
        id: s.id().into(),
        kind: s.kind_name().into(),
        name: s.name().map(String::from),
        slot: match s {
            Step::Model(m) => Some(m.slot.clone()),
            Step::Agent(a) => Some(a.slot.clone()),
            _ => None,
        },
        tool: match s {
            Step::Tool(t) => Some(t.tool.clone()),
            _ => None,
        },
        depends_on: crate::propagate::dependencies(s).into_iter().collect(),
        uses_inputs: crate::propagate::input_dependencies(s).into_iter().collect(),
        when: s.when().map(String::from),
        steps: match s {
            Step::Map(m) => m.steps.iter().map(step_info).collect(),
            _ => Vec::new(),
        },
        item_name: match s {
            Step::Map(m) => Some(m.item_name.clone()),
            _ => None,
        },
        allowed_tools: match s {
            Step::Agent(a) => a.allowed_tools.clone(),
            _ => Vec::new(),
        },
        show: match s {
            Step::Review(r) => r.show.clone(),
            _ => Vec::new(),
        },
        title: match s {
            Step::Review(r) => r.title.clone(),
            _ => None,
        },
    }
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

/// Read the prompt and inline the schema of one step, recursing into map steps.
fn load_step(root: &Path, step: &mut Step, prompts: &mut HashMap<String, String>) -> Result<(), BundleError> {
    let schema_slot = match step {
        Step::Model(m) => {
            if !prompts.contains_key(&m.prompt) {
                prompts.insert(m.prompt.clone(), read(root, &m.prompt)?);
            }
            &mut m.output_schema
        }
        Step::Agent(a) => {
            if !prompts.contains_key(&a.prompt) {
                prompts.insert(a.prompt.clone(), read(root, &a.prompt)?);
            }
            &mut a.output_schema
        }
        Step::Tool(t) => &mut t.output_schema,
        Step::Map(m) => {
            for sub in &mut m.steps {
                load_step(root, sub, prompts)?;
            }
            return Ok(());
        }
        _ => return Ok(()),
    };
    if let Some(Value::String(rel)) = schema_slot {
        let rel = rel.clone();
        let path = safe_join(root, &rel)?;
        let text = read(root, &rel)?;
        let v: Value = serde_json::from_str(&text).map_err(|source| BundleError::Json { path, source })?;
        *schema_slot = Some(v);
    }
    Ok(())
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
            load_step(&root, step, &mut prompts)?;
        }
        Ok(Self { root, source, recipe, prompts })
    }

    pub fn info(&self) -> RecipeInfo {
        RecipeInfo {
            id: self.recipe.id.clone(),
            name: self.recipe.name.clone(),
            version: self.recipe.version.clone(),
            description: self.recipe.description.clone(),
            icon: self.recipe.icon.clone(),
            source: self.source,
            path: self.root.to_string_lossy().into_owned(),
            inputs: self.recipe.inputs.clone(),
            model_slots: self.recipe.model_slots.clone(),
            permissions: self.recipe.permissions.clone(),
            steps: self.recipe.steps.iter().map(step_info).collect(),
            outputs: self.recipe.outputs.clone(),
        }
    }

    pub fn request(&self, inputs: IndexMap<String, Value>, resolutions: BTreeMap<String, Resolution>) -> RunRequest {
        RunRequest {
            recipe: self.recipe.clone(),
            prompts: self.prompts.clone(),
            inputs,
            resolutions,
            artifact_base: None,
            reuse: true,
            edits: BTreeMap::new(),
            resume_from: None,
            trigger: None,
        }
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
        assert_eq!(info.steps[1].depends_on, vec!["read".to_string()]);
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
    fn loads_inbox_triage_with_nested_prompts_and_schemas() {
        let b = RecipeBundle::load(repo_recipes().join("inbox-triage"), RecipeSource::Builtin).unwrap();
        assert_eq!(b.prompts.len(), 3, "{:?}", b.prompts.keys());
        let info = b.info();
        let triage = info.steps.iter().find(|s| s.id == "triage").unwrap();
        assert_eq!(triage.kind, "map");
        assert_eq!(triage.steps.len(), 4);
        assert_eq!(triage.steps[3].allowed_tools, vec!["fs.list", "fs.write"]);
        assert_eq!(triage.depends_on, vec!["list".to_string()], "inner refs to siblings are not map deps");
        assert_eq!(triage.uses_inputs, vec!["about".to_string(), "archive".to_string()], "map inputs include inner steps' inputs");
        assert_eq!(triage.steps[2].when.as_deref(), Some("${steps.classify.output.needsReply}"));
        assert_eq!(triage.steps[2].depends_on, vec!["classify".to_string(), "parse".to_string()]);
        let review = info.steps.iter().find(|s| s.id == "review").unwrap();
        assert_eq!(review.show.len(), 4);
        match b.recipe.step("triage").unwrap() {
            Step::Map(m) => match &m.steps[1] {
                Step::Model(c) => assert!(c.output_schema.as_ref().unwrap().is_object(), "nested schema inlined"),
                _ => panic!(),
            },
            _ => panic!(),
        }
        assert!(info.permissions.is_some());
    }

    #[test]
    fn refuses_paths_that_escape() {
        assert!(safe_join(Path::new("/r"), "../x.md").is_err());
        assert!(safe_join(Path::new("/r"), "/etc/passwd").is_err());
        assert!(safe_join(Path::new("/r"), "prompts/a.md").is_ok());
    }
}
