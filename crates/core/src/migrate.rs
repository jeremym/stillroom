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
pub const FORMAT_FEATURES: &[(&str, &str)] = &[("stages", "0.3.0"), ("keep", "0.3.0"), ("stage source", "0.4.0")];

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
/// placement below sees `${steps.x.output}` and knows what a step reads), then a step in no
/// stage finds one, then a stage that writes a keep declares it.
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
        assert_eq!(required_version(&staged), Some(("stage source", "0.4.0")));
        assert_eq!(stamp_min_version(&mut staged).as_deref(), Some("0.4.0"));
        staged["runtime"]["minVersion"] = json!("9.0");
        assert_eq!(stamp_min_version(&mut staged).as_deref(), Some("9.0"), "an author's higher floor stays");
        assert_eq!(supported_version("0.2.0"), "0.4.0", "this build reads every feature in the table");
        assert_eq!(supported_version("1.2.0"), "1.2.0");
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
}
