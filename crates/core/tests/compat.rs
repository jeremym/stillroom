//! Compatibility: frozen recipes and planner answers that must keep loading. A format change is a
//! change to `tests/compat/` first; a fixture is only replaced when a migration is written for it.
//!
//! - `compat/recipes/v0-pre-stages/`: the four recipes as they were on `main` before stages.
//! - `compat/recipes/v0-stages/`: the six built-ins and Inbox Triage (then `samples/inbox-triage`, now a
//!   built-in) as of the stages work.
//! - `compat/recipes/v0-notes/`: hand-made recipes that are fine but out of date in one way each,
//!   so the checker's Notes and their Fixes are pinned here too.
//! - `compat/planner-answers/`: planner answers in the shapes the merge rules were written for.

use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use stillroom_core::planner::{answer_schema, draft_from_answers, strip_schema, PlanRequest};
use stillroom_core::stagelib::{repo_stages_dir, StageLibrary};
use stillroom_core::tools::ToolRegistry;
use stillroom_core::{migrate, Draft, RecipeBundle, RecipeSource, Severity, CURRENT_SCHEMA_VERSION, RECIPE_SCHEMA};

fn compat_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/compat")
}

fn known_tools() -> Vec<String> {
    ToolRegistry::builtin().ids().into_iter().map(String::from).collect()
}

/// Every `<compat>/recipes/<set>/<recipe>/` holding a recipe.yaml, as (set, recipe, dir).
fn fixture_recipes() -> Vec<(String, String, PathBuf)> {
    let mut out = Vec::new();
    let mut sets: Vec<PathBuf> = std::fs::read_dir(compat_dir().join("recipes")).unwrap().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    sets.sort();
    for set in sets {
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(&set).unwrap().flatten().map(|e| e.path()).filter(|p| p.join("recipe.yaml").is_file()).collect();
        dirs.sort();
        for d in dirs {
            out.push((set.file_name().unwrap().to_string_lossy().into_owned(), d.file_name().unwrap().to_string_lossy().into_owned(), d));
        }
    }
    assert!(out.len() >= 13, "expected the four pre-stages recipes, seven stages-era ones, and the notes set; found {}", out.len());
    out
}

/// The Notes each fixture is expected to carry, by `<set>/<recipe>`: (text, fix id). Any fixture
/// not listed must check with nothing to say. When a change adds a Note to a built-in, this is
/// the test that goes red, and the fixture is only moved on once a migration is written for it.
fn expected_notes(set: &str, name: &str) -> Vec<(&'static str, &'static str)> {
    match format!("{set}/{name}").as_str() {
        "v0-notes/undeclared-keep" => vec![("Stage “Remember” writes a keep but does not say so.", "declare_keep:remember")],
        _ => Vec::new(),
    }
}

#[test]
fn every_fixture_recipe_loads_and_checks_without_problems() {
    let tools = known_tools();
    for (set, name, dir) in fixture_recipes() {
        let bundle = RecipeBundle::load(&dir, RecipeSource::Builtin).unwrap_or_else(|e| panic!("{set}/{name}: {e}"));
        assert_eq!(bundle.recipe.id, name, "{set}/{name}: the folder is named after the recipe");
        assert!(!bundle.recipe.stages_or_implied().is_empty(), "{set}/{name}: the strip draws");
        let draft = Draft::read(&dir).unwrap_or_else(|e| panic!("{set}/{name}: {e}"));
        let findings = draft.check(&tools);
        let problems: Vec<&str> = findings.iter().filter(|f| f.severity == Severity::Problem).map(|f| f.text.as_str()).collect();
        assert!(problems.is_empty(), "{set}/{name} has problems: {problems:#?}");
        let notes: Vec<(&str, &str)> = findings.iter().filter(|f| f.severity == Severity::Note).map(|f| (f.text.as_str(), f.fix.as_deref().unwrap_or(""))).collect();
        assert_eq!(notes, expected_notes(&set, &name), "{set}/{name}: notes");
        if !notes.is_empty() {
            // A Note does not stop a save, and its Fix leaves nothing to say.
            let tmp = std::env::temp_dir().join(format!("stillroom-compat-{}", uuid::Uuid::now_v7()));
            draft.write(&tmp, &tools).unwrap_or_else(|e| panic!("{set}/{name}: a Note must not refuse a save: {e}"));
            let _ = std::fs::remove_dir_all(&tmp);
            let fixes: Vec<String> = notes.iter().map(|(_, f)| f.to_string()).collect();
            let fixed = draft.fix(&fixes).unwrap_or_else(|e| panic!("{set}/{name}: {e}"));
            assert!(fixed.check(&tools).is_empty(), "{set}/{name}: fixed, but the checker still says {:?}", fixed.check(&tools));
        }

        // Migration is idempotent on the fixture's YAML as written: a second pass changes nothing
        // and has nothing to say.
        let raw: Value = serde_yaml::from_str(&std::fs::read_to_string(dir.join("recipe.yaml")).unwrap()).unwrap();
        let once = migrate(raw.clone()).unwrap_or_else(|e| panic!("{set}/{name}: {e}"));
        assert_eq!(once.yaml["schemaVersion"], CURRENT_SCHEMA_VERSION, "{set}/{name}");
        let twice = migrate(once.yaml.clone()).unwrap();
        assert_eq!(twice.yaml, once.yaml, "{set}/{name}: migrate is idempotent");
        assert!(twice.notes.is_empty(), "{set}/{name}: a migrated recipe has nothing left to note: {:?}", twice.notes);
        assert_eq!(draft.yaml, once.yaml, "{set}/{name}: Draft::read is migrate");
    }
}

/// Every property name in a planner schema, recursively.
fn property_names(schema: &Value, out: &mut BTreeSet<String>) {
    if let Some(props) = schema.get("properties").and_then(Value::as_object) {
        for (k, v) in props {
            out.insert(k.clone());
            property_names(v, out);
        }
    }
    if let Some(items) = schema.get("items") {
        property_names(items, out);
    }
    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(list) = schema.get(key).and_then(Value::as_array) {
            list.iter().for_each(|s| property_names(s, out));
        }
    }
    if let Some(defs) = schema.get("$defs").and_then(Value::as_object) {
        defs.values().for_each(|s| property_names(s, out));
    }
}

#[test]
fn planner_schemas_name_only_properties_the_recipe_schema_has() {
    let recipe_schema: Value = serde_json::from_str(RECIPE_SCHEMA).unwrap();
    let mut known = BTreeSet::new();
    property_names(&recipe_schema, &mut known);
    // `use` is the planner's own marker for a ready-made stage; it is expanded away before the
    // recipe is checked, so it is not (and must not be) in the recipe schema.
    known.insert("use".to_string());

    let mut asked = BTreeSet::new();
    property_names(&answer_schema()["properties"]["recipe"], &mut asked);
    let lib = StageLibrary::load(&repo_stages_dir()).unwrap();
    property_names(&strip_schema(&lib)["properties"]["stages"], &mut asked);
    let unknown: Vec<&String> = asked.iter().filter(|k| !known.contains(*k)).collect();
    assert!(unknown.is_empty(), "the planner asks for properties the recipe format does not have: {unknown:?}");
    assert!(asked.contains("stages") && asked.contains("keep") && asked.contains("why"), "{asked:?}");
}

#[derive(serde::Deserialize)]
struct PlannerAnswerFixture {
    #[allow(dead_code)]
    about: String,
    request: PlanRequest,
    #[serde(default)]
    strip: Option<Value>,
    #[serde(default)]
    answer: Option<Value>,
    /// What the merge is expected to report (a permission dropped, say). Empty when nothing.
    #[serde(default)]
    problems: Vec<String>,
}

#[test]
fn every_recorded_planner_answer_becomes_a_clean_draft() {
    let lib = StageLibrary::load(&repo_stages_dir()).unwrap();
    let tools = known_tools();
    let mut files: Vec<PathBuf> = std::fs::read_dir(compat_dir().join("planner-answers")).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
    files.sort();
    assert!(files.len() >= 8, "found {} planner-answer fixtures", files.len());
    for f in files {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let fx: PlannerAnswerFixture = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let strip = fx.strip.as_ref().map(Value::to_string);
        let answer = fx.answer.as_ref().map(Value::to_string);
        let (draft, merge_problems) = draft_from_answers(&fx.request, strip.as_deref(), answer.as_deref(), Some(&lib)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(merge_problems, fx.problems, "{name}: what the merge reports");
        assert_eq!(draft.yaml["id"], fx.request.id, "{name}: the requested id wins");
        let findings = draft.check(&tools);
        assert!(findings.is_empty(), "{name}: a drafted recipe arrives with nothing to fix, got {findings:#?}");
        // The draft must also be a recipe the bundle loader accepts, not only one the checker passes.
        let tmp = std::env::temp_dir().join(format!("stillroom-compat-{}", uuid::Uuid::now_v7()));
        draft.write(&tmp, &tools).unwrap_or_else(|e| panic!("{name}: {e}"));
        RecipeBundle::load(&tmp, RecipeSource::User).unwrap_or_else(|e| panic!("{name}: {e}"));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
