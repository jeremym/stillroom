//! The go-between for older recipes: `migrate` rewrites a recipe's YAML from the format version
//! it was written in to the current one before anything else reads it, and says in Notes what
//! it changed. The deprecation registry below is the same table read two ways: `migrate` applies
//! its rewrites when it bumps a version; the checker consults it on a current recipe and turns
//! each entry into a Note with a one-click Fix instead of a refusal.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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
    pub detect: fn(&Value) -> Vec<(Option<String>, String)>,
    pub rewrite: fn(&mut Value, Option<&str>) -> bool,
}

pub const DEPRECATIONS: &[Deprecation] = &[Deprecation {
    id: "declare_keep",
    path: "/stages/*/keep",
    since: "0.3.0",
    replaced_by: "a stage whose steps write a keep (keep.put, keep.index, keep.remove) declares `keep: { refresh: ... }`",
    fix_label: "Declare the keep",
    detect: detect_undeclared_keeps,
    rewrite: declare_keeps,
}];

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
fn detect_undeclared_keeps(yaml: &Value) -> Vec<(Option<String>, String)> {
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
