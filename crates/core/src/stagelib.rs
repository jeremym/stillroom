//! The stage library: ready-made stages the planner may pick instead of composing from raw
//! tools. Each lives in `stages/<id>/` as `stage.yaml` plus its prompt and schema files. When a
//! drafted recipe uses one, its inputs, slots, permissions, steps, outputs, and files are copied
//! into the recipe, so the recipe stays self-contained and editable like any other.

use crate::draft::Draft;
use crate::recipe::KeepSpec;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageDef {
    pub id: String,
    /// Bumped whenever the stage's steps, prompts, or keep rule change, so a recipe that copied
    /// an earlier version can be told a newer one exists.
    #[serde(default = "one")]
    pub version: u32,
    pub name: String,
    pub why: String,
    /// One line for the planner's list.
    pub summary: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Library stages that must come earlier in the recipe (this one reads their steps).
    #[serde(default)]
    pub requires: Vec<String>,
    /// Recipe inputs this stage asks for (added to the recipe when missing).
    #[serde(default)]
    pub inputs: BTreeMap<String, Value>,
    #[serde(default)]
    pub model_slots: BTreeMap<String, Value>,
    #[serde(default)]
    pub permissions: Option<Value>,
    #[serde(default)]
    pub keep: Option<KeepSpec>,
    /// Steps as they appear in recipe.yaml; ids are prefixed by convention so they never clash.
    #[serde(default)]
    pub steps: Vec<Value>,
    /// Recipe outputs this stage delivers.
    #[serde(default)]
    pub outputs: BTreeMap<String, Value>,
    /// What later stages can reference, one line each, e.g. "${steps.index_files.output}: …".
    #[serde(default)]
    pub returns: Vec<String>,
    /// Prompt and schema files, by bundle-relative path.
    #[serde(default)]
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default)]
pub struct StageLibrary {
    pub stages: Vec<StageDef>,
}

#[derive(Debug, thiserror::Error)]
pub enum StageLibError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{path}: {source}")]
    Yaml { path: PathBuf, source: serde_yaml::Error },
}

impl StageLibrary {
    /// Every `<dir>/<id>/stage.yaml`, with its `prompts/` and `schemas/` files.
    pub fn load(dir: &Path) -> Result<Self, StageLibError> {
        let mut stages = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else { return Ok(Self::default()) };
        let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.join("stage.yaml").is_file()).collect();
        dirs.sort();
        for d in dirs {
            let path = d.join("stage.yaml");
            let text = std::fs::read_to_string(&path).map_err(|source| StageLibError::Io { path: path.clone(), source })?;
            let mut def: StageDef = serde_yaml::from_str(&text).map_err(|source| StageLibError::Yaml { path: path.clone(), source })?;
            for sub in ["prompts", "schemas"] {
                let Ok(files) = std::fs::read_dir(d.join(sub)) else { continue };
                let mut files: Vec<PathBuf> = files.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
                files.sort();
                for f in files {
                    let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if name.starts_with('.') {
                        continue;
                    }
                    let text = std::fs::read_to_string(&f).map_err(|source| StageLibError::Io { path: f.clone(), source })?;
                    def.files.insert(format!("{sub}/{name}"), text);
                }
            }
            stages.push(def);
        }
        Ok(Self { stages })
    }

    pub fn get(&self, id: &str) -> Option<&StageDef> {
        self.stages.iter().find(|s| s.id == id)
    }

    /// The list the planner reads, one short block per stage: id, what it does, what it adds,
    /// what it returns. Kept terse; every line here is paid for on every plan.
    pub fn describe(&self) -> String {
        let mut s = String::new();
        for st in &self.stages {
            s.push_str(&format!("\n- `{}`: {}", st.id, st.summary));
            let mut facts: Vec<String> = Vec::new();
            if !st.inputs.is_empty() {
                facts.push(format!("adds inputs {}", st.inputs.keys().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", ")));
            }
            if !st.requires.is_empty() {
                facts.push(format!("use after {}", st.requires.iter().map(|r| format!("`{r}`")).collect::<Vec<_>>().join(", ")));
            }
            if !st.outputs.is_empty() {
                facts.push(format!("delivers {}", st.outputs.keys().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", ")));
            }
            if !facts.is_empty() {
                s.push_str(&format!(" ({})", facts.join("; ")));
            }
            s.push('.');
            for r in &st.returns {
                s.push_str(&format!("\n  - returns {r}"));
            }
            s.push('\n');
        }
        s
    }

    /// Copy a library stage into a draft: inputs, slots, permissions, steps, outputs, files; and
    /// replace the stage entry at `index` with the real one (keeping any name or why override).
    /// A step the recipe already has is left alone; see `update_into` for the other way.
    pub fn expand_into(&self, draft: &mut Draft, index: usize, use_id: &str) -> Result<(), String> {
        self.expand(draft, index, use_id, false)
    }

    /// Bring a stage copied earlier up to the library's current version: its steps and files are
    /// replaced with the current ones, and the stage entry's provenance is rewritten. Only for a
    /// copy whose steps are still as copied; the checker makes sure of that before offering it.
    pub fn update_into(&self, draft: &mut Draft, index: usize, use_id: &str) -> Result<(), String> {
        self.expand(draft, index, use_id, true)
    }

    fn expand(&self, draft: &mut Draft, index: usize, use_id: &str, replace: bool) -> Result<(), String> {
        let def = self.get(use_id).ok_or_else(|| format!("no ready-made stage called “{use_id}”"))?;
        let yaml = draft.yaml.as_object_mut().ok_or("recipe is not an object")?;
        // Inputs: add what is missing, never overwrite what the recipe declares.
        {
            let entry = yaml.entry("inputs").or_insert_with(|| Value::Object(Default::default()));
            if let Some(map) = entry.as_object_mut() {
                for (k, v) in &def.inputs {
                    map.entry(k.clone()).or_insert_with(|| v.clone());
                }
            }
        }
        // Slots: two stages that declare the same slot share it, asking for everything either
        // needs (the union of `requires`) and the larger context window.
        {
            let entry = yaml.entry("modelSlots").or_insert_with(|| Value::Object(Default::default()));
            if let Some(map) = entry.as_object_mut() {
                for (k, v) in &def.model_slots {
                    match map.get_mut(k) {
                        Some(existing) => merge_slot(existing, v),
                        None => {
                            map.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
        }
        if let Some(p) = &def.permissions {
            merge_permissions(yaml.entry("permissions").or_insert_with(|| Value::Object(Default::default())), p);
        }
        let step_ids: Vec<String> = def.steps.iter().filter_map(|s| s.get("id").and_then(Value::as_str).map(String::from)).collect();
        let digest;
        {
            let steps = yaml.entry("steps").or_insert_with(|| Value::Array(Vec::new()));
            let arr = steps.as_array_mut().ok_or("steps is not a list")?;
            let mut present: Vec<String> = arr.iter().filter_map(|x| x.get("id").and_then(Value::as_str).map(String::from)).collect();
            present.extend(step_ids.iter().cloned());
            for s in &def.steps {
                let id = s.get("id").and_then(Value::as_str).unwrap_or("");
                let resolved = resolve_optional_inputs(s.clone(), &present);
                match arr.iter_mut().find(|x| x.get("id").and_then(Value::as_str) == Some(id)) {
                    Some(existing) if replace => *existing = resolved,
                    Some(_) => {}
                    None => arr.push(resolved),
                }
            }
            digest = steps_digest(&steps_named(arr, &step_ids));
        }
        {
            let outputs = yaml.entry("outputs").or_insert_with(|| Value::Object(Default::default()));
            if let Some(map) = outputs.as_object_mut() {
                for (k, v) in &def.outputs {
                    map.entry(k.clone()).or_insert_with(|| v.clone());
                }
            }
        }
        for (path, text) in &def.files {
            if replace {
                draft.files.insert(path.clone(), text.clone());
            } else {
                draft.files.entry(path.clone()).or_insert_with(|| text.clone());
            }
        }
        let stages = yaml.entry("stages").or_insert_with(|| Value::Array(Vec::new()));
        if let Some(arr) = stages.as_array_mut() {
            // An input is asked for in the first stage that needs it; a later library stage
            // that also reads it does not list it again.
            let claimed: Vec<String> = arr
                .iter()
                .take(index)
                .flat_map(|s| s.get("inputs").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect::<Vec<_>>()).unwrap_or_default())
                .collect();
            let old = arr.get(index).cloned().unwrap_or(Value::Null);
            let name = old.get("name").and_then(Value::as_str).filter(|n| !n.trim().is_empty()).unwrap_or(&def.name).to_string();
            let why = old.get("why").and_then(Value::as_str).filter(|n| !n.trim().is_empty()).unwrap_or(&def.why).to_string();
            let mut st = serde_json::Map::new();
            st.insert("id".into(), Value::String(def.id.clone()));
            st.insert("name".into(), Value::String(name));
            st.insert("why".into(), Value::String(why));
            let mine: Vec<Value> = def.inputs.keys().filter(|k| !claimed.contains(k)).map(|k| Value::String(k.clone())).collect();
            if !mine.is_empty() {
                st.insert("inputs".into(), Value::Array(mine));
            }
            st.insert("steps".into(), Value::Array(step_ids.into_iter().map(Value::String).collect()));
            if !def.outputs.is_empty() {
                st.insert("outputs".into(), Value::Array(def.outputs.keys().map(|k| Value::String(k.clone())).collect()));
            }
            if let Some(k) = &def.keep {
                st.insert("keep".into(), serde_json::to_value(k).unwrap_or(Value::Null));
            }
            st.insert("source".into(), serde_json::json!({ "stage": def.id, "version": def.version, "digest": digest }));
            if index < arr.len() {
                arr[index] = Value::Object(st);
            } else {
                arr.push(Value::Object(st));
            }
        }
        Ok(())
    }

    /// Expand every `{ "use": id }` stage in a draft, in order. Unknown ids become problems.
    pub fn expand_all(&self, draft: &mut Draft) -> Vec<String> {
        let uses: Vec<(usize, String)> = draft
            .yaml
            .get("stages")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().enumerate().filter_map(|(i, s)| s.get("use").and_then(Value::as_str).map(|u| (i, u.to_string()))).collect())
            .unwrap_or_default();
        let mut problems = Vec::new();
        for (i, id) in uses {
            if let Err(e) = self.expand_into(draft, i, &id) {
                problems.push(e);
            }
        }
        problems
    }
}

/// A library step input written as `name?: ${steps.x...}` is optional: it is kept (as `name`)
/// when step `x` exists in the recipe and dropped otherwise, so a stage can read from a sibling
/// stage that may or may not have been used.
fn resolve_optional_inputs(mut step: Value, present: &[String]) -> Value {
    if let Some(inputs) = step.get_mut("inputs").and_then(Value::as_object_mut) {
        let keys: Vec<String> = inputs.keys().filter(|k| k.ends_with('?')).cloned().collect();
        for k in keys {
            let v = inputs.remove(&k).unwrap_or(Value::Null);
            let target = v.as_str().and_then(|s| s.strip_prefix("${steps.")).and_then(|s| s.split(['.', '}']).next()).map(String::from);
            if target.as_ref().is_none_or(|t| present.contains(t)) {
                inputs.insert(k.trim_end_matches('?').to_string(), v);
            }
        }
    }
    step
}

/// The steps of a stage as the recipe holds them, in the stage's order; what the digest covers.
pub fn steps_named(steps: &[Value], ids: &[String]) -> Vec<Value> {
    ids.iter().filter_map(|id| steps.iter().find(|s| s.get("id").and_then(Value::as_str) == Some(id.as_str())).cloned()).collect()
}

/// `sha256:<hex>` of the canonical JSON (keys sorted, no spaces) of some steps. Two copies of a
/// stage with the same steps digest the same whatever order their keys were written in.
pub fn steps_digest(steps: &[Value]) -> String {
    fn canonical(v: &Value) -> Value {
        match v {
            Value::Object(o) => {
                let sorted: BTreeMap<&String, Value> = o.iter().map(|(k, v)| (k, canonical(v))).collect();
                Value::Object(sorted.into_iter().map(|(k, v)| (k.clone(), v)).collect())
            }
            Value::Array(a) => Value::Array(a.iter().map(canonical).collect()),
            _ => v.clone(),
        }
    }
    let text = serde_json::to_string(&canonical(&Value::Array(steps.to_vec()))).unwrap_or_default();
    let mut h = Sha256::new();
    h.update(text.as_bytes());
    format!("sha256:{:x}", h.finalize())
}

/// Two declarations of one slot become one that satisfies both: `requires` is the union, the
/// larger `minContext` wins, and the first description, preference, and recommendation stay.
pub fn merge_slot(into: &mut Value, from: &Value) {
    let (Some(a), Some(b)) = (into.as_object_mut(), from.as_object()) else { return };
    if let Some(Value::Array(theirs)) = b.get("requires") {
        let mine = a.entry("requires").or_insert_with(|| Value::Array(Vec::new()));
        if let Some(mine) = mine.as_array_mut() {
            for x in theirs {
                if !mine.contains(x) {
                    mine.push(x.clone());
                }
            }
        }
    }
    if let Some(theirs) = b.get("minContext").and_then(Value::as_u64) {
        let mine = a.get("minContext").and_then(Value::as_u64).unwrap_or(0);
        if theirs > mine {
            a.insert("minContext".into(), Value::from(theirs));
        }
    }
    for k in ["preference", "recommended", "description"] {
        if !a.contains_key(k)
            && let Some(v) = b.get(k)
        {
            a.insert(k.into(), v.clone());
        }
    }
}

pub fn merge_permissions(into: &mut Value, from: &Value) {
    let (Some(a), Some(b)) = (into.as_object_mut(), from.as_object()) else { return };
    for (k, v) in b {
        match (a.get_mut(k), v) {
            (Some(Value::Object(ao)), Value::Object(bo)) => {
                for (kk, vv) in bo {
                    match (ao.get_mut(kk), vv) {
                        (Some(Value::Array(aa)), Value::Array(ba)) => {
                            for x in ba {
                                if !aa.contains(x) {
                                    aa.push(x.clone());
                                }
                            }
                        }
                        (None, _) => {
                            ao.insert(kk.clone(), vv.clone());
                        }
                        _ => {}
                    }
                }
            }
            (Some(Value::Array(aa)), Value::Array(ba)) => {
                for x in ba {
                    if !aa.contains(x) {
                        aa.push(x.clone());
                    }
                }
            }
            (None, _) => {
                a.insert(k.clone(), v.clone());
            }
            _ => {}
        }
    }
}

/// Where the library ships in the repository, for tests and the dev shell.
pub fn repo_stages_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stages")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolRegistry;

    #[test]
    fn every_library_stage_expands_into_a_recipe_that_checks_clean() {
        let lib = StageLibrary::load(&repo_stages_dir()).unwrap();
        assert!(!lib.stages.is_empty(), "the library has stages");
        let known: Vec<String> = ToolRegistry::builtin().catalog().into_iter().map(|t| t.id).collect();
        for st in &lib.stages {
            let uses: Vec<Value> = st.requires.iter().chain(std::iter::once(&st.id)).map(|id| serde_json::json!({ "use": id })).collect();
            let mut draft = Draft {
                yaml: serde_json::json!({ "schemaVersion": "0", "id": "x", "name": "X", "steps": [], "outputs": {}, "stages": uses }),
                files: Default::default(),
            };
            let problems = lib.expand_all(&mut draft);
            assert!(problems.is_empty(), "{}: {:?}", st.id, problems);
            // A stage that delivers no outputs needs one to be a whole recipe; give it a stand-in.
            if draft.yaml["outputs"].as_object().is_some_and(|o| o.is_empty()) {
                let last = draft.yaml["steps"].as_array().and_then(|a| a.last()).and_then(|s| s.get("id")).and_then(Value::as_str).unwrap_or("").to_string();
                draft.yaml["outputs"] = serde_json::json!({ "result": { "from": format!("${{steps.{last}.output}}"), "type": "json" } });
                let n = draft.yaml["stages"].as_array().map(Vec::len).unwrap_or(1);
                draft.yaml["stages"][n - 1]["outputs"] = serde_json::json!(["result"]);
            }
            let findings = draft.check(&known);
            assert!(findings.is_empty(), "{}: {:?}", st.id, findings);
            assert!(!st.summary.is_empty() && !st.why.is_empty(), "{} has a summary and a why", st.id);
        }
        let text = lib.describe();
        assert!(text.contains("- `index_folder`:"));
    }

    #[test]
    fn an_expanded_stage_carries_its_provenance_and_shared_slots_are_unioned() {
        let lib = StageLibrary::load(&repo_stages_dir()).unwrap();
        let mut draft = Draft {
            yaml: serde_json::json!({ "schemaVersion": "0", "id": "x", "name": "X", "steps": [], "outputs": {}, "stages": [{ "use": "index_folder" }, { "use": "answer_from_index" }] }),
            files: Default::default(),
        };
        assert!(lib.expand_all(&mut draft).is_empty());
        let st = &draft.yaml["stages"][0];
        assert_eq!(st["source"]["stage"], "index_folder");
        assert_eq!(st["source"]["version"], 1);
        let digest = st["source"]["digest"].as_str().unwrap().to_string();
        assert!(digest.starts_with("sha256:") && digest.len() == 71, "{digest}");
        let ids: Vec<String> = st["steps"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
        assert_eq!(steps_digest(&steps_named(draft.yaml["steps"].as_array().unwrap(), &ids)), digest, "the digest is of the steps as copied");
        let recipe: crate::recipe::Recipe = serde_json::from_value(draft.yaml.clone()).unwrap();
        assert_eq!(recipe.stages[0].source.as_ref().map(|s| s.version), Some(1));
        assert_eq!(recipe.stages[0].keep_id(), "index_folder", "a keep is named after its stage unless it says otherwise");
        let known: Vec<String> = ToolRegistry::builtin().catalog().into_iter().map(|t| t.id).collect();
        assert!(draft.check(&known).is_empty(), "{:?}", draft.check(&known));

        // Two stages declaring one slot: the recipe gets one slot that asks for both.
        let mut a = serde_json::json!({ "requires": ["text"], "minContext": 8192, "description": "first" });
        merge_slot(&mut a, &serde_json::json!({ "requires": ["text", "structured_output"], "minContext": 32768, "preference": "speed", "description": "second" }));
        assert_eq!(a, serde_json::json!({ "requires": ["text", "structured_output"], "minContext": 32768, "description": "first", "preference": "speed" }));
        let mut lib2 = lib.clone();
        let w = lib2.stages.iter_mut().find(|s| s.id == "write_brief").unwrap();
        w.model_slots.insert("reader".into(), serde_json::json!({ "requires": ["vision"], "minContext": 65536 }));
        let mut d2 = Draft { yaml: serde_json::json!({ "schemaVersion": "0", "id": "x", "name": "X", "steps": [], "outputs": {}, "stages": [{ "use": "read_links" }, { "use": "write_brief" }] }), files: Default::default() };
        assert!(lib2.expand_all(&mut d2).is_empty());
        assert_eq!(d2.yaml["modelSlots"]["reader"]["requires"], serde_json::json!(["text", "structured_output", "vision"]));
        assert_eq!(d2.yaml["modelSlots"]["reader"]["minContext"], 65536);
    }

    #[test]
    fn a_newer_library_version_is_a_note_only_while_the_copy_is_untouched() {
        let lib = StageLibrary::load(&repo_stages_dir()).unwrap();
        let known: Vec<String> = ToolRegistry::builtin().catalog().into_iter().map(|t| t.id).collect();
        let mut draft = Draft { yaml: serde_json::json!({ "schemaVersion": "0", "id": "x", "name": "X", "steps": [], "outputs": {}, "stages": [{ "use": "summarize_file", "name": "Read it" }] }), files: Default::default() };
        assert!(lib.expand_all(&mut draft).is_empty());
        assert!(draft.check_with(&known, Some(&lib)).is_empty(), "same version: nothing to say");

        // The library moves on: version 2 with a different prompt and a step tweak.
        let mut newer = lib.clone();
        {
            let def = newer.stages.iter_mut().find(|s| s.id == "summarize_file").unwrap();
            def.version = 2;
            def.files.insert("prompts/summarize_file.md".into(), "Summarize in two lines.".into());
            def.steps[1]["params"] = serde_json::json!({ "temperature": 0.1 });
        }
        let findings = draft.check_with(&known, Some(&newer));
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].text, "A newer version of the ready-made stage “Summarize a file” exists.");
        assert_eq!(findings[0].fix.as_deref(), Some("update_stage:summarize_file"));
        assert_eq!(findings[0].fix_label.as_deref(), Some("Update it"));
        assert!(draft.check(&known).is_empty(), "without the library in hand there is nothing to compare with");
        assert!(draft.fix(&["update_stage:summarize_file".into()]).is_err(), "the update needs the library");
        let updated = draft.fix_with(&["update_stage:summarize_file".into()], Some(&newer)).unwrap();
        assert_eq!(updated.yaml["stages"][0]["source"]["version"], 2);
        assert_eq!(updated.yaml["stages"][0]["name"], "Read it", "the author's name for the stage stays");
        assert_eq!(updated.yaml["steps"][1]["params"]["temperature"], 0.1);
        assert_eq!(updated.files["prompts/summarize_file.md"], "Summarize in two lines.");
        assert!(updated.check_with(&known, Some(&newer)).is_empty(), "{:?}", updated.check_with(&known, Some(&newer)));

        // An edited copy is never touched: the digest no longer matches, so no Note.
        let mut edited = draft.clone();
        edited.yaml["steps"][1]["inputs"]["text"] = serde_json::json!("${steps.summary_read.output.text} (edited)");
        assert!(edited.check_with(&known, Some(&newer)).is_empty(), "{:?}", edited.check_with(&known, Some(&newer)));
    }
}
