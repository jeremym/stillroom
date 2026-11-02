//! The planner: a goal in words becomes a draft recipe. The model proposes inputs, steps, prompts,
//! and outputs as one JSON object; the draft checker says what is wrong; the model gets up to two
//! more tries with those problems. The planner never picks a model or provider for the recipe it
//! writes, and it never writes to disk: the user saves the draft from the recipe card.

use crate::draft::Draft;
use crate::executor::{Cancel, ModelDefaults};
use crate::model::{ChatMessage, ChatRequest, ModelClient};
use crate::recipe::{Capability, ModelSlot, Preference, Recipe};
use crate::tools::ToolInfo;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

/// The slot the planner itself runs in. One slot, resolved like any recipe slot: the user's pin
/// first, then their default, then the largest local model that answers in JSON.
pub const PLANNER_SLOT: &str = "planner";

pub fn planner_slot() -> ModelSlot {
    ModelSlot {
        requires: vec![Capability::Text, Capability::StructuredOutput],
        preference: Some(Preference::Accuracy),
        recommended: None,
        min_context: None,
        description: Some("Designs a recipe from your goal.".into()),
    }
}

/// A recipe with only the planner slot, so the ordinary resolver can fill it.
pub fn planner_recipe() -> Recipe {
    let mut model_slots = IndexMap::new();
    model_slots.insert(PLANNER_SLOT.to_string(), planner_slot());
    Recipe {
        schema_version: "0".into(),
        id: "__planner".into(),
        name: "Planner".into(),
        version: None,
        description: None,
        icon: None,
        inputs: IndexMap::new(),
        model_slots,
        permissions: None,
        skills: Vec::new(),
        steps: Vec::new(),
        outputs: IndexMap::new(),
        budget: None,
        stages: Vec::new(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRequest {
    /// What the recipe should do, in the user's words.
    pub goal: String,
    /// Notes, links, examples: anything the recipe should know. Becomes input defaults.
    #[serde(default)]
    pub material: Option<String>,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum PlanEvent {
    /// The first call: choosing the stages.
    Choosing,
    /// A model call is in flight.
    Thinking { attempt: u32, of: u32 },
    /// The answer is being checked.
    Checking { attempt: u32 },
    /// The checker found problems; the model is asked to fix them.
    Fixing { attempt: u32, problems: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanOutcome {
    pub draft: Draft,
    /// Problems the checker still finds. Empty means the draft can be saved as it is.
    pub problems: Vec<String>,
    pub attempts: u32,
    pub model_id: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub duration_ms: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("the model could not be reached: {0}")]
    Model(String),
    #[error("no answer within {0} s")]
    Timeout(u64),
    #[error("stopped")]
    Cancelled,
    #[error("the model never answered with a recipe: {0}")]
    Unusable(String),
}

/// What the model is asked to answer with. Loose on purpose: the real recipe schema is applied by
/// the draft checker afterwards, and its problems go back to the model in words.
pub fn answer_schema() -> Value {
    json!({
        "type": "object",
        "required": ["recipe", "files"],
        "properties": {
            "recipe": {
                "type": "object",
                "required": ["id", "name", "description", "inputs", "modelSlots", "steps", "outputs", "stages"],
                "properties": {
                    "id": { "type": "string" },
                    "name": { "type": "string" },
                    "description": { "type": "string" },
                    "inputs": { "type": "object" },
                    "modelSlots": { "type": "object" },
                    "permissions": { "type": "object" },
                    "steps": { "type": "array", "items": { "type": "object" } },
                    "outputs": { "type": "object" },
                    "stages": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "name": { "type": "string" },
                                "why": { "type": "string" },
                                "inputs": { "type": "array", "items": { "type": "string" } },
                                "steps": { "type": "array", "items": { "type": "string" } },
                                "outputs": { "type": "array", "items": { "type": "string" } },
                                "keep": { "type": "object", "properties": { "refresh": { "type": "string", "enum": ["always", "when_sources_change", "manual"] } } },
                                "use": { "type": "string", "description": "The id of a ready-made stage to use instead of steps" }
                            }
                        }
                    }
                }
            },
            "files": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["path", "text"],
                    "properties": { "path": { "type": "string" }, "text": { "type": "string" } }
                }
            }
        }
    })
}

/// The planner prompt with the live tool list and the stage library appended, so adding a tool or
/// a ready-made stage teaches the planner.
pub fn assemble_system(prompt: &str, tools: &[ToolInfo]) -> String {
    assemble_system_with(prompt, tools, None)
}

pub fn assemble_system_with(prompt: &str, tools: &[ToolInfo], library: Option<&crate::stagelib::StageLibrary>) -> String {
    let mut s = String::from(prompt.trim_end());
    if let Some(lib) = library.filter(|l| !l.stages.is_empty()) {
        s.push_str("\n\n## Ready-made stages\n\nThese stages are already built and tested. Prefer one whenever it does what a stage of the recipe should do; compose steps yourself only for what no ready-made stage covers. To use one, the stage entry is exactly `{ \"use\": \"<id>\" }`, optionally with your own `name` and `why`, and you write none of its steps, inputs, prompts, or files: they are added for you, with the ids listed under returns. A recipe that answers questions over a folder is just:\n\n```\n\"stages\": [ { \"use\": \"index_folder\" }, { \"use\": \"answer_from_index\" } ], \"steps\": [], \"inputs\": {}, \"modelSlots\": {}, \"outputs\": {}\n```\n\nYour own stages may follow and reference what a ready-made stage returns.\n");
        s.push_str(&lib.describe());
    }
    s.push_str("\n\n## Tools available\n\nThese are the only tools. Use the parameter names exactly.\n");
    for t in tools {
        s.push_str(&format!("\n### `{}`\n\n{}\n", t.id, t.description));
        if let Some(p) = &t.parameters {
            s.push_str(&format!("\nParameters (JSON Schema): `{}`\n", serde_json::to_string(p).unwrap_or_default()));
        }
        if !t.returns.is_empty() {
            s.push_str(&format!("\nOutput: {}\n", t.returns));
        }
        if !t.cacheable {
            s.push_str("\nActs on the world (writes or sends), so its result is never reused from an earlier run.\n");
        }
    }
    s.push_str("\n\nAnswer with the one JSON object described above and nothing else.");
    s
}

pub fn user_message(req: &PlanRequest) -> String {
    let mut s = format!("## Goal\n\n{}\n", req.goal.trim());
    if let Some(m) = req.material.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
        s.push_str(&format!("\n## Material the user supplied (notes, links, examples)\n\n{m}\n"));
    }
    s.push_str(&format!("\n## Use these\n\n- id: `{}`\n- name: {}\n", req.id, req.name));
    if let Some(i) = &req.icon {
        s.push_str(&format!("- icon: `{i}`\n"));
    }
    s
}

fn extract_json(content: &str) -> Result<Value, String> {
    let trimmed = content.trim();
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        return Ok(v);
    }
    let start = trimmed.find('{').ok_or("no JSON object in the answer")?;
    let end = trimmed.rfind('}').ok_or("no JSON object in the answer")?;
    if end <= start {
        return Err("no JSON object in the answer".into());
    }
    serde_json::from_str::<Value>(&trimmed[start..=end]).map_err(|e| e.to_string())
}

fn safe_bundle_path(p: &str) -> bool {
    let p = p.trim_start_matches("./");
    (p.starts_with("prompts/") || p.starts_with("schemas/")) && !p.split('/').any(|c| c == ".." || c.is_empty())
}

/// The model's answer as a draft, with the id, name, icon, and format version the user asked for.
pub fn parse_answer(content: &str, req: &PlanRequest) -> Result<Draft, String> {
    let v = extract_json(content)?;
    let mut recipe = match v.get("recipe") {
        Some(Value::Object(o)) => o.clone(),
        Some(_) => return Err("`recipe` is not an object".into()),
        None => match v.as_object() {
            Some(o) if o.contains_key("steps") => o.clone(),
            _ => return Err("the answer has no `recipe`".into()),
        },
    };
    let mut yaml = serde_json::Map::new();
    yaml.insert("schemaVersion".into(), Value::String("0".into()));
    yaml.insert("id".into(), Value::String(req.id.clone()));
    yaml.insert("name".into(), Value::String(req.name.clone()));
    if let Some(i) = &req.icon {
        yaml.insert("icon".into(), Value::String(i.clone()));
    }
    yaml.insert("version".into(), Value::String(recipe.get("version").and_then(Value::as_str).unwrap_or("0.1.0").to_string()));
    for k in ["schemaVersion", "id", "name", "icon", "version"] {
        recipe.remove(k);
    }
    for (k, val) in recipe {
        yaml.insert(k, val);
    }
    let mut yaml = crate::migrate::migrate(Value::Object(yaml)).map_err(|e| e.to_string())?.yaml;
    // A drafted recipe gets every registry repair (a stage that writes a keep declares one, and
    // so on): the same repairs the checker offers an author as Notes with a Fix.
    crate::migrate::repair_all(&mut yaml);
    let mut files = BTreeMap::new();
    match v.get("files") {
        Some(Value::Array(items)) => {
            for f in items {
                if let (Some(p), Some(t)) = (f.get("path").and_then(Value::as_str), f.get("text").and_then(Value::as_str)) {
                    let p = p.trim_start_matches("./").to_string();
                    if safe_bundle_path(&p) {
                        files.insert(p, t.to_string());
                    }
                }
            }
        }
        Some(Value::Object(map)) => {
            for (p, t) in map {
                if let (true, Some(t)) = (safe_bundle_path(p), t.as_str()) {
                    files.insert(p.trim_start_matches("./").to_string(), t.to_string());
                }
            }
        }
        _ => {}
    }
    Ok(Draft { yaml, files })
}

pub struct Planner {
    /// Ready-made stages the model may pick with `"use": "<id>"`.
    pub library: Option<Arc<crate::stagelib::StageLibrary>>,
    pub models: Arc<dyn ModelClient>,
    pub model_id: String,
    pub tools: Vec<ToolInfo>,
    /// The planner prompt in use (original or the user's revision).
    pub prompt: String,
    /// The strip prompt in use: the first, short call that only picks the stages.
    pub strip_prompt: String,
    pub timeout: Option<Duration>,
    pub cancel: Option<Cancel>,
    pub model_defaults: ModelDefaults,
    /// Model calls at most for the composing call, counting the fixes. Default 3.
    pub max_attempts: u32,
}

pub type PlanObserver = Arc<dyn Fn(PlanEvent) + Send + Sync>;

/// What the first call answers: the strip only.
#[derive(Debug, Deserialize)]
struct StripAnswer {
    #[serde(default)]
    stages: Vec<StripStage>,
}

#[derive(Debug, Deserialize)]
struct StripStage {
    #[serde(default, rename = "use")]
    use_: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    why: Option<String>,
}

pub fn strip_schema(lib: &crate::stagelib::StageLibrary) -> Value {
    let ids: Vec<Value> = lib.stages.iter().map(|s| Value::String(s.id.clone())).collect();
    json!({
        "type": "object",
        "required": ["stages"],
        "properties": {
            "stages": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "use": { "type": "string", "enum": ids },
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "why": { "type": "string" }
                    }
                }
            }
        }
    })
}

fn slug_id(name: &str, taken: &[String]) -> String {
    let mut base: String = name.to_ascii_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    base = base.trim_matches('_').to_string();
    while base.contains("__") {
        base = base.replace("__", "_");
    }
    if base.is_empty() || !base.chars().next().unwrap().is_ascii_alphabetic() {
        base = format!("stage_{base}");
    }
    let mut id = base.clone();
    let mut n = 2;
    while taken.iter().any(|t| t == &id) {
        id = format!("{base}_{n}");
        n += 1;
    }
    id
}

/// The already-built part of a draft, in words, for the composing call.
fn already_built(draft: &Draft, lib: &crate::stagelib::StageLibrary) -> String {
    let mut s = String::new();
    let Some(stages) = draft.yaml.get("stages").and_then(Value::as_array) else { return s };
    for (i, st) in stages.iter().enumerate() {
        let id = st.get("id").and_then(Value::as_str).unwrap_or("");
        let Some(def) = lib.get(id) else { continue };
        s.push_str(&format!("\n{}. **{}** (`{}`): {}\n", i + 1, st.get("name").and_then(Value::as_str).unwrap_or(&def.name), id, def.summary));
        if !def.inputs.is_empty() {
            s.push_str(&format!("   Inputs it already asks for: {}.\n", def.inputs.keys().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", ")));
        }
        if !def.model_slots.is_empty() {
            s.push_str(&format!("   Model slots already declared: {}.\n", def.model_slots.keys().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", ")));
        }
        for r in &def.returns {
            s.push_str(&format!("   - returns {r}\n"));
        }
    }
    s
}

/// Fold the model's answer for the custom stages into the skeleton that already holds the
/// ready-made ones: inputs, slots, outputs, and files are added when missing; steps are appended
/// by id; a custom stage takes the model's entry with the same id or name.
fn merge_partial(skeleton: &mut Draft, partial: &Draft, lib: &crate::stagelib::StageLibrary) {
    let Some(base) = skeleton.yaml.as_object_mut() else { return };
    let Some(part) = partial.yaml.as_object() else { return };
    if base.get("description").and_then(Value::as_str).unwrap_or("").is_empty() {
        if let Some(d) = part.get("description") {
            base.insert("description".into(), d.clone());
        }
    }
    for section in ["inputs", "modelSlots", "outputs"] {
        let entry = base.entry(section).or_insert_with(|| Value::Object(Default::default()));
        if let (Some(map), Some(from)) = (entry.as_object_mut(), part.get(section).and_then(Value::as_object)) {
            for (k, v) in from {
                map.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
    }
    if let Some(p) = part.get("permissions") {
        crate::stagelib::merge_permissions(base.entry("permissions").or_insert_with(|| Value::Object(Default::default())), p);
    }
    {
        let steps = base.entry("steps").or_insert_with(|| Value::Array(Vec::new()));
        if let (Some(arr), Some(from)) = (steps.as_array_mut(), part.get("steps").and_then(Value::as_array)) {
            for s in from {
                let id = s.get("id").and_then(Value::as_str).unwrap_or("");
                if !arr.iter().any(|x| x.get("id").and_then(Value::as_str) == Some(id)) {
                    arr.push(s.clone());
                }
            }
        }
    }
    let part_stages: Vec<Value> = part.get("stages").and_then(Value::as_array).cloned().unwrap_or_default();
    let lower = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(|s| s.trim().to_ascii_lowercase()).unwrap_or_default();
    let mut used: Vec<usize> = Vec::new();
    if let Some(stages) = base.get_mut("stages").and_then(Value::as_array_mut) {
        for st in stages.iter_mut() {
            let id = st.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            if lib.get(&id).is_some() {
                continue;
            }
            let found = part_stages.iter().enumerate().find(|(i, p)| {
                !used.contains(i) && p.get("use").is_none() && (lower(p, "id") == id.to_ascii_lowercase() || (!lower(p, "name").is_empty() && lower(p, "name") == lower(st, "name")))
            });
            if let Some((i, p)) = found {
                used.push(i);
                if let (Some(o), Some(po)) = (st.as_object_mut(), p.as_object()) {
                    for k in ["why", "inputs", "steps", "outputs", "keep"] {
                        if let Some(v) = po.get(k) {
                            o.insert(k.into(), v.clone());
                        }
                    }
                }
            }
        }
        for (i, p) in part_stages.iter().enumerate() {
            if used.contains(&i) || p.get("use").is_some() || p.get("id").and_then(Value::as_str).is_some_and(|id| lib.get(id).is_some()) {
                continue;
            }
            if p.get("id").and_then(Value::as_str).is_some() {
                stages.push(p.clone());
            }
        }
    }
    for (path, text) in &partial.files {
        skeleton.files.entry(path.clone()).or_insert_with(|| text.clone());
    }
    // An input, step, or output lives in the first stage that lists it.
    if let Some(stages) = base.get_mut("stages").and_then(Value::as_array_mut) {
        for key in ["inputs", "steps", "outputs"] {
            let mut seen: Vec<String> = Vec::new();
            for st in stages.iter_mut() {
                if let Some(list) = st.get_mut(key).and_then(Value::as_array_mut) {
                    list.retain(|v| {
                        let s = v.as_str().unwrap_or("").to_string();
                        if seen.contains(&s) {
                            false
                        } else {
                            seen.push(s);
                            true
                        }
                    });
                }
            }
        }
    }
    // The model sometimes names a ready-made stage where it means the step that stage returns.
    let aliases: Vec<(String, String)> = lib
        .stages
        .iter()
        .filter_map(|def| {
            let first = def.returns.iter().find_map(|r| r.split("${steps.").nth(1).and_then(|s| s.split(['.', '}']).next()).map(String::from))?;
            Some((format!("${{steps.{}.", def.id), format!("${{steps.{first}.")))
        })
        .collect();
    if !aliases.is_empty() {
        fn rewrite(v: &mut Value, aliases: &[(String, String)]) {
            match v {
                Value::String(s) => {
                    for (from, to) in aliases {
                        if s.contains(from.as_str()) {
                            *s = s.replace(from.as_str(), to);
                        }
                    }
                }
                Value::Array(a) => a.iter_mut().for_each(|x| rewrite(x, aliases)),
                Value::Object(o) => o.values_mut().for_each(|x| rewrite(x, aliases)),
                _ => {}
            }
        }
        if let Some(steps) = base.get_mut("steps") {
            rewrite(steps, &aliases);
        }
        if let Some(outputs) = base.get_mut("outputs") {
            rewrite(outputs, &aliases);
        }
    }
    crate::migrate::repair_all(&mut skeleton.yaml);
}

/// The strip answer as a skeleton draft: ready-made stages expanded, custom ones named with
/// their why and no steps yet. Also how many custom stages there are (none means nothing is
/// left to compose).
pub fn skeleton_from_strip(content: &str, req: &PlanRequest, lib: &crate::stagelib::StageLibrary) -> Result<(Draft, usize), String> {
    let answer: StripAnswer = serde_json::from_value(extract_json(content)?).map_err(|e| e.to_string())?;
    let mut yaml = serde_json::Map::new();
    yaml.insert("schemaVersion".into(), Value::String("0".into()));
    yaml.insert("id".into(), Value::String(req.id.clone()));
    yaml.insert("name".into(), Value::String(req.name.clone()));
    if let Some(i) = &req.icon {
        yaml.insert("icon".into(), Value::String(i.clone()));
    }
    yaml.insert("version".into(), Value::String("0.1.0".into()));
    yaml.insert("description".into(), Value::String(String::new()));
    yaml.insert("inputs".into(), Value::Object(Default::default()));
    yaml.insert("modelSlots".into(), Value::Object(Default::default()));
    yaml.insert("steps".into(), Value::Array(Vec::new()));
    yaml.insert("outputs".into(), Value::Object(Default::default()));
    let mut stages = Vec::new();
    let mut taken: Vec<String> = Vec::new();
    let mut custom = 0usize;
    for st in answer.stages {
        // The model often names the ready-made stage it means in the why, "(watch_pages)",
        // instead of writing `use`. Take the hint.
        let hinted = st.use_.clone().filter(|u| lib.get(u).is_some()).or_else(|| {
            let text = format!("{} {}", st.name.clone().unwrap_or_default(), st.why.clone().unwrap_or_default());
            lib.stages.iter().map(|d| d.id.clone()).find(|id| text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).any(|w| w == id))
        });
        match hinted.as_deref() {
            Some(u) if taken.iter().any(|t| t == u) => continue, // a ready-made stage once; a repeat is a slip
            Some(u) => {
                let mut o = serde_json::Map::new();
                o.insert("use".into(), Value::String(u.into()));
                if let Some(n) = st.name.filter(|n| !n.trim().is_empty()) {
                    o.insert("name".into(), Value::String(n));
                }
                if let Some(w) = st.why.filter(|w| !w.trim().is_empty()) {
                    o.insert("why".into(), Value::String(w));
                }
                taken.push(u.to_string());
                stages.push(Value::Object(o));
            }
            None => {
                let name = st.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Make".into());
                let id = st.id.filter(|i| !i.trim().is_empty() && !taken.contains(i)).unwrap_or_else(|| slug_id(&name, &taken));
                taken.push(id.clone());
                let mut o = serde_json::Map::new();
                o.insert("id".into(), Value::String(id));
                o.insert("name".into(), Value::String(name));
                if let Some(w) = st.why {
                    o.insert("why".into(), Value::String(w));
                }
                custom += 1;
                stages.push(Value::Object(o));
            }
        }
    }
    yaml.insert("stages".into(), Value::Array(stages));
    let mut draft = Draft { yaml: Value::Object(yaml), files: BTreeMap::new() };
    let _ = lib.expand_all(&mut draft);
    Ok((draft, custom))
}

/// The composing answer folded into the skeleton (when there is one) and every `use` expanded.
/// Returns the draft and the problems the merge itself found (an unknown ready-made stage);
/// the caller checks the draft. This is the planner with the model calls taken out, so recorded
/// answers can be replayed in tests.
pub fn compose(skeleton: Option<&Draft>, content: &str, req: &PlanRequest, lib: Option<&crate::stagelib::StageLibrary>) -> Result<(Draft, Vec<String>), String> {
    let mut draft = parse_answer(content, req)?;
    if let (Some(sk), Some(lib)) = (skeleton, lib) {
        let mut merged = sk.clone();
        merge_partial(&mut merged, &draft, lib);
        draft = merged;
    }
    let problems = lib.map(|l| l.expand_all(&mut draft)).unwrap_or_default();
    Ok((draft, problems))
}

/// What a plan makes from recorded answers, with no model call: the strip answer (none for a
/// single-call plan) and the composing answer (none when every stage was ready-made).
pub fn draft_from_answers(req: &PlanRequest, strip: Option<&str>, answer: Option<&str>, lib: Option<&crate::stagelib::StageLibrary>) -> Result<(Draft, Vec<String>), String> {
    let skeleton = match (strip, lib) {
        (Some(s), Some(lib)) => Some(skeleton_from_strip(s, req, lib)?),
        (Some(_), None) => return Err("a strip answer needs the stage library".into()),
        (None, _) => None,
    };
    match (skeleton, answer) {
        (Some((draft, 0)), None) => Ok((draft, Vec::new())),
        (Some((_, n)), None) => Err(format!("{n} custom stage(s) and no composing answer")),
        (Some((sk, _)), Some(a)) => compose(Some(&sk), a, req, lib),
        (None, Some(a)) => compose(None, a, req, lib),
        (None, None) => Err("no answers".into()),
    }
}

impl Planner {
    async fn call(&self, req: ChatRequest) -> Result<crate::model::ChatResponse, PlanError> {
        let timed = async {
            match self.timeout {
                Some(d) => match tokio::time::timeout(d, self.models.chat(&self.model_id, req)).await {
                    Ok(r) => r.map_err(|e| PlanError::Model(e.to_string())),
                    Err(_) => Err(PlanError::Timeout(d.as_secs())),
                },
                None => self.models.chat(&self.model_id, req).await.map_err(|e| PlanError::Model(e.to_string())),
            }
        };
        match &self.cancel {
            Some(c) => tokio::select! {
                r = timed => r,
                _ = c.cancelled() => Err(PlanError::Cancelled),
            },
            None => timed.await,
        }
    }

    fn chat(&self, messages: Vec<ChatMessage>, schema: Value) -> ChatRequest {
        ChatRequest {
            messages,
            tools: Vec::new(),
            json_schema: Some(schema),
            temperature: Some(0.2),
            max_tokens: None,
            context_length: self.model_defaults.context_length.or(Some(32768)),
            think: self.model_defaults.think.clone(),
        }
    }

    /// The first call: pick the strip. Returns the skeleton draft (ready-made stages expanded,
    /// custom ones named with their why and no steps yet) and how many custom stages there are.
    async fn pick_strip(&self, req: &PlanRequest, lib: &crate::stagelib::StageLibrary) -> Result<(Draft, usize, u64, u64), PlanError> {
        let system = format!("{}\n\n## Ready-made stages\n{}", self.strip_prompt.trim_end(), lib.describe());
        let resp = self.call(self.chat(vec![ChatMessage::new("system", system), ChatMessage::new("user", user_message(req))], strip_schema(lib))).await?;
        let (draft, custom) = skeleton_from_strip(&resp.content, req, lib).map_err(PlanError::Unusable)?;
        Ok((draft, custom, resp.prompt_tokens.unwrap_or(0), resp.completion_tokens.unwrap_or(0)))
    }

    pub async fn plan(&self, req: &PlanRequest, observer: Option<PlanObserver>) -> Result<PlanOutcome, PlanError> {
        let started = std::time::Instant::now();
        let known_tools: Vec<String> = self.tools.iter().map(|t| t.id.clone()).collect();
        let emit = |e: PlanEvent| {
            if let Some(o) = &observer {
                o(e);
            }
        };
        let (mut prompt_tokens, mut completion_tokens) = (0u64, 0u64);
        let lib = self.library.as_deref().filter(|l| !l.stages.is_empty());

        // First call, when there is a library: the strip. Ready-made stages come fully built;
        // custom ones are named with their why. All ready-made: nothing left to compose.
        let mut skeleton: Option<Draft> = None;
        if let Some(lib) = lib {
            emit(PlanEvent::Choosing);
            match self.pick_strip(req, lib).await {
                Ok((draft, custom, pt, ct)) => {
                    prompt_tokens += pt;
                    completion_tokens += ct;
                    if custom == 0 && draft.yaml.get("stages").and_then(Value::as_array).is_some_and(|a| !a.is_empty()) {
                        let problems = draft.problems(&known_tools);
                        return Ok(PlanOutcome { draft, problems, attempts: 1, model_id: self.model_id.clone(), prompt_tokens, completion_tokens, duration_ms: started.elapsed().as_millis() as u64 });
                    }
                    if draft.yaml.get("stages").and_then(Value::as_array).is_some_and(|a| !a.is_empty()) {
                        skeleton = Some(draft);
                    }
                }
                Err(PlanError::Cancelled) => return Err(PlanError::Cancelled),
                Err(PlanError::Timeout(s)) => return Err(PlanError::Timeout(s)),
                Err(_) => {} // fall back to composing everything in one call
            }
        }

        // The composing call. With a skeleton, the model writes only the custom stages and may
        // reference what the ready-made ones return.
        let mut system = assemble_system_with(&self.prompt, &self.tools, if skeleton.is_some() { None } else { lib });
        let mut user = user_message(req);
        if let (Some(sk), Some(lib)) = (&skeleton, lib) {
            system.push_str("\n\n## Already built\n\nThe recipe already contains these stages, in this order, fully built with their inputs, model slots, steps, prompts, and outputs. Do not write them again, do not list their inputs in your stages, and reference only what they return, exactly as written (a stage id is not a step id).\n");
            system.push_str(&already_built(sk, lib));
            let custom: Vec<String> = sk
                .yaml
                .get("stages")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter(|s| s.get("id").and_then(Value::as_str).is_some_and(|id| lib.get(id).is_none())).map(|s| format!("- `{}` **{}**{}", s.get("id").and_then(Value::as_str).unwrap_or(""), s.get("name").and_then(Value::as_str).unwrap_or(""), s.get("why").and_then(Value::as_str).map(|w| format!(": {w}")).unwrap_or_default())).collect())
                .unwrap_or_default();
            user.push_str(&format!(
                "\n## Your part\n\nWrite only these stages, keeping their ids and names:\n{}\n\nAnswer with the JSON object: the inputs, model slots, steps, outputs, and files they need (reuse inputs and slots the built stages already declare rather than adding more), and `stages` containing only these stages, each with its id, name, why, and the inputs, steps, and outputs it holds.\n",
                custom.join("\n")
            ));
        }
        let mut messages = vec![ChatMessage::new("system", system), ChatMessage::new("user", user)];
        let max = self.max_attempts.max(1);
        let mut best: Option<(Draft, Vec<String>)> = None;
        let mut last_error = String::new();
        let mut attempts = 0;
        for attempt in 1..=max {
            attempts = attempt;
            emit(PlanEvent::Thinking { attempt, of: max });
            let resp = self.call(self.chat(messages.clone(), answer_schema())).await?;
            prompt_tokens += resp.prompt_tokens.unwrap_or(0);
            completion_tokens += resp.completion_tokens.unwrap_or(0);
            emit(PlanEvent::Checking { attempt });
            let problems = match compose(skeleton.as_ref(), &resp.content, req, lib) {
                Ok((draft, mut problems)) => {
                    problems.extend(draft.problems(&known_tools));
                    let better = best.as_ref().map(|(_, p)| problems.len() < p.len()).unwrap_or(true);
                    if better {
                        best = Some((draft, problems.clone()));
                    }
                    problems
                }
                Err(e) => {
                    last_error = e.clone();
                    vec![format!("The answer was not the JSON object asked for: {e}")]
                }
            };
            if problems.is_empty() {
                break;
            }
            if attempt < max {
                emit(PlanEvent::Fixing { attempt, problems: problems.clone() });
                messages.push(ChatMessage::new("assistant", resp.content));
                let list = problems.iter().map(|p| format!("- {p}")).collect::<Vec<_>>().join("\n");
                messages.push(ChatMessage::new(
                    "user",
                    format!("The draft has these problems:\n{list}\n\nFix them and answer again with the whole corrected JSON object (recipe and every file)."),
                ));
            }
        }
        let (draft, problems) = best.ok_or(PlanError::Unusable(last_error))?;
        Ok(PlanOutcome {
            draft,
            problems,
            attempts: attempts + if skeleton.is_some() { 1 } else { 0 },
            model_id: self.model_id.clone(),
            prompt_tokens,
            completion_tokens,
            duration_ms: started.elapsed().as_millis() as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChatResponse, ModelError};
    use crate::tools::ToolRegistry;
    use std::sync::Mutex;

    struct Script {
        answers: Mutex<Vec<String>>,
        seen: Mutex<Vec<ChatRequest>>,
    }

    #[async_trait::async_trait]
    impl ModelClient for Script {
        async fn chat(&self, _model_id: &str, req: ChatRequest) -> Result<ChatResponse, ModelError> {
            self.seen.lock().unwrap().push(req);
            let mut a = self.answers.lock().unwrap();
            let content = if a.is_empty() { "{}".to_string() } else { a.remove(0) };
            Ok(ChatResponse { content, tool_calls: vec![], prompt_tokens: Some(10), completion_tokens: Some(5) })
        }
    }

    fn req() -> PlanRequest {
        PlanRequest { goal: "Summarize a file".into(), material: None, id: "summarize-2".into(), name: "Summarize 2".into(), icon: Some("notes".into()) }
    }

    fn answer(tool: &str) -> String {
        json!({
            "recipe": {
                "id": "ignored", "name": "Ignored",
                "description": "Reads a file and summarizes it.",
                "inputs": { "file": { "type": "file", "label": "File" } },
                "modelSlots": { "writer": { "requires": ["text"] } },
                "permissions": { "filesystem": { "read": ["input"] } },
                "steps": [
                    { "id": "read", "kind": "tool", "tool": tool, "inputs": { "path": "${inputs.file}" } },
                    { "id": "sum", "kind": "model", "slot": "writer", "prompt": "prompts/sum.md", "inputs": { "text": "${steps.read.output.text}" } }
                ],
                "outputs": { "summary": { "from": "${steps.sum.output}", "type": "markdown" } }
            },
            "files": [ { "path": "prompts/sum.md", "text": "Summarize." }, { "path": "../evil.md", "text": "no" } ]
        })
        .to_string()
    }

    fn planner(answers: Vec<String>) -> (Planner, Arc<Script>) {
        let script = Arc::new(Script { answers: Mutex::new(answers), seen: Mutex::new(vec![]) });
        let p = Planner {
            library: None,
            strip_prompt: crate::prompts::STRIP_PROMPT.into(),
            models: script.clone(),
            model_id: "ollama:test".into(),
            tools: ToolRegistry::builtin().catalog(),
            prompt: crate::prompts::PLANNER_PROMPT.into(),
            timeout: None,
            cancel: None,
            model_defaults: ModelDefaults::default(),
            max_attempts: 3,
        };
        (p, script)
    }

    #[test]
    fn parse_keeps_identity_and_safe_paths() {
        let d = parse_answer(&format!("Here you go:\n```json\n{}\n```", answer("file.read")), &req()).unwrap();
        assert_eq!(d.yaml["id"], "summarize-2");
        assert_eq!(d.yaml["name"], "Summarize 2");
        assert_eq!(d.yaml["icon"], "notes");
        assert_eq!(d.yaml["schemaVersion"], "0");
        assert_eq!(d.files.keys().collect::<Vec<_>>(), vec!["prompts/sum.md"]);
        assert!(parse_answer("nothing here", &req()).is_err());
    }

    #[test]
    fn a_stage_that_writes_a_keep_gets_one_declared() {
        let content = json!({
            "recipe": {
                "id": "x", "name": "X", "description": "d",
                "inputs": { "urls": { "type": "urls" } },
                "modelSlots": {},
                "steps": [
                    { "id": "fetch", "kind": "map", "over": "${inputs.urls}", "steps": [ { "id": "get", "kind": "tool", "tool": "http.fetch", "inputs": { "url": "${item}" } } ] },
                    { "id": "log", "kind": "tool", "tool": "keep.put", "inputs": { "data": "${steps.fetch.output}" } }
                ],
                "outputs": { "all": { "from": "${steps.log.output}", "type": "json" } },
                "stages": [
                    { "id": "read", "name": "Read", "why": "w", "inputs": ["urls"], "steps": ["fetch"] },
                    { "id": "remember", "name": "Remember", "why": "w", "steps": ["log"], "outputs": ["all"] }
                ]
            },
            "files": []
        })
        .to_string();
        let d = parse_answer(&content, &req()).unwrap();
        assert_eq!(d.yaml["stages"][0].get("keep"), None);
        assert_eq!(d.yaml["stages"][1]["keep"]["refresh"], "always");
        let known: Vec<String> = ToolRegistry::builtin().catalog().into_iter().map(|t| t.id).collect();
        assert!(d.check(&known).is_empty(), "{:?}", d.check(&known));
        assert!(d.problems(&known).is_empty());
    }

    #[tokio::test]
    async fn a_ready_made_stage_is_expanded_into_the_draft() {
        let lib = Arc::new(crate::stagelib::StageLibrary::load(&crate::stagelib::repo_stages_dir()).unwrap());
        let answer = json!({
            "recipe": {
                "id": "x", "name": "X", "description": "Summarize then make cards.",
                "inputs": {}, "modelSlots": {},
                "steps": [
                    { "id": "cards", "kind": "model", "slot": "writer", "prompt": "prompts/cards.md", "inputs": { "text": "${steps.summary_write.output}" } }
                ],
                "outputs": { "cards": { "from": "${steps.cards.output}", "type": "markdown" } },
                "stages": [
                    { "use": "summarize_file", "name": "Read it" },
                    { "id": "cards", "name": "Cards", "why": "w", "steps": ["cards"], "outputs": ["cards"] }
                ]
            },
            "files": [ { "path": "prompts/cards.md", "text": "Make cards." } ]
        })
        .to_string();
        let strip = json!({ "stages": [ { "use": "summarize_file", "name": "Read it" }, { "name": "Cards", "why": "Cards from the summary." } ] }).to_string();
        let (mut p, script) = planner(vec![strip, answer]);
        p.library = Some(lib.clone());
        let out = p.plan(&req(), None).await.unwrap();
        assert!(out.problems.is_empty(), "{:?}", out.problems);
        assert_eq!(out.attempts, 2, "one strip call, one composing call");
        let y = &out.draft.yaml;
        assert_eq!(y["stages"][0]["id"], "summarize_file");
        assert_eq!(y["stages"][0]["name"], "Read it");
        assert_eq!(y["stages"][0]["steps"], json!(["summary_read", "summary_write"]));
        assert_eq!(y["stages"][1]["id"], "cards");
        assert_eq!(y["stages"][1]["steps"], json!(["cards"]));
        assert_eq!(y["stages"][1]["why"], "w", "the composing answer's why wins");
        assert!(y["inputs"].get("file").is_some(), "the stage brought its input");
        assert!(y["modelSlots"].get("writer").is_some());
        assert!(out.draft.files.contains_key("prompts/summarize_file.md"));
        assert!(out.draft.files.contains_key("prompts/cards.md"));
        let seen = script.seen.lock().unwrap();
        assert!(seen[0].json_schema.as_ref().unwrap()["properties"]["stages"]["items"]["properties"]["use"]["enum"].as_array().unwrap().len() >= 10);
        assert!(seen[1].messages[0].content.contains("## Already built") && seen[1].messages[1].content.contains("`cards` **Cards**"));

        // A custom stage that re-lists a library input, references the stage id as a step, and a
        // brief that reads only the sibling that exists: all three are mended in the merge.
        let strip = json!({ "stages": [ { "use": "read_links" }, { "use": "write_brief" }, { "name": "Ideas", "why": "w" } ] }).to_string();
        let answer = json!({
            "recipe": {
                "id": "x", "name": "X", "description": "d",
                "inputs": { "urls": { "type": "urls" } }, "modelSlots": {},
                "steps": [ { "id": "ideas", "kind": "model", "slot": "writer", "prompt": "prompts/ideas.md", "inputs": { "pages": "${steps.read_links.output}", "brief": "${steps.brief_write.output}" } } ],
                "outputs": { "ideas": { "from": "${steps.ideas.output}", "type": "markdown" } },
                "stages": [ { "id": "ideas", "name": "Ideas", "why": "w", "inputs": ["urls"], "steps": ["ideas"], "outputs": ["ideas"] } ]
            },
            "files": [ { "path": "prompts/ideas.md", "text": "Ideas." } ]
        })
        .to_string();
        let (mut p, _) = planner(vec![strip, answer]);
        p.library = Some(lib.clone());
        let out = p.plan(&req(), None).await.unwrap();
        assert!(out.problems.is_empty(), "{:?}", out.problems);
        let y = &out.draft.yaml;
        assert_eq!(y["stages"][2]["inputs"], json!([]), "urls stays with read_links");
        assert_eq!(y["steps"].as_array().unwrap().iter().find(|s| s["id"] == "ideas").unwrap()["inputs"]["pages"], "${steps.links_pages.output}");
        let brief = y["steps"].as_array().unwrap().iter().find(|s| s["id"] == "brief_write").unwrap();
        assert!(brief["inputs"].get("files").is_none() && brief["inputs"].get("pages").is_some(), "{brief}");

        // A custom stage whose why names a ready-made stage is taken as that stage.
        let strip = json!({ "stages": [ { "name": "Read the pages", "why": "Fetch each link and extract findings (read_links)." }, { "name": "Brief", "why": "One brief, cited (write_brief)." } ] }).to_string();
        let (mut p, _) = planner(vec![strip]);
        p.library = Some(lib.clone());
        let out = p.plan(&req(), None).await.unwrap();
        assert!(out.problems.is_empty(), "{:?}", out.problems);
        assert_eq!(out.attempts, 1);
        assert_eq!(out.draft.yaml["stages"][0]["id"], "read_links");
        assert_eq!(out.draft.yaml["stages"][1]["id"], "write_brief");
        assert_eq!(out.draft.yaml["stages"][0]["name"], "Read the pages", "the model's name stays");
        let strip = json!({ "stages": [ { "use": "read_links" }, { "name": "Again", "why": "the same (read_links)" }, { "use": "write_brief" } ] }).to_string();
        let (mut p, _) = planner(vec![strip]);
        p.library = Some(lib.clone());
        let out = p.plan(&req(), None).await.unwrap();
        assert_eq!(out.draft.yaml["stages"].as_array().unwrap().len(), 2, "a repeated ready-made stage is dropped");

        // All ready-made: one call, nothing to compose.
        let strip = json!({ "stages": [ { "use": "index_folder" }, { "use": "answer_from_index" } ] }).to_string();
        let (mut p, _) = planner(vec![strip]);
        p.library = Some(lib.clone());
        let out = p.plan(&req(), None).await.unwrap();
        assert!(out.problems.is_empty(), "{:?}", out.problems);
        assert_eq!(out.attempts, 1);
        assert_eq!(out.draft.yaml["stages"].as_array().unwrap().len(), 2);
        assert!(out.draft.yaml["outputs"].get("answer").is_some());
        let s = assemble_system_with("P", &ToolRegistry::builtin().catalog(), Some(&lib));
        assert!(s.contains("- `summarize_file`:") && s.contains("use after `index_folder`"));
    }

    #[test]
    fn system_prompt_lists_every_tool() {
        let tools = ToolRegistry::builtin().catalog();
        let s = assemble_system("PROMPT", &tools);
        for t in &tools {
            assert!(s.contains(&format!("`{}`", t.id)), "{} missing", t.id);
        }
        assert!(s.starts_with("PROMPT"));
    }

    #[tokio::test]
    async fn fixes_problems_the_checker_reports() {
        let (p, script) = planner(vec![answer("file.readd"), answer("file.read")]);
        let out = p.plan(&req(), None).await.unwrap();
        assert!(out.problems.is_empty(), "{:?}", out.problems);
        assert_eq!(out.attempts, 2);
        let seen = script.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        let fix = &seen[1].messages.last().unwrap().content;
        assert!(fix.contains("file.readd"), "the problem goes back to the model: {fix}");
        assert!(seen[0].json_schema.is_some());
        assert_eq!(out.prompt_tokens, 20);
    }

    #[tokio::test]
    async fn gives_up_after_max_attempts_with_the_best_draft() {
        let (p, _) = planner(vec![answer("nope.a"), answer("nope.b"), answer("nope.c")]);
        let out = p.plan(&req(), None).await.unwrap();
        assert_eq!(out.attempts, 3);
        assert_eq!(out.problems.len(), 1);
        let (p, _) = planner(vec!["not json".into(), "still not".into(), "no".into()]);
        assert!(matches!(p.plan(&req(), None).await, Err(PlanError::Unusable(_))));
    }
}
