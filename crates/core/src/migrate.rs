//! The go-between for older recipes: `migrate` rewrites a recipe's YAML from the format version
//! it was written in to the current one before anything else reads it, and says in Notes what
//! it changed. The deprecation registry below is the same table read two ways: `migrate` applies
//! its rewrites when it bumps a version; the checker consults it on a current recipe and turns
//! each entry into a Note with a one-click Fix instead of a refusal.

use crate::refs::{input_refs, parse_ref, step_refs, Ref};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// The recipe format version this build writes. A string, not a number: YAML would read a bare
/// `1` as a number, and the schema asks for a string.
pub const CURRENT_SCHEMA_VERSION: &str = "0";

/// Every version `migrate` can start from. The schema's `schemaVersion` enum is checked against
/// this list in the tests.
pub const ACCEPTED_SCHEMA_VERSIONS: &[&str] = &["0"];

/// How serious a checker finding is. A Problem stops a save; a Note does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Problem,
    Note,
}

/// Names a repair `Draft::fix` can apply: `<deprecation id>` or `<deprecation id>:<target>`,
/// where the target is the stage the Note is about.
pub type FixId = String;

/// One thing the checker (or a migration) has to say, in words a recipe author can act on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub text: String,
    pub severity: Severity,
    /// A repair the app can apply, when there is one.
    pub fix: Option<FixId>,
    /// What the button says, e.g. "Declare the keep".
    pub fix_label: Option<String>,
}

impl Finding {
    pub fn problem(text: impl Into<String>) -> Self {
        Self { text: text.into(), severity: Severity::Problem, fix: None, fix_label: None }
    }

    pub fn note(text: impl Into<String>) -> Self {
        Self { text: text.into(), severity: Severity::Note, fix: None, fix_label: None }
    }

    pub fn with_fix(mut self, id: impl Into<String>, label: impl Into<String>) -> Self {
        self.fix = Some(id.into());
        self.fix_label = Some(label.into());
        self
    }

    pub fn is_problem(&self) -> bool {
        self.severity == Severity::Problem
    }
}

/// What a migration has to say: always a Note (the rewrite has already been made).
pub type Note = Finding;

#[derive(Debug, Clone)]
pub struct Migrated {
    pub yaml: Value,
    /// The version the recipe was written in, as found (a missing version reads as "0").
    pub from_version: String,
    pub notes: Vec<Note>,
}

#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    #[error("recipe.yaml is not a recipe (expected a mapping of fields at the top)")]
    NotARecipe,
    #[error("This recipe is written in recipe format “{0}”, which this Stillroom does not read (it reads “{CURRENT_SCHEMA_VERSION}”).")]
    UnknownVersion(String),
}

/// One step of the chain: rewrite a recipe written in `from` so it reads as `to`.
type MigrationStep = fn(&mut Value) -> Vec<Note>;

/// The chain, keyed on the version a step starts from. Empty while "0" is the only version; the
/// first real entry will be `("0", "1", v0_to_v1)`.
const CHAIN: &[(&str, &str, MigrationStep)] = &[];

/// Rewrite a recipe's YAML (as a JSON value) into the current format. Called before schema
/// validation by `Recipe::from_yaml`, `Draft::read`, and the planner, so the rest of the engine
/// only ever sees the current shape.
pub fn migrate(mut yaml: Value) -> Result<Migrated, MigrateError> {
    let obj = yaml.as_object_mut().ok_or(MigrateError::NotARecipe)?;
    let mut notes = Vec::new();
    let from = match obj.get("schemaVersion") {
        None => {
            notes.push(Finding::note(format!("The recipe does not say which recipe format it uses (schemaVersion); read as “{CURRENT_SCHEMA_VERSION}”.")));
            "0".to_string()
        }
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => {
            notes.push(Finding::note(format!("The recipe format (schemaVersion) was written as the number {n}; read as the text “{n}”.")));
            n.to_string()
        }
        Some(other) => return Err(MigrateError::UnknownVersion(other.to_string())),
    };
    obj.insert("schemaVersion".into(), Value::String(from.clone()));
    let mut version = from.clone();
    while version != CURRENT_SCHEMA_VERSION {
        let Some((_, to, step)) = CHAIN.iter().find(|(f, _, _)| *f == version) else {
            return Err(MigrateError::UnknownVersion(version));
        };
        notes.extend(step(&mut yaml));
        version = to.to_string();
        yaml["schemaVersion"] = Value::String(version.clone());
    }
    Ok(Migrated { yaml, from_version: from, notes })
}

// ---------------------------------------------------------------------------------------------
// The version floor: which Stillroom a recipe needs, from the features it uses

/// Features of the recipe format and the Stillroom version that first ran them. `Draft::write`
/// stamps `runtime.minVersion` from this table, so a shared recipe carries its floor without
/// its author knowing; the loader compares the floor to what this build understands.
// Every feature here shipped in 0.3.0 (stages, keeps, and the provenance a ready-made stage carries),
// so a recipe saved today asks for 0.3.0, a version that exists. The next format change adds its own row.
pub const FORMAT_FEATURES: &[(&str, &str)] = &[("stages", "0.3.0"), ("keep", "0.3.0"), ("stage source", "0.3.0"), ("input groups", "0.4.0")];

/// The version floor the recipe's features ask for, and which feature set it, if any.
pub fn required_version(yaml: &Value) -> Option<(&'static str, &'static str)> {
    let stages = yaml.get("stages").and_then(Value::as_array).filter(|a| !a.is_empty());
    let mut uses: Vec<&str> = Vec::new();
    if let Some(stages) = stages {
        uses.push("stages");
        if stages.iter().any(|s| s.get("keep").is_some_and(Value::is_object)) {
            uses.push("keep");
        }
        if stages.iter().any(|s| s.get("source").is_some_and(Value::is_object)) {
            uses.push("stage source");
        }
    }
    if yaml.get("inputGroups").and_then(Value::as_array).is_some_and(|a| !a.is_empty()) {
        uses.push("input groups");
    }
    FORMAT_FEATURES
        .iter()
        .filter(|(f, _)| uses.contains(f))
        .max_by(|a, b| crate::recipe::compare_versions(a.1, b.1))
        .map(|(f, v)| (*f, *v))
}

/// The highest version floor any recipe this build can read may carry: the app's own version,
/// or the newest feature in the table when the build runs ahead of its version number (a
/// pre-release that already has stages is still 0.2.0 by Cargo but reads a 0.3.0 recipe).
pub fn supported_version(app_version: &str) -> String {
    FORMAT_FEATURES
        .iter()
        .map(|(_, v)| *v)
        .chain(std::iter::once(app_version))
        .max_by(|a, b| crate::recipe::compare_versions(a, b))
        .unwrap_or(app_version)
        .to_string()
}

/// Stamp `runtime.minVersion` from the features the recipe uses, never lowering an author's
/// higher floor. Returns what was written, if anything.
pub fn stamp_min_version(yaml: &mut Value) -> Option<String> {
    let (_, needed) = required_version(yaml)?;
    let obj = yaml.as_object_mut()?;
    let runtime = obj.entry("runtime").or_insert_with(|| Value::Object(Default::default()));
    let runtime = runtime.as_object_mut()?;
    let current = runtime.get("minVersion").and_then(Value::as_str).map(String::from);
    if current.as_deref().is_some_and(|c| crate::recipe::compare_versions(c, needed) != std::cmp::Ordering::Less) {
        return current;
    }
    runtime.insert("minVersion".into(), Value::String(needed.into()));
    Some(needed.to_string())
}

// ---------------------------------------------------------------------------------------------
// The deprecation registry

/// Where a deprecation applies in a recipe (the stage id, say; None for the whole recipe) and
/// what the Note about it says.
pub type Detection = (Option<String>, String);

/// One way a recipe can be out of date, and how to bring it up to date. `detect` names each
/// place in a current recipe where it applies (the stage id, say) with the Note's text;
/// `rewrite` repairs one such place, or every place when no target is given, and says whether
/// it changed anything.
pub struct Deprecation {
    /// Short id, the first part of a `FixId`.
    pub id: &'static str,
    /// Where in the recipe it applies, as a path with `*` for any index.
    pub path: &'static str,
    /// The Stillroom version that started treating it as out of date.
    pub since: &'static str,
    /// What the shape is now.
    pub replaced_by: &'static str,
    /// What the Fix button says.
    pub fix_label: &'static str,
    pub detect: fn(&Value) -> Vec<Detection>,
    pub rewrite: fn(&mut Value, Option<&str>) -> bool,
}

/// The table, in the order one `repair_all` pass needs: bare step references first (so the
/// placement below sees `${steps.x.output}` and knows what a step reads), then a tool step's
/// parameters move under `inputs` (so placement sees what it reads), then a step in no stage
/// finds one, then a stage that writes a keep declares it.
pub const DEPRECATIONS: &[Deprecation] = &[
    Deprecation {
        id: "step_ref",
        path: "/steps/*/inputs",
        since: "0.3.1",
        replaced_by: "a step's result is written `${steps.<id>.output}` (with `.field` after it to reach inside), never `${<id>}`",
        fix_label: "Point at the step's result",
        detect: detect_bare_step_refs,
        rewrite: rewrite_bare_step_refs,
    },
    Deprecation {
        id: "tool_inputs",
        path: "/steps/*/params",
        since: "0.3.2",
        replaced_by: "a tool step's parameters are its `inputs`; `params` is a model step's temperature and thinking",
        fix_label: "Move them under inputs",
        detect: detect_tool_params,
        rewrite: tool_params_to_inputs,
    },
    Deprecation {
        id: "place_step",
        path: "/stages/*/steps",
        since: "0.3.1",
        replaced_by: "when a recipe declares stages, every step is listed in one of them",
        fix_label: "Put it in a stage",
        detect: detect_unplaced_steps,
        rewrite: place_steps,
    },
    Deprecation {
        id: "declare_keep",
        path: "/stages/*/keep",
        since: "0.3.0",
        replaced_by: "a stage whose steps write a keep (keep.put, keep.index, keep.remove) declares `keep: { refresh: ... }`",
        fix_label: "Declare the keep",
        detect: detect_undeclared_keeps,
        rewrite: declare_keeps,
    },
];

pub fn deprecation(id: &str) -> Option<&'static Deprecation> {
    DEPRECATIONS.iter().find(|d| d.id == id)
}

/// Every Note the registry has about a current recipe, with its fix id.
pub fn notes_for(yaml: &Value) -> Vec<Finding> {
    DEPRECATIONS
        .iter()
        .flat_map(|d| {
            (d.detect)(yaml).into_iter().map(move |(target, text)| {
                let id = match &target {
                    Some(t) => format!("{}:{t}", d.id),
                    None => d.id.to_string(),
                };
                Finding::note(text).with_fix(id, d.fix_label)
            })
        })
        .collect()
}

/// Apply one fix by id. Unknown ids are an error, in words.
pub fn apply_fix(yaml: &mut Value, fix: &str) -> Result<bool, String> {
    let (id, target) = match fix.split_once(':') {
        Some((id, t)) => (id, Some(t)),
        None => (fix, None),
    };
    let d = deprecation(id).ok_or_else(|| format!("There is no fix called “{fix}”."))?;
    Ok((d.rewrite)(yaml, target))
}

/// Apply every registry rewrite everywhere: what a freshly drafted recipe gets before it is
/// checked, so the planner's slips are repaired the same way an author's would be.
pub fn repair_all(yaml: &mut Value) -> bool {
    let mut changed = false;
    for d in DEPRECATIONS {
        changed |= (d.rewrite)(yaml, None);
    }
    changed
}

// ---------------------------------------------------------------------------------------------
// step_ref: `${x}` for a step's result, where `${steps.x.output}` is meant

/// What a bare step reference has to say, as the checker's Problem and the registry's Note both
/// word it, so the refusal and the Fix that clears it read as one thing. `who` is the subject,
/// already quoted (“Chunk materials”, or Result “note”).
pub fn bare_step_ref_text(who: &str, name: &str, path: &[String]) -> String {
    let tail = path.iter().map(|p| format!(".{p}")).collect::<String>();
    format!("{who} points at ${{{name}{tail}}}; a step's result is written ${{steps.{name}.output{tail}}}.")
}

/// A step's own fields that may hold a reference. Its inner steps are walked by the caller,
/// which knows their scope.
const REF_FIELDS: [&str; 4] = ["inputs", "over", "when", "items"];

fn each_string_mut(v: &mut Value, f: &mut dyn FnMut(&mut String)) {
    match v {
        Value::String(s) => f(s),
        Value::Array(a) => a.iter_mut().for_each(|x| each_string_mut(x, f)),
        Value::Object(o) => o.values_mut().for_each(|x| each_string_mut(x, f)),
        _ => {}
    }
}

/// `${x}` or `${x.a.b}` where `x` is a step the holder can see, and is not what the loop calls
/// each item: the step meant and the path into its result.
fn bare_step_ref(s: &str, in_scope: &[&str], item: Option<&str>) -> Option<(String, Vec<String>)> {
    match parse_ref(s) {
        Some(Ok(Ref::Var { name, path })) if item != Some(name.as_str()) && in_scope.contains(&name.as_str()) => Some((name, path)),
        _ => None,
    }
}

fn step_id(step: &Value) -> &str {
    step.get("id").and_then(Value::as_str).unwrap_or("")
}

fn step_label(step: &Value) -> &str {
    step.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).unwrap_or_else(|| step_id(step))
}

/// One place a reference can be written: a top-level step, an inner step, or a result. The
/// target reads `<step>`, `<map>/<inner>`, or `result/<name>`; `scope` is the steps it may
/// point at (never itself, never the map around it); `item` is what its loop calls each entry.
struct RefHolder {
    target: String,
    who: String,
    scope: Vec<String>,
    item: Option<String>,
}

fn ref_holders(yaml: &Value) -> Vec<RefHolder> {
    let steps = yaml.get("steps").and_then(Value::as_array).cloned().unwrap_or_default();
    let top: Vec<&str> = steps.iter().map(step_id).collect();
    let mut out = Vec::new();
    for step in &steps {
        let id = step_id(step);
        let others: Vec<String> = top.iter().filter(|t| **t != id).map(|t| t.to_string()).collect();
        out.push(RefHolder { target: id.to_string(), who: format!("“{}”", step_label(step)), scope: others.clone(), item: None });
        if let Some(inner) = step.get("steps").and_then(Value::as_array) {
            let item = step.get("as").and_then(Value::as_str).unwrap_or("item").to_string();
            let siblings: Vec<&str> = inner.iter().map(step_id).collect();
            for sub in inner {
                let mut scope = others.clone();
                scope.extend(siblings.iter().filter(|s| **s != step_id(sub)).map(|s| s.to_string()));
                out.push(RefHolder { target: format!("{id}/{}", step_id(sub)), who: format!("“{}”", step_label(sub)), scope, item: Some(item.clone()) });
            }
        }
    }
    for name in yaml.get("outputs").and_then(Value::as_object).into_iter().flat_map(|o| o.keys()) {
        out.push(RefHolder { target: format!("result/{name}"), who: format!("Result “{name}”"), scope: top.iter().map(|t| t.to_string()).collect(), item: None });
    }
    out
}

/// The value a holder's target names inside the recipe: the step itself, or a result's `from`.
fn holder_value<'a>(yaml: &'a mut Value, target: &str) -> Option<&'a mut Value> {
    if let Some(name) = target.strip_prefix("result/") {
        return yaml.get_mut("outputs")?.get_mut(name)?.get_mut("from");
    }
    let (outer, inner) = match target.split_once('/') {
        Some((o, i)) => (o, Some(i)),
        None => (target, None),
    };
    let steps = yaml.get_mut("steps")?.as_array_mut()?;
    let step = steps.iter_mut().find(|s| step_id(s) == outer)?;
    match inner {
        None => Some(step),
        Some(i) => step.get_mut("steps")?.as_array_mut()?.iter_mut().find(|s| step_id(s) == i),
    }
}

/// Every string of a holder's value that may hold a reference: a step's own fields, or a
/// result's `from`.
fn holder_strings(target: &str, v: &mut Value, f: &mut dyn FnMut(&mut String)) {
    if target.starts_with("result/") {
        each_string_mut(v, f);
        return;
    }
    let Some(o) = v.as_object_mut() else { return };
    for k in REF_FIELDS {
        if let Some(field) = o.get_mut(k) {
            each_string_mut(field, f);
        }
    }
}

/// Steps and results that point at a step by its bare id: (target, what the Note says), one
/// Note per reference as written.
fn detect_bare_step_refs(yaml: &Value) -> Vec<Detection> {
    let mut scratch = yaml.clone();
    let mut out: Vec<Detection> = Vec::new();
    for h in ref_holders(yaml) {
        let Some(v) = holder_value(&mut scratch, &h.target) else { continue };
        let scope: Vec<&str> = h.scope.iter().map(String::as_str).collect();
        let mut texts: Vec<String> = Vec::new();
        holder_strings(&h.target, v, &mut |s| {
            if let Some((name, path)) = bare_step_ref(s, &scope, h.item.as_deref()) {
                let text = bare_step_ref_text(&h.who, &name, &path);
                if !texts.contains(&text) {
                    texts.push(text);
                }
            }
        });
        out.extend(texts.into_iter().map(|t| (Some(h.target.clone()), t)));
    }
    out
}

/// `${x}` becomes `${steps.x.output}` and `${x.a.b}` becomes `${steps.x.output.a.b}`, in the
/// holder named (or everywhere). Models write a step's id for its result; so do people.
fn rewrite_bare_step_refs(yaml: &mut Value, target: Option<&str>) -> bool {
    let mut changed = false;
    for h in ref_holders(yaml) {
        if target.is_some_and(|t| t != h.target) {
            continue;
        }
        let Some(v) = holder_value(yaml, &h.target) else { continue };
        let scope: Vec<&str> = h.scope.iter().map(String::as_str).collect();
        holder_strings(&h.target, v, &mut |s| {
            if let Some((name, path)) = bare_step_ref(s, &scope, h.item.as_deref()) {
                let tail = path.iter().map(|p| format!(".{p}")).collect::<String>();
                *s = format!("${{steps.{name}.output{tail}}}");
                changed = true;
            }
        });
    }
    changed
}

// ---------------------------------------------------------------------------------------------
// place_step: a recipe with stages lists every step in one of them

fn stage_lists<'a>(stage: &'a Value, key: &str) -> Vec<&'a str> {
    stage.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default()
}

/// The index of the stage that lists `name` under `key` (inputs, steps, outputs), if any does.
fn stage_index(stages: &[Value], key: &str, name: &str) -> Option<usize> {
    stages.iter().position(|st| stage_lists(st, key).contains(&name))
}

fn stage_list_push(stage: &mut Value, key: &str, name: &str) {
    if let Some(o) = stage.as_object_mut() {
        let list = o.entry(key).or_insert_with(|| Value::Array(Vec::new()));
        if let Some(a) = list.as_array_mut() {
            a.push(Value::String(name.to_string()));
        }
    }
}

/// Top-level steps a recipe with stages lists in none of them: (step id, what the Note says).
/// An unlisted step falls into the implied Make stage at the end, which declares no keep and
/// which the author never wrote.
fn detect_unplaced_steps(yaml: &Value) -> Vec<Detection> {
    let Some(stages) = yaml.get("stages").and_then(Value::as_array).filter(|a| !a.is_empty()) else { return Vec::new() };
    yaml.get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|s| stage_index(stages, "steps", step_id(s)).is_none())
        .map(|s| (Some(step_id(s).to_string()), format!("“{}” is in no stage.", step_label(s))))
        .collect()
}

/// Put the step named (or every unlisted step) into a stage: the one holding the latest step it
/// reads from, else the one listing an input it reads, else the last stage. A result that comes
/// from the step goes with it; an input it reads that no stage lists goes to the first stage
/// whose steps read it. Steps are placed in recipe order, so a step finds the ones before it.
fn place_steps(yaml: &mut Value, target: Option<&str>) -> bool {
    let steps = yaml.get("steps").and_then(Value::as_array).cloned().unwrap_or_default();
    let outputs = yaml.get("outputs").and_then(Value::as_object).cloned().unwrap_or_default();
    let Some(stages) = yaml.get_mut("stages").and_then(Value::as_array_mut).filter(|a| !a.is_empty()) else { return false };
    let mut changed = false;
    for step in &steps {
        let id = step_id(step);
        if target.is_some_and(|t| t != id) || stage_index(stages, "steps", id).is_some() {
            continue;
        }
        let mut reads = BTreeSet::new();
        step_refs(step, &mut reads);
        let mut inputs = BTreeSet::new();
        input_refs(step, &mut inputs);
        let by_step = reads.iter().filter_map(|r| stage_index(stages, "steps", r)).max();
        let by_input = inputs.iter().filter_map(|n| stage_index(stages, "inputs", n)).max();
        let at = by_step.or(by_input).unwrap_or(stages.len() - 1);
        stage_list_push(&mut stages[at], "steps", id);
        changed = true;
        for (name, out) in &outputs {
            let from = out.get("from").and_then(Value::as_str).unwrap_or("");
            let mine = matches!(parse_ref(from), Some(Ok(Ref::Step { step, .. })) if step == id);
            if mine && stage_index(stages, "outputs", name).is_none() {
                stage_list_push(&mut stages[at], "outputs", name);
            }
        }
        for n in &inputs {
            if stage_index(stages, "inputs", n).is_some() {
                continue;
            }
            let first_reader = stages.iter().position(|st| {
                stage_lists(st, "steps").iter().any(|sid| {
                    steps.iter().find(|s| step_id(s) == *sid).is_some_and(|s| {
                        let mut r = BTreeSet::new();
                        input_refs(s, &mut r);
                        r.contains(n)
                    })
                })
            });
            if let Some(i) = first_reader {
                stage_list_push(&mut stages[i], "inputs", n);
            }
        }
    }
    changed
}

// ---------------------------------------------------------------------------------------------
// tool_inputs: a tool step's parameters live under `inputs`, not `params`

/// Whether a tool step put its parameters under `params` (and has no inputs of its own). A tool
/// step like that checks clean and then runs the tool with nothing. Models write it because the
/// tool list calls them parameters; the fix is a rename.
fn tool_params_misplaced(step: &Value) -> bool {
    step.get("kind").and_then(Value::as_str) == Some("tool")
        && step.get("params").is_some_and(|p| p.as_object().is_some_and(|o| !o.is_empty()))
        && !step.get("inputs").is_some_and(|i| i.as_object().is_some_and(|o| !o.is_empty()))
}

/// Every tool step with misplaced parameters: (target, Note). Inner steps are `<map>/<inner>`.
fn detect_tool_params(yaml: &Value) -> Vec<Detection> {
    let mut out = Vec::new();
    for step in yaml.get("steps").and_then(Value::as_array).into_iter().flatten() {
        if tool_params_misplaced(step) {
            out.push((Some(step_id(step).to_string()), format!("“{}” puts its parameters under params; a tool step takes them under inputs.", step_label(step))));
        }
        for inner in step.get("steps").and_then(Value::as_array).into_iter().flatten() {
            if tool_params_misplaced(inner) {
                out.push((Some(format!("{}/{}", step_id(step), step_id(inner))), format!("“{}” puts its parameters under params; a tool step takes them under inputs.", step_label(inner))));
            }
        }
    }
    out
}

/// Rename `params` to `inputs` on the tool step named (or on every one that needs it).
fn tool_params_to_inputs(yaml: &mut Value, target: Option<&str>) -> bool {
    fn rename(step: &mut Value) -> bool {
        if !tool_params_misplaced(step) {
            return false;
        }
        let Some(o) = step.as_object_mut() else { return false };
        if let Some(p) = o.remove("params") {
            o.insert("inputs".into(), p);
            return true;
        }
        false
    }
    let mut changed = false;
    let Some(steps) = yaml.get_mut("steps").and_then(Value::as_array_mut) else { return false };
    for step in steps.iter_mut() {
        let outer = step_id(step).to_string();
        if target.is_none_or(|t| t == outer) {
            changed |= rename(step);
        }
        if let Some(inner_steps) = step.get_mut("steps").and_then(Value::as_array_mut) {
            for inner in inner_steps.iter_mut() {
                let path = format!("{outer}/{}", step_id(inner));
                if target.is_none_or(|t| t == path) {
                    changed |= rename(inner);
                }
            }
        }
    }
    changed
}

// ---------------------------------------------------------------------------------------------
// declare_keep: a stage whose steps write a keep says so

/// Tools that change a keep (a read of an earlier stage's keep is not a write).
fn writes_keep(tool: &str) -> bool {
    matches!(tool, "keep.put" | "keep.index" | "keep.remove")
}

/// The keep-writing tools of a step, inside a loop too; agents count by their allowed tools.
fn keep_writers(step: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(t) = step.get("tool").and_then(Value::as_str).filter(|t| writes_keep(t)) {
        out.push(t.to_string());
    }
    if let Some(tools) = step.get("allowedTools").and_then(Value::as_array) {
        out.extend(tools.iter().filter_map(Value::as_str).filter(|t| writes_keep(t)).map(String::from));
    }
    if let Some(inner) = step.get("steps").and_then(Value::as_array) {
        for s in inner {
            out.extend(keep_writers(s));
        }
    }
    out
}

/// Stages whose steps write a keep without declaring one: (stage id, what the Note says).
fn detect_undeclared_keeps(yaml: &Value) -> Vec<Detection> {
    let steps = yaml.get("steps").and_then(Value::as_array).cloned().unwrap_or_default();
    let Some(stages) = yaml.get("stages").and_then(Value::as_array) else { return Vec::new() };
    let mut out = Vec::new();
    for st in stages {
        if st.get("keep").is_some_and(Value::is_object) {
            continue;
        }
        let ids: Vec<&str> = st.get("steps").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let writes = steps.iter().filter(|s| s.get("id").and_then(Value::as_str).is_some_and(|id| ids.contains(&id))).any(|s| !keep_writers(s).is_empty());
        if writes {
            let id = st.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let name = st.get("name").and_then(Value::as_str).unwrap_or(&id);
            out.push((Some(id.clone()), format!("Stage “{name}” writes a keep but does not say so.")));
        }
    }
    out
}

/// Declare a keep on the stage named (or on every stage that needs one): `when_sources_change`
/// when the stage indexes, `always` otherwise (a log). Models reach for the keep tools and
/// forget the declaration; so do people copying a recipe by hand.
fn declare_keeps(yaml: &mut Value, target: Option<&str>) -> bool {
    let steps = yaml.get("steps").and_then(Value::as_array).cloned().unwrap_or_default();
    let Some(stages) = yaml.get_mut("stages").and_then(Value::as_array_mut) else { return false };
    let mut changed = false;
    for st in stages.iter_mut() {
        if target.is_some_and(|t| st.get("id").and_then(Value::as_str) != Some(t)) || st.get("keep").is_some_and(Value::is_object) {
            continue;
        }
        let ids: Vec<&str> = st.get("steps").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let writers: Vec<String> = steps.iter().filter(|s| s.get("id").and_then(Value::as_str).is_some_and(|id| ids.contains(&id))).flat_map(keep_writers).collect();
        if writers.is_empty() {
            continue;
        }
        let refresh = if writers.iter().any(|t| t == "keep.index") { "when_sources_change" } else { "always" };
        if let Some(o) = st.as_object_mut() {
            o.insert("keep".into(), json!({ "refresh": refresh }));
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_step_with_params_is_a_note_whose_fix_moves_them_under_inputs() {
        let mut y = json!({
            "schemaVersion": "0", "id": "x", "name": "X",
            "steps": [
                { "id": "list", "kind": "tool", "name": "Find the files", "tool": "fs.list", "params": { "path": "${inputs.folder}" } },
                { "id": "each", "kind": "map", "over": "${steps.list.output.entries}", "steps": [
                    { "id": "read", "kind": "tool", "tool": "file.read", "params": { "path": "${item.path}" } },
                    { "id": "write", "kind": "model", "slot": "writer", "prompt": "prompts/w.md", "inputs": { "text": "${steps.read.output}" }, "params": { "temperature": 0.2 } }
                ] },
                { "id": "fine", "kind": "tool", "tool": "fs.write", "inputs": { "dir": "${inputs.out}", "name": "a.md", "text": "t" } }
            ],
            "outputs": {}
        });
        let notes = notes_for(&y);
        let shape: Vec<(&str, &str)> = notes.iter().map(|n| (n.text.as_str(), n.fix.as_deref().unwrap_or(""))).collect();
        assert_eq!(
            shape,
            [
                ("“Find the files” puts its parameters under params; a tool step takes them under inputs.", "tool_inputs:list"),
                ("“read” puts its parameters under params; a tool step takes them under inputs.", "tool_inputs:each/read"),
            ]
        );
        assert_eq!(notes[0].fix_label.as_deref(), Some("Move them under inputs"));
        assert!(apply_fix(&mut y, "tool_inputs:each/read").unwrap());
        assert_eq!(y["steps"][1]["steps"][0]["inputs"], json!({ "path": "${item.path}" }));
        assert!(y["steps"][1]["steps"][0].get("params").is_none());
        assert_eq!(y["steps"][1]["steps"][1]["params"], json!({ "temperature": 0.2 }), "a model step keeps its params");
        assert!(repair_all(&mut y));
        assert_eq!(y["steps"][0]["inputs"], json!({ "path": "${inputs.folder}" }));
        assert!(notes_for(&y).is_empty(), "{:?}", notes_for(&y));
        assert!(!repair_all(&mut y), "nothing left to repair");
    }

    #[test]
    fn a_current_recipe_passes_through_unchanged_and_idempotently() {
        let y = json!({ "schemaVersion": "0", "id": "x", "name": "X", "steps": [], "outputs": {} });
        let m = migrate(y.clone()).unwrap();
        assert_eq!(m.yaml, y);
        assert_eq!(m.from_version, "0");
        assert!(m.notes.is_empty());
        let again = migrate(m.yaml.clone()).unwrap();
        assert_eq!(again.yaml, m.yaml);
        assert!(again.notes.is_empty());
    }

    #[test]
    fn a_missing_or_numeric_version_is_read_as_the_text_with_a_note() {
        let m = migrate(json!({ "id": "x" })).unwrap();
        assert_eq!(m.yaml["schemaVersion"], "0");
        assert_eq!(m.from_version, "0");
        assert_eq!(m.notes.len(), 1);
        assert!(m.notes[0].text.contains("does not say which recipe format"), "{}", m.notes[0].text);
        assert_eq!(m.notes[0].severity, Severity::Note);
        let m = migrate(json!({ "schemaVersion": 0, "id": "x" })).unwrap();
        assert_eq!(m.yaml["schemaVersion"], "0");
        assert!(m.notes[0].text.contains("written as the number 0"), "{}", m.notes[0].text);
        assert!(migrate(json!({ "schemaVersion": "0" })).unwrap().notes.is_empty(), "no note the second time");
    }

    #[test]
    fn an_unknown_version_and_a_non_recipe_are_refused_in_words() {
        let e = migrate(json!({ "schemaVersion": "7" })).unwrap_err();
        assert!(matches!(e, MigrateError::UnknownVersion(ref v) if v == "7"));
        assert_eq!(e.to_string(), "This recipe is written in recipe format “7”, which this Stillroom does not read (it reads “0”).");
        assert!(matches!(migrate(json!([1, 2])), Err(MigrateError::NotARecipe)));
        assert!(matches!(migrate(json!({ "schemaVersion": true })), Err(MigrateError::UnknownVersion(_))));
    }

    #[test]
    fn the_chain_and_the_schema_agree_on_versions() {
        let schema: Value = serde_json::from_str(crate::recipe::RECIPE_SCHEMA).unwrap();
        let accepted: Vec<&str> = schema["properties"]["schemaVersion"]["enum"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert_eq!(accepted, ACCEPTED_SCHEMA_VERSIONS);
        assert!(ACCEPTED_SCHEMA_VERSIONS.contains(&CURRENT_SCHEMA_VERSION));
        for (from, to, _) in CHAIN {
            assert!(ACCEPTED_SCHEMA_VERSIONS.contains(from) && ACCEPTED_SCHEMA_VERSIONS.contains(to));
        }
    }

    #[test]
    fn the_version_floor_follows_the_features_and_never_drops_an_authors_higher_one() {
        let mut plain = json!({ "schemaVersion": "0", "id": "x", "name": "X", "steps": [], "outputs": {} });
        assert_eq!(required_version(&plain), None);
        assert_eq!(stamp_min_version(&mut plain), None);
        assert!(plain.get("runtime").is_none(), "nothing to stamp, nothing written");
        let mut staged = plain.clone();
        staged["stages"] = json!([{ "id": "a", "name": "A" }]);
        assert_eq!(required_version(&staged), Some(("stages", "0.3.0")));
        assert_eq!(stamp_min_version(&mut staged).as_deref(), Some("0.3.0"));
        assert_eq!(staged["runtime"]["minVersion"], "0.3.0");
        staged["stages"][0]["keep"] = json!({ "refresh": "always" });
        assert_eq!(required_version(&staged).map(|x| x.1), Some("0.3.0"));
        staged["stages"][0]["source"] = json!({ "stage": "a", "version": 1, "digest": "sha256:0" });
        assert_eq!(required_version(&staged).map(|x| x.1), Some("0.3.0"), "provenance needs nothing newer than stages themselves");
        assert_eq!(stamp_min_version(&mut staged).as_deref(), Some("0.3.0"));
        staged["runtime"]["minVersion"] = json!("9.0");
        assert_eq!(stamp_min_version(&mut staged).as_deref(), Some("9.0"), "an author's higher floor stays");
        assert_eq!(supported_version("0.2.0"), "0.4.0", "this build reads every feature in the table");
        assert_eq!(supported_version("1.2.0"), "1.2.0");
        // Either-or groups are the first feature past 0.3.0.
        staged["runtime"]["minVersion"] = json!("0.3.0");
        staged["inputs"] = json!({ "urls": { "type": "urls", "required": false }, "folder": { "type": "directory", "required": false } });
        staged["inputGroups"] = json!([{ "anyOf": ["urls", "folder"] }]);
        assert_eq!(required_version(&staged), Some(("input groups", "0.4.0")));
        assert_eq!(stamp_min_version(&mut staged).as_deref(), Some("0.4.0"));
    }

    #[test]
    fn an_undeclared_keep_is_a_note_with_a_fix_that_declares_it() {
        let mut y = json!({
            "schemaVersion": "0", "id": "x", "name": "X",
            "steps": [
                { "id": "fetch", "kind": "tool", "tool": "http.fetch", "inputs": {} },
                { "id": "log", "kind": "tool", "tool": "keep.put", "inputs": {} },
                { "id": "index", "kind": "map", "over": "${steps.fetch.output}", "steps": [ { "id": "store", "kind": "tool", "tool": "keep.index", "inputs": {} } ] },
                { "id": "peek", "kind": "tool", "tool": "keep.query", "inputs": { "stage": "remember" } }
            ],
            "outputs": {},
            "stages": [
                { "id": "read", "name": "Read", "steps": ["fetch"] },
                { "id": "remember", "name": "Remember", "steps": ["log"] },
                { "id": "docs", "name": "Index the files", "steps": ["index"] },
                { "id": "ask", "name": "Ask", "steps": ["peek"] }
            ]
        });
        let notes = notes_for(&y);
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert_eq!(notes[0].text, "Stage “Remember” writes a keep but does not say so.");
        assert_eq!(notes[0].fix.as_deref(), Some("declare_keep:remember"));
        assert_eq!(notes[0].fix_label.as_deref(), Some("Declare the keep"));
        assert_eq!(notes[1].fix.as_deref(), Some("declare_keep:docs"));
        assert!(apply_fix(&mut y, "declare_keep:remember").unwrap());
        assert_eq!(y["stages"][1]["keep"]["refresh"], "always");
        assert_eq!(y["stages"][2].get("keep"), None, "only the named stage was fixed");
        assert_eq!(notes_for(&y).len(), 1);
        assert!(repair_all(&mut y));
        assert_eq!(y["stages"][2]["keep"]["refresh"], "when_sources_change", "indexing keeps when sources change");
        assert_eq!(y["stages"][3].get("keep"), None, "a reader of another stage's keep needs none");
        assert!(notes_for(&y).is_empty());
        assert!(!repair_all(&mut y), "nothing left to repair");
        assert_eq!(apply_fix(&mut y, "polish_brass").unwrap_err(), "There is no fix called “polish_brass”.");
        let shape = serde_json::to_value(&notes[0]).unwrap();
        assert_eq!(shape, json!({ "text": "Stage “Remember” writes a keep but does not say so.", "severity": "note", "fix": "declare_keep:remember", "fixLabel": "Declare the keep" }));
        assert_eq!(serde_json::to_value(Finding::problem("p")).unwrap(), json!({ "text": "p", "severity": "problem", "fix": null, "fixLabel": null }));
    }

    #[test]
    fn the_table_runs_step_ref_then_place_step_then_declare_keep() {
        let ids: Vec<&str> = DEPRECATIONS.iter().map(|d| d.id).collect();
        assert_eq!(ids, ["step_ref", "tool_inputs", "place_step", "declare_keep"], "one repair_all pass must see rewritten references and moved parameters before it places, and placed steps before it declares");
        assert!(DEPRECATIONS.iter().all(|d| !d.since.is_empty() && !d.fix_label.is_empty() && !d.replaced_by.is_empty()));
    }

    #[test]
    fn a_bare_step_reference_is_a_note_with_a_fix_that_points_at_the_result() {
        let mut y = json!({
            "schemaVersion": "0", "id": "x", "name": "X",
            "inputs": { "materials": { "type": "files" } },
            "steps": [
                { "id": "read_materials", "kind": "map", "over": "${inputs.materials}", "steps": [ { "id": "read", "kind": "tool", "tool": "doc.text", "inputs": { "path": "${item}" } } ] },
                { "id": "chunk_materials", "name": "Chunk materials", "kind": "tool", "tool": "text.chunk", "inputs": { "text": "${read_materials}" } },
                { "id": "each", "kind": "map", "over": "${chunk_materials.chunks}", "as": "chunk", "steps": [
                    { "id": "first", "kind": "tool", "tool": "list.filter", "inputs": { "items": "${chunk}" } },
                    { "id": "second", "kind": "tool", "tool": "list.table", "inputs": { "items": "${first}", "all": "${chunk_materials}" } }
                ] },
                { "id": "synth", "kind": "tool", "tool": "list.table", "inputs": { "items": "${each}" }, "when": "${chunk_materials.ok}" },
                { "id": "shadow", "kind": "map", "over": "${steps.each.output}", "as": "synth", "steps": [ { "id": "use_it", "kind": "tool", "tool": "list.filter", "inputs": { "items": "${synth}" } } ] }
            ],
            "outputs": { "note": { "from": "${synth}", "type": "json" } }
        });
        let notes = notes_for(&y);
        let got: Vec<(&str, &str)> = notes.iter().map(|n| (n.text.as_str(), n.fix.as_deref().unwrap_or(""))).collect();
        assert_eq!(
            got,
            vec![
                ("“Chunk materials” points at ${read_materials}; a step's result is written ${steps.read_materials.output}.", "step_ref:chunk_materials"),
                ("“each” points at ${chunk_materials.chunks}; a step's result is written ${steps.chunk_materials.output.chunks}.", "step_ref:each"),
                ("“second” points at ${first}; a step's result is written ${steps.first.output}.", "step_ref:each/second"),
                ("“second” points at ${chunk_materials}; a step's result is written ${steps.chunk_materials.output}.", "step_ref:each/second"),
                ("“synth” points at ${each}; a step's result is written ${steps.each.output}.", "step_ref:synth"),
                ("“synth” points at ${chunk_materials.ok}; a step's result is written ${steps.chunk_materials.output.ok}.", "step_ref:synth"),
                ("Result “note” points at ${synth}; a step's result is written ${steps.synth.output}.", "step_ref:result/note"),
            ],
            "the loop item, the loop's own as name, and an as name that shadows a step id are all left alone"
        );
        assert!(notes.iter().all(|n| n.fix_label.as_deref() == Some("Point at the step's result")));
        assert!(apply_fix(&mut y, "step_ref:chunk_materials").unwrap());
        assert_eq!(y["steps"][1]["inputs"]["text"], "${steps.read_materials.output}");
        assert_eq!(y["steps"][2]["over"], "${chunk_materials.chunks}", "only the named step was fixed");
        assert_eq!(notes_for(&y).len(), 6);
        assert!(apply_fix(&mut y, "step_ref:each/second").unwrap());
        assert_eq!(y["steps"][2]["steps"][1]["inputs"], json!({ "items": "${steps.first.output}", "all": "${steps.chunk_materials.output}" }));
        assert_eq!(notes_for(&y).len(), 4);
        assert!(repair_all(&mut y));
        assert_eq!(y["steps"][2]["over"], "${steps.chunk_materials.output.chunks}", "the path follows the result");
        assert_eq!(y["steps"][3]["inputs"]["items"], "${steps.each.output}");
        assert_eq!(y["steps"][3]["when"], "${steps.chunk_materials.output.ok}");
        assert_eq!(y["outputs"]["note"]["from"], "${steps.synth.output}");
        assert_eq!(y["steps"][0]["steps"][0]["inputs"]["path"], "${item}", "the loop item stays");
        assert_eq!(y["steps"][2]["steps"][0]["inputs"]["items"], "${chunk}", "the loop's as name stays");
        assert_eq!(y["steps"][4]["steps"][0]["inputs"]["items"], "${synth}", "an as name that shadows a step id is the item, not the step");
        assert!(notes_for(&y).is_empty(), "{:?}", notes_for(&y));
        assert!(!repair_all(&mut y), "nothing left to repair");
        assert_eq!(bare_step_ref_text("“A”", "b", &["c".into(), "d".into()]), "“A” points at ${b.c.d}; a step's result is written ${steps.b.output.c.d}.");
    }

    #[test]
    fn a_step_in_no_stage_is_a_note_with_a_fix_that_places_it_by_what_it_reads() {
        let mut y = json!({
            "schemaVersion": "0", "id": "x", "name": "X",
            "inputs": { "folder": { "type": "directory" }, "question": { "type": "text" }, "extra": { "type": "json" } },
            "steps": [
                { "id": "list", "kind": "tool", "tool": "fs.list", "inputs": { "path": "${inputs.folder}" } },
                { "id": "chunk", "kind": "tool", "tool": "text.chunk", "inputs": { "text": "${steps.list.output}" } },
                { "id": "index", "name": "Index passages", "kind": "tool", "tool": "keep.index", "inputs": { "items": "${steps.chunk.output}" } },
                { "id": "ask", "kind": "tool", "tool": "keep.search", "inputs": { "stage": "gather", "query": "${inputs.question}" } },
                { "id": "orphan", "kind": "tool", "tool": "list.table", "inputs": { "items": "${inputs.extra}" } },
                { "id": "loose", "kind": "tool", "tool": "list.table", "inputs": { "items": [] } }
            ],
            "outputs": { "answer": { "from": "${steps.ask.output}", "type": "json" }, "table": { "from": "${steps.orphan.output}", "type": "json" } },
            "stages": [
                { "id": "gather", "name": "Gather", "inputs": ["folder"], "steps": ["list", "chunk"] },
                { "id": "ask", "name": "Ask", "inputs": ["question"] }
            ]
        });
        let notes = notes_for(&y);
        let got: Vec<(&str, &str, &str)> = notes.iter().map(|n| (n.text.as_str(), n.fix.as_deref().unwrap_or(""), n.fix_label.as_deref().unwrap_or(""))).collect();
        assert_eq!(
            got,
            vec![
                ("“Index passages” is in no stage.", "place_step:index", "Put it in a stage"),
                ("“ask” is in no stage.", "place_step:ask", "Put it in a stage"),
                ("“orphan” is in no stage.", "place_step:orphan", "Put it in a stage"),
                ("“loose” is in no stage.", "place_step:loose", "Put it in a stage"),
            ]
        );
        assert!(apply_fix(&mut y, "place_step:index").unwrap());
        assert_eq!(y["stages"][0]["steps"], json!(["list", "chunk", "index"]), "it joins the stage of the step it reads");
        assert_eq!(y["stages"][1].get("steps"), None, "only the named step was placed");
        assert_eq!(notes_for(&y).len(), 3 + 1, "three steps still to place, and Gather now writes a keep without saying so");
        assert!(repair_all(&mut y));
        assert_eq!(y["stages"][1]["steps"], json!(["ask", "orphan", "loose"]), "an input it reads places it; nothing to go on means the last stage");
        assert_eq!(y["stages"][1]["inputs"], json!(["question", "extra"]), "an input nobody listed joins the first stage whose steps read it");
        assert_eq!(y["stages"][1]["outputs"], json!(["answer", "table"]), "results go with the step they come from");
        assert_eq!(y["stages"][0]["keep"]["refresh"], "when_sources_change", "the same pass declares the keep of the stage the step landed in");
        assert!(notes_for(&y).is_empty(), "{:?}", notes_for(&y));
        assert!(!repair_all(&mut y), "nothing left to repair");
        // No stages, or an empty list of them: nothing to place a step into.
        let mut plain = y.clone();
        plain.as_object_mut().unwrap().remove("stages");
        assert!(notes_for(&plain).is_empty());
        assert!(!repair_all(&mut plain));
        plain["stages"] = json!([]);
        assert!(notes_for(&plain).is_empty());
        assert!(!repair_all(&mut plain));
    }

    #[test]
    fn one_repair_pass_fixes_a_bare_reference_the_placement_it_decides_and_the_keep() {
        // The run that motivated this: the unlisted keep.index step reads ${chunk} bare, so its
        // stage can only be found once the reference is rewritten; then the keep follows.
        let mut y = json!({
            "schemaVersion": "0", "id": "x", "name": "X",
            "inputs": { "folder": { "type": "directory" } },
            "steps": [
                { "id": "list", "kind": "tool", "tool": "fs.list", "inputs": { "path": "${inputs.folder}" } },
                { "id": "chunk", "kind": "tool", "tool": "text.chunk", "inputs": { "text": "${list}" } },
                { "id": "index", "kind": "tool", "tool": "keep.index", "inputs": { "items": "${chunk.chunks}" } },
                { "id": "ask", "kind": "tool", "tool": "keep.search", "inputs": { "stage": "gather", "query": "x" } }
            ],
            "outputs": { "answer": { "from": "${ask}", "type": "json" } },
            "stages": [
                { "id": "gather", "name": "Gather", "inputs": ["folder"], "steps": ["list", "chunk"] },
                { "id": "ask", "name": "Ask", "steps": ["ask"], "outputs": ["answer"] }
            ]
        });
        assert_eq!(notes_for(&y).len(), 4, "{:?}", notes_for(&y));
        assert!(repair_all(&mut y));
        assert_eq!(y["steps"][2]["inputs"]["items"], "${steps.chunk.output.chunks}");
        assert_eq!(y["stages"][0]["steps"], json!(["list", "chunk", "index"]));
        assert_eq!(y["stages"][0]["keep"]["refresh"], "when_sources_change");
        assert_eq!(y["outputs"]["answer"]["from"], "${steps.ask.output}");
        assert!(notes_for(&y).is_empty(), "{:?}", notes_for(&y));
        assert!(!repair_all(&mut y));
    }
}
