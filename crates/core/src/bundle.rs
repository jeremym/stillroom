//! A recipe as it lives on disk: `recipe.yaml` plus its prompt and schema files.

use crate::recipe::{Recipe, RecipeError, Step};
use crate::resolve::Resolution;
use crate::store::ShelfState;
use crate::RunRequest;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecipeSource {
    /// Ships with the app, written here (`recipes/`).
    Builtin,
    /// Ships with the app, contributed through the repository and reviewed before it shipped (`community/`).
    Community,
    /// Ships with the app as a worked example that needs its sample folders copied first (`samples/`).
    Sample,
    /// The user's own, in the app data folder.
    User,
}

impl RecipeSource {
    /// The user's own recipe, as opposed to one from the catalog (built-in, community, or sample).
    pub fn is_user(self) -> bool {
        self == RecipeSource::User
    }
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
    #[error("This recipe needs Stillroom {needs} or newer; you have {have}.")]
    NeedsNewer { needs: String, have: String },
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
    /// Who wrote it, when the recipe says (`author: { name, url }`).
    #[serde(default)]
    pub author: Option<crate::recipe::Author>,
    /// The recipe's own tags (`documents`, `research`, `sample`, ...); the catalog filters by them.
    #[serde(default)]
    pub tags: Vec<String>,
    pub inputs: IndexMap<String, Value>,
    pub model_slots: IndexMap<String, crate::recipe::ModelSlot>,
    pub permissions: Option<Value>,
    pub steps: Vec<StepInfo>,
    pub outputs: IndexMap<String, crate::recipe::OutputSpec>,
    /// The strip as drawn: declared stages with anything unassigned placed, or one implied stage.
    pub stages: Vec<crate::recipe::Stage>,
    /// Where the recipe sits for the Library: on the shelf, in the archive, or (a catalog recipe
    /// only) in the catalog. `info()` gives the default for the source; the shell applies the
    /// store's placement through `with_shelf`.
    #[serde(default)]
    pub shelf: ShelfState,
}

impl RecipeInfo {
    /// Apply what the shelf table says, if anything. A recipe of the user's own is on the shelf
    /// unless archived (never in the catalog); a catalog recipe (built-in, community, sample) is
    /// in the catalog unless placed.
    pub fn with_shelf(mut self, placed: Option<ShelfState>) -> Self {
        self.shelf = match (self.source, placed) {
            (RecipeSource::User, Some(ShelfState::Archive)) => ShelfState::Archive,
            (RecipeSource::User, _) => ShelfState::Shelf,
            (_, Some(s)) => s,
            (_, None) => ShelfState::Catalog,
        };
        self
    }
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
    /// `outputSchema` given as a file path. The version floor is judged against this crate's
    /// own version; the shell passes its own through `load_for`.
    pub fn load(root: impl Into<PathBuf>, source: RecipeSource) -> Result<Self, BundleError> {
        Self::load_for(root, source, env!("CARGO_PKG_VERSION"))
    }

    /// `load`, with the running app's version: a recipe whose `runtime.minVersion` is above what
    /// this build understands is refused in words before anything else reads it, so a recipe
    /// from a newer Stillroom never surfaces as a schema message.
    pub fn load_for(root: impl Into<PathBuf>, source: RecipeSource, app_version: &str) -> Result<Self, BundleError> {
        let root = root.into();
        let yaml = read(&root, "recipe.yaml")?;
        if let Some(needs) = serde_yaml::from_str::<Value>(&yaml).ok().and_then(|v| v.get("runtime")?.get("minVersion")?.as_str().map(String::from)) {
            let have = crate::migrate::supported_version(app_version);
            if crate::recipe::compare_versions(&needs, &have) == std::cmp::Ordering::Greater {
                return Err(BundleError::NeedsNewer { needs, have: app_version.to_string() });
            }
        }
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
            author: self.recipe.author.clone(),
            tags: self.recipe.tags.clone(),
            inputs: self.recipe.inputs.clone(),
            model_slots: self.recipe.model_slots.clone(),
            permissions: self.recipe.permissions.clone(),
            steps: self.recipe.steps.iter().map(step_info).collect(),
            outputs: self.recipe.outputs.clone(),
            stages: self.recipe.stages_or_implied(),
            shelf: ShelfState::Catalog,
        }
        .with_shelf(None)
    }

    pub fn request(&self, inputs: IndexMap<String, Value>, resolutions: BTreeMap<String, Resolution>) -> RunRequest {
        RunRequest {
            keep_base: None,
            rebuild: Vec::new(),
            recipe: self.recipe.clone(),
            prompts: self.prompts.clone(),
            inputs,
            resolutions,
            artifact_base: None,
            reuse: true,
            edits: BTreeMap::new(),
            resume_from: None,
            trigger: None,
            cancel: None,
            model_timeout: None,
            model_defaults: Default::default(),
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
        assert_eq!(info.author.as_ref().and_then(|a| a.name.as_deref()), Some("Stillroom"));
        assert!(info.tags.contains(&"built-in".to_string()), "{:?}", info.tags);
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["author"]["name"], "Stillroom");
        assert!(json["tags"].is_array());
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
        let b = RecipeBundle::load(repo_recipes().join("../samples/inbox-triage"), RecipeSource::User).unwrap();
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
    fn a_recipe_from_a_newer_stillroom_is_refused_in_words() {
        let tmp = std::env::temp_dir().join(format!("stillroom-needs-newer-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(tmp.join("prompts")).unwrap();
        std::fs::write(tmp.join("prompts/summarize.md"), "Summarize.").unwrap();
        let yaml = std::fs::read_to_string(repo_recipes().join("summarize-file/recipe.yaml")).unwrap().replace("license: MIT", "license: MIT\nruntime: { minVersion: \"0.5\" }\nnewThing: true");
        std::fs::write(tmp.join("recipe.yaml"), &yaml).unwrap();
        let err = RecipeBundle::load_for(&tmp, RecipeSource::User, "0.3.0").unwrap_err();
        assert!(matches!(err, BundleError::NeedsNewer { .. }), "{err}");
        assert_eq!(err.to_string(), "This recipe needs Stillroom 0.5 or newer; you have 0.3.0.");
        // The same recipe on a build that is new enough gets the ordinary schema message instead.
        let err = RecipeBundle::load_for(&tmp, RecipeSource::User, "0.5.0").unwrap_err();
        assert!(matches!(err, BundleError::Recipe(_)), "{err}");
        // And a floor this build's features already cover loads, whatever Cargo says the version is.
        std::fs::write(tmp.join("recipe.yaml"), yaml.replace("\nnewThing: true", "").replace("\"0.5\"", "\"0.3\"")).unwrap();
        assert!(RecipeBundle::load_for(&tmp, RecipeSource::User, "0.2.0").is_ok());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn placement_follows_the_source_unless_the_shelf_says_otherwise() {
        let builtin = RecipeBundle::load(repo_recipes().join("summarize-file"), RecipeSource::Builtin).unwrap().info();
        assert_eq!(builtin.shelf, ShelfState::Catalog, "a built-in starts in the catalog");
        assert_eq!(builtin.clone().with_shelf(Some(ShelfState::Shelf)).shelf, ShelfState::Shelf);
        assert_eq!(builtin.clone().with_shelf(Some(ShelfState::Archive)).shelf, ShelfState::Archive);
        let yours = RecipeBundle::load(repo_recipes().join("../samples/inbox-triage"), RecipeSource::User).unwrap().info();
        assert_eq!(yours.shelf, ShelfState::Shelf, "your own starts on the shelf");
        assert_eq!(yours.clone().with_shelf(Some(ShelfState::Archive)).shelf, ShelfState::Archive);
        assert_eq!(yours.clone().with_shelf(Some(ShelfState::Catalog)).shelf, ShelfState::Shelf, "never in the catalog");
        assert_eq!(serde_json::to_value(&builtin).unwrap()["shelf"], serde_json::json!("catalog"));
        let sample = RecipeBundle::load(repo_recipes().join("../samples/inbox-triage"), RecipeSource::Sample).unwrap().info();
        assert_eq!(sample.shelf, ShelfState::Catalog, "a sample starts in the catalog too");
        assert_eq!(sample.clone().with_shelf(Some(ShelfState::Shelf)).shelf, ShelfState::Shelf);
        assert_eq!(serde_json::to_value(&sample).unwrap()["source"], serde_json::json!("sample"));
    }

    #[test]
    fn refuses_paths_that_escape() {
        assert!(safe_join(Path::new("/r"), "../x.md").is_err());
        assert!(safe_join(Path::new("/r"), "/etc/passwd").is_err());
        assert!(safe_join(Path::new("/r"), "prompts/a.md").is_ok());
    }
}
