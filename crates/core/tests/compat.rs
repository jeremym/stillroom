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

/// The first-run example goals (`FIRST-RUN-PLAN.md` §2) and the honesty case, each recorded from a
/// live draft through the planner harness (`STILLROOM_PLAN_ANSWERS_OUT`). A prompt or library
/// change that stops one of them drafting clean fails here, not on a stranger's first run.
const FIRST_RUN_FIXTURES: &[&str] = &[
    "first-run-new-on-my-sites",
    "first-run-sort-saved-email",
    "first-run-ask-my-pdfs",
    "first-run-document-to-notes",
    "first-run-cited-brief",
    "first-run-news-to-notion",
];

fn first_run_draft(name: &str, lib: &StageLibrary) -> (PlannerAnswerFixture, Draft) {
    let path = compat_dir().join("planner-answers").join(format!("{name}.json"));
    let fx: PlannerAnswerFixture = serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"))).unwrap_or_else(|e| panic!("{name}: {e}"));
    let strip = fx.strip.as_ref().map(Value::to_string);
    let answer = fx.answer.as_ref().map(Value::to_string);
    let (draft, _) = draft_from_answers(&fx.request, strip.as_deref(), answer.as_deref(), Some(lib)).unwrap_or_else(|e| panic!("{name}: {e}"));
    (fx, draft)
}

/// Jeremy's learn-anything goal: two source stages make links and the folder either-or, and the
/// study cards take their material from the brief instead of asking for a paste.
#[test]
fn links_or_a_folder_become_either_or_and_cards_are_fed_from_the_brief() {
    let lib = StageLibrary::load(&repo_stages_dir()).unwrap();
    let (_, draft) = first_run_draft("two-call-links-or-folder-brief-cards", &lib);
    let y = &draft.yaml;
    assert_eq!(y["inputs"]["urls"]["required"], Value::Bool(false), "links are optional on their own");
    assert_eq!(y["inputs"]["folder"]["required"], Value::Bool(false), "the folder is optional on its own");
    assert!(y["inputs"].get("material").is_none(), "study_cards asks for no paste: {}", y["inputs"]);
    assert_eq!(y["inputGroups"], serde_json::json!([{ "anyOf": ["urls", "folder"], "label": "Give at least one of these: Pages to read, Your documents." }]));
    let cards = y["steps"].as_array().unwrap().iter().find(|s| s["id"] == "cards_write").unwrap();
    assert_eq!(cards["inputs"]["material"], "${steps.brief_write.output}", "the cards read the brief");
    let quiz = y["steps"].as_array().unwrap().iter().find(|s| s["id"] == "cards_quiz").unwrap();
    assert_eq!(quiz["inputs"]["material"], "${steps.brief_write.output}");
    let stage = y["stages"].as_array().unwrap().iter().find(|s| s["id"] == "study_cards").unwrap();
    assert!(stage.get("inputs").is_none(), "the cards stage lists no input of its own: {stage}");
    assert_eq!(y["inputs"]["focus"]["required"], Value::Null, "the focus is still asked for");
    // Saved, the recipe carries the floor either-or needs.
    let mut stamped = y.clone();
    migrate::stamp_min_version(&mut stamped);
    assert_eq!(stamped["runtime"]["minVersion"], "0.4.0");
}

/// Every tool a draft's steps name, loops included.
fn tools_named(steps: &Value, out: &mut Vec<String>) {
    for s in steps.as_array().into_iter().flatten() {
        if let Some(t) = s.get("tool").and_then(Value::as_str) {
            out.push(t.to_string());
            // A tool step's parameters live under `inputs`; a model that puts them under `params`
            // drafts a recipe that checks clean and then runs the tool with nothing.
            let inputs = s.get("inputs").and_then(Value::as_object).map(|o| o.len()).unwrap_or(0);
            assert!(inputs > 0, "tool step {:?} ({t}) has no inputs: {s}", s["id"]);
        }
        for t in s.get("allowedTools").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            out.push(t.to_string());
        }
        if let Some(inner) = s.get("steps") {
            tools_named(inner, out);
        }
    }
}

#[test]
fn the_first_run_examples_are_recorded_and_draft_as_a_strip() {
    let lib = StageLibrary::load(&repo_stages_dir()).unwrap();
    let tools = known_tools();
    for name in FIRST_RUN_FIXTURES {
        let (fx, draft) = first_run_draft(name, &lib);
        assert!(fx.about.contains("Recorded live"), "{name}: a first-run fixture is recorded, not hand-made: {}", fx.about);
        let n = draft.yaml["stages"].as_array().map(Vec::len).unwrap_or(0);
        assert!((2..=6).contains(&n), "{name}: two to six stages, got {n}");
        assert!(draft.check(&tools).is_empty(), "{name}: {:#?}", draft.check(&tools));
        let mut named = Vec::new();
        tools_named(&draft.yaml["steps"], &mut named);
        let unknown: Vec<&String> = named.iter().filter(|t| !tools.contains(t)).collect();
        assert!(unknown.is_empty(), "{name}: no step names a tool that does not exist: {unknown:?}");
    }
}

/// The honesty case: the goal asks for a schedule and a post to Notion, neither of which any tool
/// does. The recipe must not invent a step for them; its last stage writes into a folder the user
/// chooses and says so.
#[test]
fn the_notion_goal_delivers_to_a_folder_and_names_no_unknown_tool() {
    let lib = StageLibrary::load(&repo_stages_dir()).unwrap();
    let (fx, draft) = first_run_draft("first-run-news-to-notion", &lib);
    assert!(fx.request.goal.contains("Notion"), "{}", fx.request.goal);
    let stages = draft.yaml["stages"].as_array().expect("stages");
    let last = stages.last().expect("a last stage");
    let why = last["why"].as_str().unwrap_or("").to_ascii_lowercase();
    assert!(why.contains("folder"), "the last stage says where the report really goes: {why}");
    let last_inputs: Vec<&str> = last["inputs"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let folder = last_inputs.iter().find(|i| draft.yaml["inputs"][**i]["type"] == "directory");
    assert!(folder.is_some(), "the last stage asks for a folder (a directory input): {last_inputs:?}");
    let mut named = Vec::new();
    tools_named(&draft.yaml["steps"], &mut named);
    let known = known_tools();
    let unknown: Vec<&String> = named.iter().filter(|t| !known.contains(t)).collect();
    assert!(unknown.is_empty(), "no step names a tool that does not exist: {unknown:?}");
    assert!(named.iter().any(|t| t == "fs.write"), "the report is written as a file: {named:?}");
    let text = serde_json::to_string(&draft.yaml).unwrap().to_ascii_lowercase();
    assert!(!text.contains("\"notion."), "no invented Notion tool: {text}");
}
