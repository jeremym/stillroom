//! A recipe being edited: `recipe.yaml` as a JSON value plus its prompt and schema files.
//! Read from a bundle directory, checked for problems in plain words, written back out.

use crate::migrate::{Finding, FixId};
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
    #[error("{0}")]
    Migrate(#[from] crate::migrate::MigrateError),
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
    /// Read `root/recipe.yaml` (brought up to the current format) and every text file under
    /// `prompts/` and `schemas/`.
    pub fn read(root: &Path) -> Result<Self, DraftError> {
        let yaml_path = root.join("recipe.yaml");
        let text = std::fs::read_to_string(&yaml_path).map_err(io(&yaml_path))?;
        let yaml: Value = serde_yaml::from_str(&text)?;
        let yaml = crate::migrate::migrate(yaml)?.yaml;
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

    /// Everything the checker has to say, in words a recipe author can act on: Problems (the
    /// draft would not load or run; a save is refused) and Notes (it works, but is out of date
    /// in a way the app can repair; each carries a fix id for `Draft::fix`). No findings means
    /// it loads, every reference points at something that exists, and nothing depends on itself.
    pub fn check(&self, known_tools: &[String]) -> Vec<Finding> {
        let mut findings: Vec<Finding> = self.problem_texts(known_tools).into_iter().map(Finding::problem).collect();
        if format_problems(&self.yaml).is_empty() {
            findings.extend(crate::migrate::notes_for(&self.yaml));
        }
        findings
    }

    /// Only the Problems, as text: what stops a save. Notes are left out.
    pub fn problems(&self, known_tools: &[String]) -> Vec<String> {
        self.check(known_tools).into_iter().filter(Finding::is_problem).map(|f| f.text).collect()
    }

    /// The draft with the named fixes applied (see `migrate::DEPRECATIONS`). An unknown fix id
    /// is an error in words; a fix that finds nothing to change is not.
    pub fn fix(&self, fixes: &[FixId]) -> Result<Draft, String> {
        let mut out = self.clone();
        for f in fixes {
            crate::migrate::apply_fix(&mut out.yaml, f)?;
        }
        Ok(out)
    }

    fn problem_texts(&self, known_tools: &[String]) -> Vec<String> {
        let mut problems = format_problems(&self.yaml);
        if !problems.is_empty() {
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
        self.check_stages(&recipe, &top, &mut problems);
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

    /// The strip must be honest: every stage names things that exist, nothing sits in two
    /// stages, and a step reads only from its own stage or earlier ones.
    fn check_stages(&self, recipe: &Recipe, top: &[&str], problems: &mut Vec<String>) {
        use std::collections::HashMap;
        let mut seen_stage = HashSet::new();
        let mut place: HashMap<(&str, &str), (usize, &str)> = HashMap::new(); // (kind, name) -> (stage index, stage name)
        for (i, st) in recipe.stages.iter().enumerate() {
            if !seen_stage.insert(st.id.as_str()) {
                problems.push(format!("Two stages share the id “{}”.", st.id));
            }
            for (kind, names, exists) in [
                ("input", &st.inputs, recipe.inputs.keys().map(String::as_str).collect::<Vec<_>>()),
                ("step", &st.steps, top.to_vec()),
                ("output", &st.outputs, recipe.outputs.keys().map(String::as_str).collect::<Vec<_>>()),
            ] {
                for n in names {
                    if !exists.contains(&n.as_str()) {
                        problems.push(format!("Stage “{}” lists {kind} “{n}”, which does not exist.", st.name));
                    } else if let Some((_, other)) = place.insert((kind, n.as_str()), (i, st.name.as_str())) {
                        problems.push(format!("The {kind} “{n}” is in two stages: “{other}” and “{}”.", st.name));
                    }
                }
            }
        }
        // Keep tools write to the step's own stage's keep, so that stage must declare one; a read
        // may name an earlier kept stage instead.
        let kept: HashSet<&str> = recipe.stages.iter().filter(|s| s.keep.is_some()).map(|s| s.id.as_str()).collect();
        // A stage that writes a keep without declaring one is a Note with a Fix (the registry's
        // `declare_keep`), not a Problem; its steps are not reported again here.
        let undeclared: HashSet<String> = crate::migrate::notes_for(&self.yaml).into_iter().filter_map(|n| n.fix).filter_map(|f| f.strip_prefix("declare_keep:").map(String::from)).collect();
        let stage_name_of = |step_id: &str| place.get(&("step", step_id)).map(|p| p.1.to_string());
        let stage_id_of = |step_id: &str| place.get(&("step", step_id)).and_then(|p| recipe.stages.get(p.0)).map(|s| s.id.as_str());
        let uses_keep = |tool: &str| tool.starts_with("keep.");
        for step in &recipe.steps {
            let mut tools: Vec<(String, Option<String>)> = Vec::new();
            let mut collect = |s: &Step| match s {
                Step::Tool(t) if uses_keep(&t.tool) => tools.push((t.tool.clone(), t.inputs.get("stage").and_then(Value::as_str).map(String::from))),
                Step::Agent(a) => tools.extend(a.allowed_tools.iter().filter(|t| uses_keep(t)).map(|t| (t.clone(), None))),
                _ => {}
            };
            collect(step);
            if let Step::Map(m) = step {
                m.steps.iter().for_each(&mut collect);
            }
            for (tool, named) in tools {
                let own_kept = stage_id_of(step.id()).is_some_and(|id| kept.contains(id));
                let own_undeclared = stage_id_of(step.id()).is_some_and(|id| undeclared.contains(id));
                let reads_other = matches!(tool.as_str(), "keep.get" | "keep.query" | "keep.list" | "keep.search") && named.as_deref().is_some_and(|n| kept.contains(n));
                if !own_kept && !reads_other && !(own_undeclared && named.is_none()) {
                    let where_ = stage_name_of(step.id()).map(|n| format!("its stage “{n}”")).unwrap_or_else(|| "its stage".into());
                    problems.push(match named.as_deref() {
                        Some(n) if !kept.contains(n) => format!("“{}” reads the keep of stage “{n}”, which has no run keep.", label(step)),
                        _ => format!("“{}” uses {tool}, but {where_} has no run keep. Give the stage keep: {{ refresh: when_sources_change }} (or always, or manual).", label(step)),
                    });
                }
                // A keep is filled by its stage, so a step can only read the keep of its own
                // stage or an earlier one. Later, and it would read a keep not built yet.
                let mine = place.get(&("step", step.id())).map(|p| p.0);
                let theirs = named.as_deref().and_then(|n| recipe.stages.iter().position(|s| s.id == n));
                if let (Some(mine), Some(theirs)) = (mine, theirs)
                    && theirs > mine
                {
                    problems.push(format!(
                        "“{}” (stage “{}”) reads the keep of stage “{}”, which comes later. Stages run in order, so that keep is not filled yet.",
                        label(step), recipe.stages[mine].name, recipe.stages[theirs].name
                    ));
                }
            }
        }
        if recipe.stages.is_empty() {
            return;
        }
        // Reading order: with unassigned inputs in the first stage and unassigned steps last.
        let last = recipe.stages.len();
        let stage_of_input = |n: &str| place.get(&("input", n)).map(|p| p.0).unwrap_or(0);
        let stage_of_step = |n: &str| place.get(&("step", n)).map(|p| p.0).unwrap_or(last);
        let name_of = |i: usize| recipe.stages.get(i).map(|s| s.name.clone()).unwrap_or_else(|| "Make".into());
        for step in &recipe.steps {
            let mine = stage_of_step(step.id());
            let mut refs = Vec::new();
            collect_refs(&Value::Object(step.inputs().clone().into_iter().collect()), &mut refs);
            if let Step::Map(m) = step {
                refs.push(m.over.clone());
                for sub in &m.steps {
                    collect_refs(&Value::Object(sub.inputs().clone().into_iter().collect()), &mut refs);
                }
            }
            for r in refs {
                let later = match parse_ref(&r) {
                    Some(Ok(Ref::Input { name, .. })) => Some((stage_of_input(&name), format!("the input “{name}”"))),
                    Some(Ok(Ref::Step { step: s, .. })) if top.contains(&s.as_str()) => Some((stage_of_step(&s), format!("the step “{s}”"))),
                    _ => None,
                };
                if let Some((theirs, what)) = later {
                    if theirs > mine {
                        problems.push(format!(
                            "“{}” (stage “{}”) reads {what}, which comes later, in stage “{}”. Stages run in order.",
                            label(step), name_of(mine), name_of(theirs)
                        ));
                    }
                }
            }
        }
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
    /// draft no longer has. Refuses a draft with Problems; Notes do not stop a save.
    pub fn write(&self, dir: &Path, known_tools: &[String]) -> Result<(), DraftError> {
        let problems = self.problems(known_tools);
        if !problems.is_empty() {
            return Err(DraftError::Invalid(problems));
        }
        std::fs::create_dir_all(dir).map_err(io(dir))?;
        let mut stamped = self.yaml.clone();
        if let Some(o) = stamped.as_object_mut() {
            o.insert("schemaVersion".into(), Value::String(crate::migrate::CURRENT_SCHEMA_VERSION.into()));
        }
        let yaml = serde_yaml::to_string(&stamped)?;
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

/// Schema problems in words. A step that fails the format is checked again against the schema
/// for its own kind, so the message names the field instead of saying "not valid under any of
/// the schemas" (what a `oneOf` over the step kinds reports).
fn format_problems(yaml: &Value) -> Vec<String> {
    let schema: Value = match serde_json::from_str(crate::recipe::RECIPE_SCHEMA) {
        Ok(s) => s,
        Err(e) => return vec![format!("Format: {e}")],
    };
    let validator = match jsonschema::validator_for(&schema) {
        Ok(v) => v,
        Err(e) => return vec![format!("Format: {e}")],
    };
    let mut out = Vec::new();
    for e in validator.iter_errors(yaml) {
        let path = e.instance_path.to_string();
        let is_step = {
            let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
            !parts.is_empty() && parts.len() % 2 == 0 && parts.chunks(2).all(|c| c[0] == "steps" && c[1].parse::<usize>().is_ok())
        };
        if is_step && matches!(e.kind, jsonschema::error::ValidationErrorKind::OneOfNotValid | jsonschema::error::ValidationErrorKind::OneOfMultipleValid) {
            if let Some(step) = yaml.pointer(&path) {
                out.extend(explain_step(&schema, step, &path));
                continue;
            }
        }
        out.push(format!("Format: {} at {}", e, path));
    }
    out
}

fn explain_step(schema: &Value, step: &Value, path: &str) -> Vec<String> {
    let who = step
        .get("name")
        .or_else(|| step.get("id"))
        .and_then(Value::as_str)
        .map(|s| format!("“{s}”"))
        .unwrap_or_else(|| format!("the step at {path}"));
    let kinds = ["tool", "model", "agent", "recipe", "branch", "map", "review"];
    let kind = step.get("kind").and_then(Value::as_str).unwrap_or("");
    if !kinds.contains(&kind) {
        return vec![format!(
            "Format: {who} has {}. Kinds are tool, model, agent, map, review.",
            if kind.is_empty() { "no kind".to_string() } else { format!("an unknown kind “{kind}”") }
        )];
    }
    let sub = serde_json::json!({ "$ref": format!("#/$defs/{kind}Step"), "$defs": schema.get("$defs").cloned().unwrap_or(Value::Null) });
    let Ok(validator) = jsonschema::validator_for(&sub) else { return vec![format!("Format: {who} is not a valid {kind} step.")] };
    let mut out: Vec<String> = validator
        .iter_errors(step)
        .map(|e| {
            let at = e.instance_path.to_string();
            match &e.kind {
                jsonschema::error::ValidationErrorKind::Pattern { .. } if at == "/id" => {
                    format!("{who}: a step id is a letter followed by letters, digits, or underscores (no hyphens or spaces), like read_file.")
                }
                jsonschema::error::ValidationErrorKind::Required { property } => {
                    format!("{who} is missing {} (a {kind} step needs it).", property.as_str().map(String::from).unwrap_or_else(|| property.to_string()))
                }
                _ => format!("{who}: {} at {}.", e, if at.is_empty() { "the step".to_string() } else { at }),
            }
        })
        .collect();
    if out.is_empty() {
        out.push(format!("Format: {who} is not a valid {kind} step."));
    }
    out
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

    #[test]
    fn stages_are_checked_and_implied() {
        let mut yaml = serde_json::json!({
            "schemaVersion": "0", "id": "x", "name": "X",
            "inputs": { "folder": { "type": "directory" }, "question": { "type": "text" } },
            "modelSlots": { "writer": { "requires": ["text"] } },
            "steps": [
                { "id": "list", "kind": "tool", "tool": "fs.list", "inputs": { "path": "${inputs.folder}" } },
                { "id": "answer", "kind": "model", "slot": "writer", "prompt": "prompts/a.md", "inputs": { "q": "${inputs.question}", "files": "${steps.list.output}" } }
            ],
            "outputs": { "answer": { "from": "${steps.answer.output}", "type": "markdown" } },
            "stages": [
                { "id": "docs", "name": "Add documents", "inputs": ["folder"], "steps": ["list"] },
                { "id": "ask", "name": "Ask", "inputs": ["question"], "steps": ["answer"], "outputs": ["answer"] }
            ]
        });
        let files = [("prompts/a.md".to_string(), "Answer.".to_string())].into_iter().collect();
        let d = Draft { yaml: yaml.clone(), files };
        let known = vec!["fs.list".to_string()];
        assert!(d.problems(&known).is_empty(), "{:?}", d.problems(&known));
        let r: Recipe = serde_json::from_value(yaml.clone()).unwrap();
        let stages = r.stages_or_implied();
        assert_eq!(stages.len(), 2);
        // A stage without a why gets one written from what it holds.
        assert_eq!(stages[0].why.as_deref(), Some("Takes folder. Runs list (a program)."));
        assert_eq!(stages[1].why.as_deref(), Some("Takes question. Runs answer (AI). Gives you answer."));
        let mut bare = yaml.clone();
        bare.as_object_mut().unwrap().remove("stages");
        let r: Recipe = serde_json::from_value(bare).unwrap();
        let implied = r.stages_or_implied();
        assert_eq!(implied.len(), 1);
        assert_eq!(implied[0].why.as_deref(), Some("Takes folder and question. Runs list (a program) and answer (AI). Gives you answer."));

        // Reading forward, a missing step, and a double listing are all named.
        yaml["stages"][0]["steps"] = serde_json::json!(["list", "answer", "ghost"]);
        yaml["steps"][0]["inputs"]["q"] = serde_json::json!("${inputs.question}");
        let d = Draft { yaml: yaml.clone(), files: d.files.clone() };
        let text = d.problems(&known).join("\n");
        assert!(text.contains("lists step “ghost”, which does not exist"), "{text}");
        assert!(text.contains("The step “answer” is in two stages"), "{text}");
        assert!(text.contains("reads the input “question”, which comes later, in stage “Ask”"), "{text}");

        // Keep tools need a kept stage.
        let mut bad = yaml.clone();
        bad["stages"] = serde_json::json!([
            { "id": "docs", "name": "Add documents", "inputs": ["folder"], "steps": ["list"] },
            { "id": "ask", "name": "Ask", "inputs": ["question"], "steps": ["answer"], "outputs": ["answer"] }
        ]);
        bad["steps"][0] = serde_json::json!({ "id": "list", "kind": "tool", "tool": "keep.put", "inputs": { "data": "${inputs.folder}" } });
        let d = Draft { yaml: bad.clone(), files: d.files.clone() };
        let findings = d.check(&vec!["keep.put".to_string(), "keep.query".to_string()]);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].severity, crate::migrate::Severity::Note);
        assert_eq!(findings[0].text, "Stage “Add documents” writes a keep but does not say so.");
        assert_eq!(findings[0].fix.as_deref(), Some("declare_keep:docs"));
        assert_eq!(findings[0].fix_label.as_deref(), Some("Declare the keep"));
        assert!(d.problems(&vec!["keep.put".to_string()]).is_empty(), "a Note is not a Problem");
        let fixed = d.fix(&["declare_keep:docs".to_string()]).unwrap();
        assert_eq!(fixed.yaml["stages"][0]["keep"]["refresh"], "always");
        assert!(fixed.check(&vec!["keep.put".to_string()]).is_empty());
        assert_eq!(d.fix(&["nonsense".to_string()]).unwrap_err(), "There is no fix called “nonsense”.");
        // A reader with no keep anywhere near it is still a Problem: nothing would ever fill it.
        bad["steps"][0] = serde_json::json!({ "id": "list", "kind": "tool", "tool": "keep.query", "inputs": {} });
        let d = Draft { yaml: bad.clone(), files: d.files.clone() };
        let text = d.problems(&vec!["keep.put".to_string(), "keep.query".to_string()]).join("\n");
        assert!(text.contains("uses keep.query, but its stage “Add documents” has no run keep"), "{text}");
        bad["steps"][0] = serde_json::json!({ "id": "list", "kind": "tool", "tool": "keep.put", "inputs": { "data": "${inputs.folder}" } });
        bad["stages"][0]["keep"] = serde_json::json!({ "refresh": "always" });
        let d = Draft { yaml: bad.clone(), files: d.files.clone() };
        assert!(d.problems(&vec!["keep.put".to_string()]).is_empty(), "{:?}", d.problems(&vec!["keep.put".to_string()]));
        bad["steps"][1]["inputs"]["files"] = serde_json::json!("${steps.list.output}");
        bad["steps"][1] = serde_json::json!({ "id": "answer", "kind": "tool", "tool": "keep.query", "inputs": { "stage": "nope" } });
        let d = Draft { yaml: bad.clone(), files: d.files.clone() };
        let text = d.problems(&vec!["keep.put".to_string(), "keep.query".to_string()]).join("\n");
        assert!(text.contains("reads the keep of stage “nope”, which has no run keep"), "{text}");
        // Reading the keep of a later stage is refused: that keep is not filled yet.
        bad["stages"][1]["keep"] = serde_json::json!({ "refresh": "always" });
        bad["steps"][0] = serde_json::json!({ "id": "list", "kind": "tool", "tool": "keep.query", "inputs": { "stage": "ask" } });
        bad["steps"][1] = serde_json::json!({ "id": "answer", "kind": "tool", "tool": "keep.put", "inputs": { "data": "${inputs.question}" } });
        let d = Draft { yaml: bad.clone(), files: d.files.clone() };
        let text = d.problems(&vec!["keep.put".to_string(), "keep.query".to_string()]).join("\n");
        assert!(text.contains("“list” (stage “Add documents”) reads the keep of stage “Ask”, which comes later"), "{text}");
        // Reading an earlier stage's keep is fine.
        bad["steps"][0] = serde_json::json!({ "id": "list", "kind": "tool", "tool": "keep.put", "inputs": { "data": "${inputs.folder}" } });
        bad["steps"][1] = serde_json::json!({ "id": "answer", "kind": "tool", "tool": "keep.query", "inputs": { "stage": "docs" } });
        let d = Draft { yaml: bad.clone(), files: d.files.clone() };
        assert!(d.problems(&vec!["keep.put".to_string(), "keep.query".to_string()]).is_empty(), "{:?}", d.problems(&vec!["keep.put".to_string(), "keep.query".to_string()]));

        // No stages: one implied stage holds everything. Partial stages: the rest lands in Make.
        let mut plain = Draft::read(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../recipes/summarize-file")).unwrap().yaml;
        plain.as_object_mut().unwrap().remove("stages");
        let r: Recipe = serde_json::from_value(plain).unwrap();
        let implied = r.stages_or_implied();
        assert_eq!(implied.len(), 1);
        assert_eq!(implied[0].id, crate::recipe::IMPLIED_STAGE);
        assert_eq!(implied[0].steps.len(), r.steps.len());
        yaml["stages"] = serde_json::json!([{ "id": "docs", "name": "Add documents", "steps": ["list"] }]);
        let r: Recipe = serde_json::from_value(yaml).unwrap();
        let st = r.stages_or_implied();
        assert_eq!(st.len(), 2);
        assert_eq!(st[0].inputs, vec!["folder", "question"]);
        assert_eq!(st[1].steps, vec!["answer"]);
        assert_eq!(st[1].outputs, vec!["answer"]);
    }

    #[test]
    fn a_step_that_fails_the_format_is_explained_by_field() {
        let yaml = serde_json::json!({
            "schemaVersion": "0", "id": "x", "name": "X",
            "inputs": { "file": { "type": "file" } },
            "modelSlots": { "writer": { "requires": ["text"] } },
            "steps": [
                { "id": "read-file", "kind": "tool", "tool": "file.read", "inputs": { "path": "${inputs.file}" } },
                { "id": "sum", "kind": "model", "slot": "writer", "inputs": { "text": "${steps.read-file.output.text}" } },
                { "id": "odd", "kind": "sparkle" }
            ],
            "outputs": { "summary": { "from": "${steps.sum.output}", "type": "markdown" } }
        });
        let d = Draft { yaml, files: Default::default() };
        let p = d.problems(&["file.read".into()]);
        let joined = p.join("\n");
        assert!(joined.contains("“read-file”: a step id is a letter followed by"), "{joined}");
        assert!(joined.contains("“sum” is missing prompt (a model step needs it)"), "{joined}");
        assert!(joined.contains("“odd” has an unknown kind “sparkle”"), "{joined}");
        assert!(!joined.contains("oneOf"), "{joined}");
    }

    fn repo_recipes() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../recipes")
    }

    fn tools() -> Vec<String> {
        crate::tools::ToolRegistry::builtin().ids().into_iter().map(String::from).collect()
    }

    /// The recipe the app's "New recipe › Blank" makes, byte for byte as `src/recipes/new.ts` builds it.
    /// If this stops passing, the button makes something the checker refuses.
    #[test]
    fn blank_recipe_from_the_app_passes_the_checker() {
        let yaml = serde_json::json!({
            "schemaVersion": "0",
            "id": "weekly-report",
            "name": "Weekly report",
            "version": "0.1.0",
            "icon": "notes",
            "description": "A new recipe. Open its recipe card to add steps.",
            "inputs": { "source": { "type": "file", "label": "Source file", "description": "The file this recipe starts from." } },
            "permissions": { "filesystem": { "read": ["input"] } },
            "steps": [ { "id": "read", "kind": "tool", "name": "Read the file", "tool": "file.read", "inputs": { "path": "${inputs.source}" } } ],
            "outputs": { "text": { "from": "${steps.read.output.text}", "type": "text", "label": "The file's text" } }
        });
        let draft = Draft { yaml, files: BTreeMap::new() };
        let problems = draft.problems(&tools());
        assert!(problems.is_empty(), "blank recipe should be valid, got: {problems:?}");
    }

    #[test]
    fn builtin_recipes_read_clean() {
        for name in ["summarize-file", "learning-kit", "research-brief", "daily-news-log", "ask-your-documents", "batch-briefing"] {
            let d = Draft::read(&repo_recipes().join(name)).unwrap();
            assert_eq!(d.problems(&tools()), Vec::<String>::new(), "{name}");
        }
        // Inbox Triage is a sample now, not a built-in, but it still has to read clean.
        let d = Draft::read(&repo_recipes().join("../samples/inbox-triage")).unwrap();
        assert_eq!(d.problems(&tools()), Vec::<String>::new(), "inbox-triage");
        assert!(d.files.contains_key("prompts/classify.md"));
        assert!(d.files.contains_key("schemas/draft.json"));
    }

    #[test]
    fn reports_problems_in_words() {
        let mut d = Draft::read(&repo_recipes().join("../samples/inbox-triage")).unwrap();
        let steps = d.yaml["steps"].as_array_mut().unwrap();
        // table reads from a step that is not there; classify (inside the loop) uses the wrong item name.
        steps[2]["inputs"]["items"] = Value::String("${steps.nope.output}".into());
        steps[1]["steps"][0]["inputs"]["path"] = Value::String("${message.path}".into());
        steps[1]["steps"][1]["prompt"] = Value::String("prompts/missing.md".into());
        d.files.remove("schemas/draft.json");
        let problems = d.problems(&tools());
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
        let text = d.problems(&tools()).join("\n");
        assert!(text.contains("in a circle: read → summarize → read"), "{text}");
        assert!(text.contains("tool this app does not have: magic.wand"), "{text}");

        d.yaml["steps"][0]["kind"] = Value::String("spell".into());
        let text = d.problems(&tools()).join("\n");
        assert!(text.contains("has an unknown kind “spell”"), "{text}");
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
