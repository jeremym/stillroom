//! A recipe being edited: `recipe.yaml` as a JSON value plus its prompt and schema files.
//! Read from a bundle directory, checked for problems in plain words, written back out.

use crate::propagate::dependencies;
use crate::recipe::{Recipe, Step};
use crate::refs::{parse_ref, Ref};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Draft {
    /// `recipe.yaml` parsed to a JSON value, key order preserved.
    pub yaml: Value,
    /// Text files inside the bundle (prompts and schemas), by bundle-relative path.
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DraftError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("recipe.yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("path {0} escapes the recipe directory")]
    Escape(String),
    #[error("the draft has problems:\n{}", .0.join("\n"))]
    Invalid(Vec<String>),
}

const FILE_DIRS: [&str; 2] = ["prompts", "schemas"];

fn safe_join(root: &Path, rel: &str) -> Result<PathBuf, DraftError> {
    let p = Path::new(rel);
    if p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir | Component::Prefix(_))) {
        return Err(DraftError::Escape(rel.to_string()));
    }
    Ok(root.join(p))
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> DraftError + '_ {
    move |source| DraftError::Io { path: path.to_path_buf(), source }
}

fn walk(dir: &Path, rel: &str, out: &mut BTreeMap<String, String>) -> Result<(), DraftError> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Ok(()) };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let child_rel = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        let path = e.path();
        if path.is_dir() {
            walk(&path, &child_rel, out)?;
        } else if let Ok(text) = std::fs::read_to_string(&path) {
            out.insert(child_rel, text);
        }
    }
    Ok(())
}

impl Draft {
    /// Read `root/recipe.yaml` and every text file under `prompts/` and `schemas/`.
    pub fn read(root: &Path) -> Result<Self, DraftError> {
        let yaml_path = root.join("recipe.yaml");
        let text = std::fs::read_to_string(&yaml_path).map_err(io(&yaml_path))?;
        let yaml: Value = serde_yaml::from_str(&text)?;
        let mut files = BTreeMap::new();
        for d in FILE_DIRS {
            walk(&root.join(d), d, &mut files)?;
        }
        Ok(Self { yaml, files })
    }

    /// The recipe id the draft declares, if any.
    pub fn id(&self) -> Option<&str> {
        self.yaml.get("id").and_then(Value::as_str)
    }

    /// Everything wrong with the draft, in words a recipe author can act on. Empty means it
    /// loads, every reference points at something that exists, and nothing depends on itself.
    pub fn check(&self, known_tools: &[String]) -> Vec<String> {
        let mut problems = Vec::new();
        if let Err(e) = Recipe::validate_value(&self.yaml) {
            let msg = e.to_string();
            problems.extend(msg.lines().filter(|l| l.starts_with("- ")).map(|l| format!("Format: {}", &l[2..])));
            if problems.is_empty() {
                problems.push(format!("Format: {msg}"));
            }
            return problems;
        }
        let recipe: Recipe = match serde_json::from_value(self.yaml.clone()) {
            Ok(r) => r,
            Err(e) => return vec![format!("Format: {e}")],
        };
        let top: Vec<&str> = recipe.steps.iter().map(Step::id).collect();
        let mut seen = HashSet::new();
        for id in &top {
            if !seen.insert(*id) {
                problems.push(format!("Two steps share the id “{id}”."));
            }
        }
        if recipe.steps.is_empty() {
            problems.push("The recipe has no steps.".into());
        }
        for step in &recipe.steps {
            let others: Vec<&str> = top.iter().copied().filter(|s| *s != step.id()).collect();
            self.check_step(&recipe, step, &others, None, known_tools, &mut problems);
            if let Step::Map(m) = step {
                let siblings: Vec<&str> = m.steps.iter().map(Step::id).collect();
                let mut seen = HashSet::new();
                for sub in &m.steps {
                    if !seen.insert(sub.id()) {
                        problems.push(format!("Two steps inside “{}” share the id “{}”.", label(step), sub.id()));
                    }
                    let mut scope: Vec<&str> = others.clone();
                    scope.extend(siblings.iter().copied().filter(|s| *s != sub.id()));
                    self.check_step(&recipe, sub, &scope, Some(m.item_name.as_str()), known_tools, &mut problems);
                }
                if let Some(cycle) = find_cycle(&m.steps) {
                    problems.push(format!("Inside “{}”, these steps feed each other in a circle: {}.", label(step), cycle.join(" → ")));
                }
            }
        }
        if let Some(cycle) = find_cycle(&recipe.steps) {
            problems.push(format!("These steps feed each other in a circle: {}.", cycle.join(" → ")));
        }
        for (name, out) in &recipe.outputs {
            match parse_ref(&out.from) {
                Some(Ok(Ref::Step { step, .. })) if top.contains(&step.as_str()) => {}
                Some(Ok(Ref::Input { name: input, .. })) if recipe.inputs.contains_key(&input) => {}
                Some(Ok(Ref::Step { step, .. })) => problems.push(format!("Result “{name}” comes from a step that does not exist: {step}.")),
                Some(Ok(Ref::Input { name: input, .. })) => problems.push(format!("Result “{name}” comes from an input that does not exist: {input}.")),
                _ => problems.push(format!("Result “{name}” needs to come from a step, like ${{steps.<id>.output}}.")),
            }
        }
        if recipe.outputs.is_empty() {
            problems.push("The recipe has no results. Add at least one.".into());
        }
        problems
    }

    fn check_step(
        &self, recipe: &Recipe, step: &Step, steps_in_scope: &[&str], item: Option<&str>, known_tools: &[String], problems: &mut Vec<String>,
    ) {
        let who = label(step);
        let mut refs = Vec::new();
        collect_refs(&Value::Object(step.inputs().clone().into_iter().collect()), &mut refs);
        if let Some(w) = step.when() {
            refs.push(w.to_string());
        }
        match step {
            Step::Map(m) => refs.push(m.over.clone()),
            Step::Review(r) => refs.push(r.items.clone()),
            _ => {}
        }
        for r in refs {
            match parse_ref(&r) {
                None => {}
                Some(Err(e)) => problems.push(format!("“{who}”: {e}.")),
                Some(Ok(Ref::Input { name, .. })) => {
                    if !recipe.inputs.contains_key(&name) {
                        problems.push(format!("“{who}” reads an input that does not exist: {name}."));
                    }
                }
                Some(Ok(Ref::Step { step: s, .. })) => {
                    if s == step.id() {
                        problems.push(format!("“{who}” reads from itself."));
                    } else if !steps_in_scope.contains(&s.as_str()) {
                        problems.push(format!("“{who}” reads from a step it cannot see: {s}."));
                    }
                }
                Some(Ok(Ref::Var { name, .. })) => {
                    if item != Some(name.as_str()) {
                        problems.push(match item {
                            Some(i) => format!("“{who}” uses ${{{name}}}, but the loop calls each item “{i}”."),
                            None => format!("“{who}” uses ${{{name}}}, which only works inside a for-each step."),
                        });
                    }
                }
            }
        }
        for d in step.depends_on() {
            if !steps_in_scope.contains(&d.as_str()) {
                problems.push(format!("“{who}” says it runs after a step that does not exist: {d}."));
            }
        }
        let (slot, prompt, schema) = match step {
            Step::Model(m) => (Some(&m.slot), Some(&m.prompt), m.output_schema.as_ref()),
            Step::Agent(a) => (Some(&a.slot), Some(&a.prompt), a.output_schema.as_ref()),
            Step::Tool(t) => {
                if t.tool.is_empty() {
                    problems.push(format!("“{who}” has no tool chosen yet."));
                } else if !known_tools.is_empty() && !known_tools.iter().any(|k| k == &t.tool) {
                    problems.push(format!("“{who}” uses a tool this app does not have: {}.", t.tool));
                }
                (None, None, t.output_schema.as_ref())
            }
            _ => (None, None, None),
        };
        if let Some(slot) = slot {
            if !recipe.model_slots.contains_key(slot) {
                problems.push(format!("“{who}” uses a model slot that is not declared: {slot}."));
            }
        }
        if let Some(p) = prompt {
            match self.files.get(p) {
                None => problems.push(format!("“{who}” needs a prompt file that is missing: {p}.")),
                Some(t) if t.trim().is_empty() => problems.push(format!("“{who}” has no prompt written yet.")),
                _ => {}
            }
        }
        if let Step::Map(m) = step {
            if m.over.is_empty() {
                problems.push(format!("“{who}” has nothing to loop over yet."));
            }
        }
        if let Step::Review(r) = step {
            if r.items.is_empty() {
                problems.push(format!("“{who}” has no items to decide on yet."));
            }
        }
        if let Some(Value::String(p)) = schema {
            match self.files.get(p) {
                None => problems.push(format!("“{who}” names an output schema file that is missing: {p}.")),
                Some(t) => {
                    if let Err(e) = serde_json::from_str::<Value>(t) {
                        problems.push(format!("“{who}”: {p} is not valid JSON ({e})."));
                    }
                }
            }
        }
        if let Step::Agent(a) = step {
            for t in &a.allowed_tools {
                if !known_tools.is_empty() && !known_tools.contains(t) {
                    problems.push(format!("“{who}” may call a tool this app does not have: {t}."));
                }
            }
        }
    }

    /// Write `recipe.yaml` and every file into `dir`, removing prompt and schema files the
    /// draft no longer has. Refuses a draft with problems.
    pub fn write(&self, dir: &Path, known_tools: &[String]) -> Result<(), DraftError> {
        let problems = self.check(known_tools);
        if !problems.is_empty() {
            return Err(DraftError::Invalid(problems));
        }
        std::fs::create_dir_all(dir).map_err(io(dir))?;
        let yaml = serde_yaml::to_string(&self.yaml)?;
        let yaml_path = dir.join("recipe.yaml");
        std::fs::write(&yaml_path, yaml).map_err(io(&yaml_path))?;
        for (rel, text) in &self.files {
            let path = safe_join(dir, rel)?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(io(parent))?;
            }
            std::fs::write(&path, text).map_err(io(&path))?;
        }
        let mut existing = BTreeMap::new();
        for d in FILE_DIRS {
            walk(&dir.join(d), d, &mut existing)?;
        }
        for rel in existing.keys() {
            if !self.files.contains_key(rel) {
                let path = safe_join(dir, rel)?;
                std::fs::remove_file(&path).map_err(io(&path))?;
            }
        }
        Ok(())
    }
}

fn label(step: &Step) -> String {
    step.name().map(String::from).unwrap_or_else(|| step.id().to_string())
}

/// Every string that looks like a reference, anywhere inside `v`.
fn collect_refs(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) if s.starts_with("${") => out.push(s.clone()),
        Value::Array(a) => a.iter().for_each(|x| collect_refs(x, out)),
        Value::Object(m) => m.values().for_each(|x| collect_refs(x, out)),
        _ => {}
    }
}

/// A dependency cycle among `steps` (by id), if there is one.
fn find_cycle(steps: &[Step]) -> Option<Vec<String>> {
    let ids: BTreeSet<&str> = steps.iter().map(Step::id).collect();
    let deps: BTreeMap<&str, Vec<String>> = steps
        .iter()
        .map(|s| (s.id(), dependencies(s).into_iter().filter(|d| ids.contains(d.as_str())).collect()))
        .collect();
    fn visit<'a>(id: &'a str, deps: &BTreeMap<&'a str, Vec<String>>, stack: &mut Vec<String>, done: &mut HashSet<String>) -> Option<Vec<String>> {
        if let Some(pos) = stack.iter().position(|s| s == id) {
            let mut cycle = stack[pos..].to_vec();
            cycle.push(id.to_string());
            return Some(cycle);
        }
        if done.contains(id) {
            return None;
        }
        stack.push(id.to_string());
        for d in deps.get(id).map(Vec::as_slice).unwrap_or(&[]) {
            if let Some(c) = visit(d.as_str(), deps, stack, done) {
                return Some(c);
            }
        }
        stack.pop();
        done.insert(id.to_string());
        None
    }
    let mut done = HashSet::new();
    for id in ids {
        if let Some(c) = visit(id, &deps, &mut Vec::new(), &mut done) {
            return Some(c);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_recipes() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../recipes")
    }

    fn tools() -> Vec<String> {
        crate::tools::ToolRegistry::builtin().ids().into_iter().map(String::from).collect()
    }

    #[test]
    fn builtin_recipes_read_clean() {
        for name in ["summarize-file", "learning-kit", "inbox-triage"] {
            let d = Draft::read(&repo_recipes().join(name)).unwrap();
            assert_eq!(d.check(&tools()), Vec::<String>::new(), "{name}");
        }
        let d = Draft::read(&repo_recipes().join("inbox-triage")).unwrap();
        assert!(d.files.contains_key("prompts/classify.md"));
        assert!(d.files.contains_key("schemas/draft.json"));
    }

    #[test]
    fn reports_problems_in_words() {
        let mut d = Draft::read(&repo_recipes().join("inbox-triage")).unwrap();
        let steps = d.yaml["steps"].as_array_mut().unwrap();
        // table reads from a step that is not there; classify (inside the loop) uses the wrong item name.
        steps[2]["inputs"]["items"] = Value::String("${steps.nope.output}".into());
        steps[1]["steps"][0]["inputs"]["path"] = Value::String("${message.path}".into());
        steps[1]["steps"][1]["prompt"] = Value::String("prompts/missing.md".into());
        d.files.remove("schemas/draft.json");
        let problems = d.check(&tools());
        let text = problems.join("\n");
        assert!(text.contains("“Summarize the triage” reads from a step it cannot see: nope"), "{text}");
        assert!(text.contains("the loop calls each item “email”"), "{text}");
        assert!(text.contains("prompt file that is missing: prompts/missing.md"), "{text}");
        assert!(text.contains("schema file that is missing: schemas/draft.json"), "{text}");
        assert_eq!(problems.len(), 4, "{text}");
    }

    #[test]
    fn reports_cycles_unknown_tools_and_format_errors() {
        let mut d = Draft::read(&repo_recipes().join("summarize-file")).unwrap();
        d.yaml["steps"][0]["inputs"]["path"] = Value::String("${steps.summarize.output}".into());
        d.yaml["steps"][0]["tool"] = Value::String("magic.wand".into());
        let text = d.check(&tools()).join("\n");
        assert!(text.contains("in a circle: read → summarize → read"), "{text}");
        assert!(text.contains("tool this app does not have: magic.wand"), "{text}");

        d.yaml["steps"][0]["kind"] = Value::String("spell".into());
        let text = d.check(&tools()).join("\n");
        assert!(text.starts_with("Format:"), "{text}");
    }

    #[test]
    fn writes_a_bundle_that_loads_and_prunes_stale_files() {
        let tmp = std::env::temp_dir().join(format!("stillroom-draft-{}", uuid::Uuid::now_v7()));
        let mut d = Draft::read(&repo_recipes().join("summarize-file")).unwrap();
        d.yaml["id"] = Value::String("summarize-file-short".into());
        d.yaml["name"] = Value::String("Summarize, short".into());
        d.files.insert("prompts/summarize.md".into(), "Summarize in three bullets:\n\n{{text}}\n".into());
        d.files.insert("prompts/old.md".into(), "stale".into());
        d.write(&tmp, &tools()).unwrap();
        d.files.remove("prompts/old.md");
        d.write(&tmp, &tools()).unwrap();
        assert!(!tmp.join("prompts/old.md").exists(), "stale prompt removed");
        let b = crate::bundle::RecipeBundle::load(&tmp, crate::bundle::RecipeSource::User).unwrap();
        assert_eq!(b.recipe.id, "summarize-file-short");
        assert!(b.prompts["prompts/summarize.md"].starts_with("Summarize in three"));
        let yaml = std::fs::read_to_string(tmp.join("recipe.yaml")).unwrap();
        assert!(yaml.starts_with("schemaVersion:"), "key order kept:\n{yaml}");
        std::fs::remove_dir_all(&tmp).unwrap();

        d.yaml["outputs"] = serde_json::json!({});
        assert!(matches!(d.write(&tmp, &tools()), Err(DraftError::Invalid(_))));
    }
}
