//! The planner: a goal in words becomes a draft recipe. The model proposes inputs, steps, prompts,
//! and outputs as one JSON object; the draft checker says what is wrong; the model gets up to two
//! more tries with those problems. The planner never picks a model or provider for the recipe it
//! writes, and it never writes to disk: the user saves the draft from the recipe card.

use crate::draft::Draft;
use crate::executor::{Cancel, ModelDefaults};
use crate::model::{ChatMessage, ChatRequest, ModelClient};
use crate::recipe::{Capability, ModelSlot, Recipe};
use crate::tools::ToolInfo;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

/// The slot the planner itself runs in. One slot, resolved like any recipe slot: the user's pin
/// first, then their default, then Auto. Auto for this slot is not "largest" but
/// [`crate::resolve::rank_for_planning`]: a measured clean plan first, then what fits in memory,
/// then a family rank for drafting in JSON, then size.
pub const PLANNER_SLOT: &str = "planner";
/// The id of the one-slot recipe the planner is resolved through; the resolver keys on it.
pub const PLANNER_RECIPE_ID: &str = "__planner";

pub fn planner_slot() -> ModelSlot {
    ModelSlot {
        requires: vec![Capability::Text, Capability::StructuredOutput],
        preference: None,
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
        id: PLANNER_RECIPE_ID.into(),
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
        input_groups: Vec::new(),
        author: None,
        license: None,
        runtime: None,
        tags: Vec::new(),
        tests: Vec::new(),
        extra: IndexMap::new(),
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

/// Which model call the planner is making.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanPhase {
    /// The short first call that only picks the stages.
    Strip,
    /// The composing call that writes the steps.
    Steps,
    /// A round that fixes the checker's problems.
    Fix,
}

impl PlanPhase {
    fn as_str(self) -> &'static str {
        match self {
            PlanPhase::Strip => "strip",
            PlanPhase::Steps => "steps",
            PlanPhase::Fix => "fix",
        }
    }
}

/// Progress while a plan is drafted. Every event carries `elapsed_ms` since the plan started and
/// `tokens`, the completion tokens written so far across calls (counted as they stream, then
/// settled from the provider's count when a call ends).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum PlanEvent {
    /// The first call: choosing the stages.
    Choosing { elapsed_ms: u64, tokens: u64 },
    /// A model call is in flight.
    Thinking { attempt: u32, of: u32, elapsed_ms: u64, tokens: u64 },
    /// The answer is being checked.
    Checking { attempt: u32, elapsed_ms: u64, tokens: u64 },
    /// The checker found problems; the model is asked to fix them.
    Fixing { attempt: u32, problems: Vec<String>, elapsed_ms: u64, tokens: u64 },
    /// A model call starts.
    Phase { which: PlanPhase, attempt: u32, of: u32, elapsed_ms: u64, tokens: u64 },
    /// Where the model call is before and while text arrives: loading the model into memory,
    /// reading the prompt, writing.
    Model { phase: crate::model::CallPhase, elapsed_ms: u64, tokens: u64 },
    /// A piece of what the model is writing. Chunks are coalesced: at most a few a second.
    Delta { text: String, elapsed_ms: u64, tokens: u64 },
    /// The last model call ended; only the final check remains.
    Done { elapsed_ms: u64, tokens: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanOutcome {
    pub draft: Draft,
    /// Problems the checker still finds. Empty means the draft can be saved as it is.
    pub problems: Vec<String>,
    /// The recipe format version the answer was written against.
    pub schema_version: String,
    pub attempts: u32,
    pub model_id: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub duration_ms: u64,
    /// Everything the model wrote, every call, with a `── call N (strip|steps|fix) ──` line before each.
    #[serde(default)]
    pub transcript: String,
    /// The custom stages were composed one call each (a small or tight model).
    #[serde(default)]
    pub per_stage: bool,
}

/// Collects what the model writes and tells the observer, throttled: a Delta goes out every
/// 200 ms or 400 characters, whichever comes first, and what is left when a call ends.
struct Progress {
    started: std::time::Instant,
    observer: Option<PlanObserver>,
    inner: std::sync::Mutex<ProgressInner>,
}

struct ProgressInner {
    /// Completion tokens of the calls that have ended.
    tokens_done: u64,
    /// Deltas seen in the call in flight: one per streamed line, about one token each.
    tokens_now: u64,
    pending: String,
    last_flush: std::time::Instant,
    transcript: String,
    calls: u32,
}

const DELTA_EVERY: Duration = Duration::from_millis(200);
const DELTA_CHARS: usize = 400;

impl Progress {
    fn new(observer: Option<PlanObserver>) -> Self {
        let now = std::time::Instant::now();
        Self {
            started: now,
            observer,
            inner: std::sync::Mutex::new(ProgressInner { tokens_done: 0, tokens_now: 0, pending: String::new(), last_flush: now, transcript: String::new(), calls: 0 }),
        }
    }

    fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ProgressInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Emit an event, filling in the elapsed time and the tokens so far.
    fn emit(&self, make: impl FnOnce(u64, u64) -> PlanEvent) {
        if let Some(o) = &self.observer {
            let tokens = {
                let g = self.lock();
                g.tokens_done + g.tokens_now
            };
            o(make(self.elapsed_ms(), tokens));
        }
    }

    /// A model call starts: a separator in the transcript, then the Phase event.
    fn begin_call(&self, which: PlanPhase, attempt: u32, of: u32) {
        {
            let mut g = self.lock();
            g.calls += 1;
            if !g.transcript.is_empty() {
                g.transcript.push_str("\n\n");
            }
            let header = format!("── call {} ({}) ──\n", g.calls, which.as_str());
            g.transcript.push_str(&header);
            g.tokens_now = 0;
            g.pending.clear();
            g.last_flush = std::time::Instant::now();
        }
        self.emit(|elapsed_ms, tokens| PlanEvent::Phase { which, attempt, of, elapsed_ms, tokens });
    }

    /// The model call moved: loading, reading, or writing.
    fn model_phase(&self, phase: crate::model::CallPhase) {
        self.emit(|elapsed_ms, tokens| PlanEvent::Model { phase, elapsed_ms, tokens });
    }

    /// A piece of text from the model.
    fn delta(&self, text: &str) {
        let flush = {
            let mut g = self.lock();
            g.transcript.push_str(text);
            g.pending.push_str(text);
            g.tokens_now += 1;
            g.pending.len() >= DELTA_CHARS || g.last_flush.elapsed() >= DELTA_EVERY
        };
        if flush {
            self.flush();
        }
    }

    fn flush(&self) {
        let text = {
            let mut g = self.lock();
            g.last_flush = std::time::Instant::now();
            std::mem::take(&mut g.pending)
        };
        if !text.is_empty() {
            self.emit(|elapsed_ms, tokens| PlanEvent::Delta { text, elapsed_ms, tokens });
        }
    }

    /// A model call ended: the rest of the text goes out and the provider's count replaces the estimate.
    fn end_call(&self, resp: &crate::model::ChatResponse) {
        self.flush();
        let mut g = self.lock();
        let counted = std::mem::take(&mut g.tokens_now);
        g.tokens_done += resp.completion_tokens.unwrap_or(counted);
    }

    fn done(&self) {
        self.emit(|elapsed_ms, tokens| PlanEvent::Done { elapsed_ms, tokens });
    }

    fn transcript(&self) -> String {
        self.lock().transcript.clone()
    }
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
/// the draft checker afterwards, and its problems go back to the model in words. The property
/// lists of a step and a stage come from the recipe schema's `$defs`, so a field added there
/// is offered to the model without anyone copying its name here; the constrained enums (step
/// kind, keep refresh) are kept as the schema has them.
pub fn answer_schema() -> Value {
    let schema: Value = serde_json::from_str(crate::recipe::RECIPE_SCHEMA).unwrap_or(Value::Null);
    let defs = &schema["$defs"];
    let mut step_props = serde_json::Map::new();
    let mut add = |props: Option<&Value>| {
        for (k, v) in props.and_then(Value::as_object).into_iter().flatten() {
            let keep_as_is = k == "kind" && v.get("enum").is_some();
            step_props.entry(k.clone()).or_insert_with(|| if keep_as_is { v.clone() } else { json!({}) });
        }
    };
    // Property order matters: a constrained decoder (Ollama's) makes the model write an object's
    // properties in the order the schema lists them, so once `tool` is written, every property
    // listed before it is out of reach. The keys that say what a step is come first (id, kind,
    // name, description, then tool, slot, prompt, over, items, …), `inputs` and the rest of the
    // common keys after them, and `params` last, since a model step writes it after its inputs.
    let base = defs["stepBase"].get("properties").cloned().unwrap_or(Value::Null);
    for k in ["id", "kind", "name", "description"] {
        add(Some(&json!({ k: base[k] })));
    }
    for kind in ["toolStep", "modelStep", "agentStep", "recipeStep", "branchStep", "mapStep", "reviewStep"] {
        for part in defs[kind]["allOf"].as_array().into_iter().flatten() {
            if let Some(props) = part.get("properties").and_then(Value::as_object) {
                let without_params: serde_json::Map<String, Value> = props.iter().filter(|(k, _)| k.as_str() != "params").map(|(k, v)| (k.clone(), v.clone())).collect();
                add(Some(&Value::Object(without_params)));
            }
        }
    }
    add(defs["stepBase"].get("properties"));
    add(Some(&json!({ "params": {} })));
    // The same for a stage: `use` first, so `{ "use": "<id>", "name": …, "why": … }` can be written.
    let mut stage_props = serde_json::Map::new();
    stage_props.insert("use".into(), json!({ "type": "string", "description": "The id of a ready-made stage to use instead of steps" }));
    for (k, v) in defs["stage"]["properties"].as_object().into_iter().flatten() {
        let v = match k.as_str() {
            "keep" => json!({ "type": "object", "properties": { "refresh": v["properties"]["refresh"].clone() } }),
            "source" => continue, // provenance is written by the app when a ready-made stage is copied, never by the model
            "inputs" | "steps" | "outputs" => json!({ "type": "array", "items": { "type": "string" } }),
            _ => json!({ "type": "string" }),
        };
        stage_props.insert(k.clone(), v);
    }
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
                    "steps": { "type": "array", "items": { "type": "object", "properties": step_props } },
                    "outputs": { "type": "object" },
                    "stages": { "type": "array", "items": { "type": "object", "properties": stage_props } }
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
    /// The edit prompt in use: changing a recipe from an instruction (Ask for a change).
    pub edit_prompt: String,
    pub timeout: Option<Duration>,
    pub cancel: Option<Cancel>,
    pub model_defaults: ModelDefaults,
    /// Model calls at most for the composing call, counting the fixes. Default 3.
    pub max_attempts: u32,
    /// Compose one custom stage per call instead of all at once: for a small or tight model,
    /// every answer is short and every mistake local. The fix rounds still work on the whole.
    pub per_stage: bool,
}

pub type PlanObserver = Arc<dyn Fn(PlanEvent) + Send + Sync>;

/// The goal Try planning uses, the same every time so one model's number compares with another's.
pub const TRIAL_GOAL: &str = "Ask me for a folder of PDFs and a question each time, and answer from the documents with the page it came from.";

/// The request Try planning sends: [`TRIAL_GOAL`], no material.
pub fn trial_request() -> PlanRequest {
    PlanRequest { goal: TRIAL_GOAL.into(), material: None, id: "answer-from-pdfs".into(), name: "Answer from PDFs".into(), icon: None }
}

/// What Try planning measured: the strip call's time and whether its answer checked clean.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StripTrial {
    pub ms: u64,
    pub clean: bool,
    /// Why it was not clean, in the checker's words. Empty when clean.
    pub problems: Vec<String>,
}

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
    // Only for stages that are in this strip, and never when the answer has a step of that id:
    // a custom step may be called `write_brief` in a recipe that has no write_brief stage.
    let in_strip: Vec<String> = base
        .get("stages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|st| st.get("use").or_else(|| st.get("id")).and_then(Value::as_str))
        .filter(|id| lib.get(id).is_some())
        .map(String::from)
        .collect();
    let defined: Vec<String> = base.get("steps").and_then(Value::as_array).into_iter().flatten().filter_map(|s| s.get("id").and_then(Value::as_str)).map(String::from).collect();
    let aliases: Vec<(String, String)> = lib
        .stages
        .iter()
        .filter(|def| in_strip.contains(&def.id) && !defined.contains(&def.id))
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

/// A permission the planner wrote is kept only when it is `input`, `output`, or
/// `${inputs.<a declared input>}`: what the user hands the run. A literal path or host, cloud
/// models, running programs, or a credential is dropped and reported in words, so the fix loop
/// sees it and the user never has to notice. What a ready-made stage brought is not touched here:
/// it was reviewed with the stage, and it lives in the skeleton, not in the model's answer.
pub fn whitelist_permissions(yaml: &mut Value, declared_inputs: &[String]) -> Vec<String> {
    let mut problems = Vec::new();
    let Some(perms) = yaml.get_mut("permissions").and_then(Value::as_object_mut) else { return problems };
    let allowed = |entry: &Value| -> bool {
        match entry.as_str() {
            Some("input") | Some("output") => true,
            Some(s) => s.strip_prefix("${inputs.").and_then(|r| r.strip_suffix('}')).is_some_and(|name| declared_inputs.iter().any(|d| d == name)),
            None => false,
        }
    };
    let removed = |what: &str| format!("The draft asked to {what}, which a recipe may not do on its own. Removed.");
    if let Some(fs) = perms.get_mut("filesystem").and_then(Value::as_object_mut) {
        for (key, verb) in [("read", "read"), ("write", "write to")] {
            if let Some(list) = fs.get_mut(key).and_then(Value::as_array_mut) {
                list.retain(|e| {
                    let ok = allowed(e);
                    if !ok {
                        problems.push(removed(&format!("{verb} {}", e.as_str().map(String::from).unwrap_or_else(|| e.to_string()))));
                    }
                    ok
                });
            }
        }
        fs.retain(|_, v| !v.as_array().is_some_and(Vec::is_empty));
    }
    if let Some(list) = perms.get_mut("network").and_then(Value::as_array_mut) {
        list.retain(|e| {
            let ok = allowed(e);
            if !ok {
                problems.push(removed(&format!("reach {}", e.as_str().map(String::from).unwrap_or_else(|| e.to_string()))));
            }
            ok
        });
    }
    if perms.get("cloudModels").and_then(Value::as_bool) == Some(true) {
        problems.push(removed("use cloud models"));
    }
    if perms.get("shell").and_then(Value::as_bool) == Some(true) {
        problems.push(removed("run programs on this Mac (shell)"));
    }
    for c in perms.get("credentials").and_then(Value::as_array).into_iter().flatten() {
        problems.push(removed(&format!("use the credential {}", c.as_str().map(String::from).unwrap_or_else(|| c.to_string()))));
    }
    perms.remove("cloudModels");
    perms.remove("shell");
    perms.remove("credentials");
    perms.retain(|_, v| !(v.as_array().is_some_and(Vec::is_empty) || v.as_object().is_some_and(serde_json::Map::is_empty)));
    problems
}

/// The composing answer folded into the skeleton (when there is one) and every `use` expanded.
/// Returns the draft and the problems the merge itself found (a permission the planner may not
/// ask for, an unknown ready-made stage); the caller checks the draft. This is the planner with
/// the model calls taken out, so recorded answers can be replayed in tests.
pub fn compose(skeleton: Option<&Draft>, content: &str, req: &PlanRequest, lib: Option<&crate::stagelib::StageLibrary>) -> Result<(Draft, Vec<String>), String> {
    let mut draft = parse_answer(content, req)?;
    let declared: Vec<String> = [draft.yaml.get("inputs"), skeleton.and_then(|s| s.yaml.get("inputs"))]
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .flat_map(|o| o.keys().cloned())
        .collect();
    let mut problems = whitelist_permissions(&mut draft.yaml, &declared);
    if let (Some(sk), Some(lib)) = (skeleton, lib) {
        let mut merged = sk.clone();
        merge_partial(&mut merged, &draft, lib);
        draft = merged;
    }
    problems.extend(lib.map(|l| l.expand_all(&mut draft)).unwrap_or_default());
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

/// What Ask for a change hands back: the changed draft as a plan outcome, and the changes in words.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditOutcome {
    #[serde(flatten)]
    pub outcome: PlanOutcome,
    /// One line per change, for the person to read before saving.
    pub changes: Vec<String>,
}

/// The differences between two drafts, in words: stages, inputs, steps, outputs, slots,
/// permissions, and files. Empty means nothing changed.
pub fn changes(before: &Draft, after: &Draft) -> Vec<String> {
    let mut out = Vec::new();
    let keys = |d: &Draft, section: &str| -> Vec<String> { d.yaml.get(section).and_then(Value::as_object).map(|o| o.keys().cloned().collect()).unwrap_or_default() };
    let label = |d: &Draft, section: &str, k: &str| -> String {
        d.yaml.get(section).and_then(|s| s.get(k)).and_then(|v| v.get("label").or_else(|| v.get("name"))).and_then(Value::as_str).map(|l| format!("{l} (`{k}`)")).unwrap_or_else(|| format!("`{k}`"))
    };
    let word = |n: usize, one: &str, many: &str| if n == 1 { one.to_string() } else { many.to_string() };
    for (section, one, many) in [("inputs", "Input", "Inputs"), ("outputs", "Result", "Results"), ("modelSlots", "Model slot", "Model slots")] {
        let (b, a) = (keys(before, section), keys(after, section));
        let added: Vec<String> = a.iter().filter(|k| !b.contains(k)).map(|k| label(after, section, k)).collect();
        let removed: Vec<String> = b.iter().filter(|k| !a.contains(k)).map(|k| label(before, section, k)).collect();
        if !added.is_empty() {
            out.push(format!("{} added: {}.", word(added.len(), one, many), added.join(", ")));
        }
        if !removed.is_empty() {
            out.push(format!("{} removed: {}.", word(removed.len(), one, many), removed.join(", ")));
        }
    }
    // Steps, loops included, by id; a renamed or rewired step counts as changed.
    fn steps_of(v: &Value, out: &mut Vec<(String, Value)>) {
        for s in v.get("steps").and_then(Value::as_array).into_iter().flatten() {
            if let Some(id) = s.get("id").and_then(Value::as_str) {
                out.push((id.to_string(), s.clone()));
            }
            steps_of(s, out);
        }
    }
    let (mut b, mut a) = (Vec::new(), Vec::new());
    steps_of(&before.yaml, &mut b);
    steps_of(&after.yaml, &mut a);
    let name_of = |s: &Value, id: &str| s.get("name").and_then(Value::as_str).map(|n| format!("{n} (`{id}`)")).unwrap_or_else(|| format!("`{id}`"));
    let added: Vec<String> = a.iter().filter(|(id, _)| !b.iter().any(|(x, _)| x == id)).map(|(id, s)| name_of(s, id)).collect();
    let removed: Vec<String> = b.iter().filter(|(id, _)| !a.iter().any(|(x, _)| x == id)).map(|(id, s)| name_of(s, id)).collect();
    let changed: Vec<String> = a.iter().filter(|(id, s)| b.iter().any(|(x, t)| x == id && t != s)).map(|(id, s)| name_of(s, id)).collect();
    if !added.is_empty() {
        out.push(format!("{} added: {}.", word(added.len(), "Step", "Steps"), added.join(", ")));
    }
    if !removed.is_empty() {
        out.push(format!("{} removed: {}.", word(removed.len(), "Step", "Steps"), removed.join(", ")));
    }
    if !changed.is_empty() {
        out.push(format!("{} changed: {}.", word(changed.len(), "Step", "Steps"), changed.join(", ")));
    }
    // Stages by id.
    let stages = |d: &Draft| -> Vec<(String, Value)> { d.yaml.get("stages").and_then(Value::as_array).into_iter().flatten().filter_map(|s| s.get("id").and_then(Value::as_str).map(|id| (id.to_string(), s.clone()))).collect() };
    let (b, a) = (stages(before), stages(after));
    let added: Vec<String> = a.iter().filter(|(id, _)| !b.iter().any(|(x, _)| x == id)).map(|(id, s)| name_of(s, id)).collect();
    let removed: Vec<String> = b.iter().filter(|(id, _)| !a.iter().any(|(x, _)| x == id)).map(|(id, s)| name_of(s, id)).collect();
    let changed: Vec<String> = a.iter().filter(|(id, s)| b.iter().any(|(x, t)| x == id && t != s)).map(|(id, s)| name_of(s, id)).collect();
    if !added.is_empty() {
        out.push(format!("{} added: {}.", word(added.len(), "Stage", "Stages"), added.join(", ")));
    }
    if !removed.is_empty() {
        out.push(format!("{} removed: {}.", word(removed.len(), "Stage", "Stages"), removed.join(", ")));
    }
    if !changed.is_empty() {
        out.push(format!("{} changed: {}.", word(changed.len(), "Stage", "Stages"), changed.join(", ")));
    }
    if before.yaml.get("permissions") != after.yaml.get("permissions") {
        out.push("Permissions changed.".into());
    }
    for k in ["name", "description"] {
        if before.yaml.get(k) != after.yaml.get(k) {
            out.push(format!("The {k} changed."));
        }
    }
    let added: Vec<&String> = after.files.keys().filter(|k| !before.files.contains_key(*k)).collect();
    let removed: Vec<&String> = before.files.keys().filter(|k| !after.files.contains_key(*k)).collect();
    let rewritten: Vec<&String> = after.files.iter().filter(|(k, v)| before.files.get(*k).is_some_and(|t| t != *v)).map(|(k, _)| k).collect();
    for (list, what) in [(added, "added"), (removed, "removed"), (rewritten, "rewritten")] {
        if !list.is_empty() {
            out.push(format!("{} {what}: {}.", word(list.len(), "File", "Files"), list.iter().map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ")));
        }
    }
    out
}

impl Planner {
    /// Ask for a change: the recipe as it is and one instruction go to the model; the whole
    /// recipe with only that change comes back, checked and fixed like a new draft. The original
    /// is never touched here; the caller saves the result, as a copy or in place.
    pub async fn edit(&self, before: &Draft, instruction: &str, observer: Option<PlanObserver>) -> Result<EditOutcome, PlanError> {
        let progress = Progress::new(observer);
        let started = progress.started;
        let known_tools: Vec<String> = self.tools.iter().map(|t| t.id.clone()).collect();
        let lib = self.library.as_deref().filter(|l| !l.stages.is_empty());
        let req = PlanRequest {
            goal: instruction.trim().to_string(),
            material: None,
            id: before.id().unwrap_or("recipe").to_string(),
            name: before.yaml.get("name").and_then(Value::as_str).unwrap_or("Recipe").to_string(),
            icon: before.yaml.get("icon").and_then(Value::as_str).map(String::from),
        };
        let mut system = assemble_system_with(&self.prompt, &self.tools, lib);
        system.push_str("\n\n## Changing a recipe\n\n");
        system.push_str(self.edit_prompt.trim());
        let mut user = format!("## The recipe as it is\n\n```json\n{}\n```\n", serde_json::to_string_pretty(&before.yaml).unwrap_or_default());
        if !before.files.is_empty() {
            user.push_str("\n## Its files\n");
            for (path, text) in &before.files {
                user.push_str(&format!("\n### {path}\n\n{}\n", text.trim_end()));
            }
        }
        user.push_str(&format!("\n## The change asked for\n\n{}\n\nKeep the id `{}` and the name unless the change asks otherwise.\n", req.goal, req.id));
        let mut messages = vec![ChatMessage::new("system", system), ChatMessage::new("user", user)];
        let max = self.max_attempts.max(1);
        let (mut prompt_tokens, mut completion_tokens) = (0u64, 0u64);
        let mut best: Option<(Draft, Vec<String>)> = None;
        let mut last_error = String::new();
        let mut attempts = 0;
        for attempt in 1..=max {
            attempts = attempt;
            progress.emit(|elapsed_ms, tokens| PlanEvent::Thinking { attempt, of: max, elapsed_ms, tokens });
            let which = if attempt == 1 { PlanPhase::Steps } else { PlanPhase::Fix };
            let resp = self.call(self.chat(messages.clone(), answer_schema()), &progress, which, attempt, max).await?;
            prompt_tokens += resp.prompt_tokens.unwrap_or(0);
            completion_tokens += resp.completion_tokens.unwrap_or(0);
            if attempt == max {
                progress.done();
            }
            progress.emit(|elapsed_ms, tokens| PlanEvent::Checking { attempt, elapsed_ms, tokens });
            let problems = match compose(None, &resp.content, &req, lib) {
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
                if attempt < max {
                    progress.done();
                }
                break;
            }
            if attempt < max {
                progress.emit(|elapsed_ms, tokens| PlanEvent::Fixing { attempt, problems: problems.clone(), elapsed_ms, tokens });
                messages.push(ChatMessage::new("assistant", resp.content));
                let list = problems.iter().map(|p| format!("- {p}")).collect::<Vec<_>>().join("\n");
                messages.push(ChatMessage::new("user", format!("The changed recipe has these problems:\n{list}\n\nFix them and answer again with the whole corrected JSON object (recipe and every file).")));
            }
        }
        let (draft, problems) = best.ok_or(PlanError::Unusable(last_error))?;
        let changes = changes(before, &draft);
        Ok(EditOutcome {
            outcome: PlanOutcome {
                draft,
                problems,
                schema_version: crate::migrate::CURRENT_SCHEMA_VERSION.into(),
                attempts,
                model_id: self.model_id.clone(),
                prompt_tokens,
                completion_tokens,
                duration_ms: started.elapsed().as_millis() as u64,
                transcript: progress.transcript(),
                per_stage: false,
            },
            changes,
        })
    }

    /// One model call, streamed into `progress`: a Phase event first, Deltas as the text arrives,
    /// then the response. The timeout covers the whole call; Stop drops it mid-stream.
    async fn call(&self, req: ChatRequest, progress: &Progress, which: PlanPhase, attempt: u32, of: u32) -> Result<crate::model::ChatResponse, PlanError> {
        progress.begin_call(which, attempt, of);
        let on = |ev: crate::model::StreamEvent<'_>| match ev {
            crate::model::StreamEvent::Phase(p) => progress.model_phase(p),
            crate::model::StreamEvent::Text(text) => progress.delta(text),
        };
        let timed = async {
            match self.timeout {
                Some(d) => match tokio::time::timeout(d, self.models.chat_events(&self.model_id, req, &on)).await {
                    Ok(r) => r.map_err(|e| PlanError::Model(e.to_string())),
                    Err(_) => Err(PlanError::Timeout(d.as_secs())),
                },
                None => self.models.chat_events(&self.model_id, req, &on).await.map_err(|e| PlanError::Model(e.to_string())),
            }
        };
        let resp = match &self.cancel {
            Some(c) => tokio::select! {
                r = timed => r,
                _ = c.cancelled() => Err(PlanError::Cancelled),
            },
            None => timed.await,
        }?;
        progress.end_call(&resp);
        Ok(resp)
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
    async fn pick_strip(&self, req: &PlanRequest, lib: &crate::stagelib::StageLibrary, progress: &Progress) -> Result<(Draft, usize, u64, u64), PlanError> {
        let system = format!("{}\n\n## Ready-made stages\n{}", self.strip_prompt.trim_end(), lib.describe());
        let resp = self.call(self.chat(vec![ChatMessage::new("system", system), ChatMessage::new("user", user_message(req))], strip_schema(lib)), progress, PlanPhase::Strip, 1, 1).await?;
        let (draft, custom) = skeleton_from_strip(&resp.content, req, lib).map_err(PlanError::Unusable)?;
        Ok((draft, custom, resp.prompt_tokens.unwrap_or(0), resp.completion_tokens.unwrap_or(0)))
    }

    /// Try planning: the strip call only, timed and checked the way `plan` checks it. Clean means
    /// the model named at least one stage and, when every stage was ready-made, the draft checks
    /// clean; a strip with custom stages is not a whole recipe yet, so only the strip itself is
    /// judged. An answer that is not a strip at all comes back as not clean, not as an error; the
    /// model being unreachable, the time cap, and Stop are errors. The observer, if any, sees
    /// the same events a plan emits (Phase, Delta, Done).
    pub async fn try_strip(&self, req: &PlanRequest, observer: Option<PlanObserver>) -> Result<StripTrial, PlanError> {
        let lib = self
            .library
            .as_deref()
            .filter(|l| !l.stages.is_empty())
            .ok_or_else(|| PlanError::Unusable("there are no ready-made stages to choose from".into()))?;
        let progress = Progress::new(observer);
        let picked = self.pick_strip(req, lib, &progress).await;
        let ms = progress.elapsed_ms();
        let (draft, custom, _, _) = match picked {
            Ok(x) => x,
            Err(PlanError::Unusable(why)) => return Ok(StripTrial { ms, clean: false, problems: vec![format!("the answer was not a list of stages: {why}")] }),
            Err(e) => return Err(e),
        };
        let mut problems = Vec::new();
        let stages = draft.yaml.get("stages").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
        if stages == 0 {
            problems.push("the answer named no stages".to_string());
        } else if custom == 0 {
            let known_tools: Vec<String> = self.tools.iter().map(|t| t.id.clone()).collect();
            problems.extend(draft.problems(&known_tools));
        }
        Ok(StripTrial { ms, clean: problems.is_empty(), problems })
    }

    pub async fn plan(&self, req: &PlanRequest, observer: Option<PlanObserver>) -> Result<PlanOutcome, PlanError> {
        let progress = Progress::new(observer);
        let started = progress.started;
        let known_tools: Vec<String> = self.tools.iter().map(|t| t.id.clone()).collect();
        let (mut prompt_tokens, mut completion_tokens) = (0u64, 0u64);
        let lib = self.library.as_deref().filter(|l| !l.stages.is_empty());

        // First call, when there is a library: the strip. Ready-made stages come fully built;
        // custom ones are named with their why. All ready-made: nothing left to compose.
        let mut skeleton: Option<Draft> = None;
        if let Some(lib) = lib {
            progress.emit(|elapsed_ms, tokens| PlanEvent::Choosing { elapsed_ms, tokens });
            match self.pick_strip(req, lib, &progress).await {
                Ok((draft, custom, pt, ct)) => {
                    prompt_tokens += pt;
                    completion_tokens += ct;
                    if custom == 0 && draft.yaml.get("stages").and_then(Value::as_array).is_some_and(|a| !a.is_empty()) {
                        progress.done();
                        let problems = draft.problems(&known_tools);
                        return Ok(PlanOutcome {
                            draft,
                            problems,
                            schema_version: crate::migrate::CURRENT_SCHEMA_VERSION.into(),
                            attempts: 1,
                            model_id: self.model_id.clone(),
                            prompt_tokens,
                            completion_tokens,
                            duration_ms: started.elapsed().as_millis() as u64,
                            transcript: progress.transcript(),
                            per_stage: false,
                        });
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
        let mut messages = vec![ChatMessage::new("system", system.clone()), ChatMessage::new("user", user)];
        let max = self.max_attempts.max(1);
        let mut best: Option<(Draft, Vec<String>)> = None;
        let mut last_error = String::new();
        let mut attempts = 0;
        let mut first_attempt = 1;

        // Per stage: with a skeleton and more than one custom stage, a small or tight model writes
        // one stage per call, each merged before the next so the following call can read it. The
        // merged whole then stands as the first attempt; a fix round, if needed, works on all of it.
        let custom: Vec<(String, String, String)> = skeleton
            .as_ref()
            .zip(lib)
            .map(|(sk, lib)| {
                sk.yaml
                    .get("stages")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|s| s.get("id").and_then(Value::as_str).is_some_and(|id| lib.get(id).is_none()))
                    .map(|s| {
                        let f = |k: &str| s.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                        (f("id"), f("name"), f("why"))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let per_stage = self.per_stage && custom.len() > 1;
        if per_stage {
            let lib = lib.expect("a skeleton comes from a library");
            let mut sk = skeleton.clone().expect("custom stages come from a skeleton");
            let n = custom.len() as u32;
            let mut written: Vec<String> = Vec::new();
            let mut whole = true;
            for (i, (id, name, why)) in custom.iter().enumerate() {
                let mut sys = system.clone();
                if !written.is_empty() {
                    sys.push_str("\n\n## Already written by you\n\nThese custom stages are done. Reference their steps as `${steps.<id>.output}` and do not write them again:\n");
                    sys.push_str(&written.join("\n"));
                }
                let mut usr = user_message(req);
                usr.push_str(&format!(
                    "\n## Your part\n\nWrite only this one stage, keeping its id and name:\n- `{id}` **{name}**{}\n\nAnswer with the JSON object: the inputs, model slots, steps, outputs, and files this stage needs (reuse inputs and slots the built stages already declare rather than adding more), and `stages` containing only this stage, with its id, name, why, and the inputs, steps, and outputs it holds.\n",
                    if why.is_empty() { String::new() } else { format!(": {why}") }
                ));
                progress.emit(|elapsed_ms, tokens| PlanEvent::Thinking { attempt: 1, of: max, elapsed_ms, tokens });
                let resp = self.call(self.chat(vec![ChatMessage::new("system", sys), ChatMessage::new("user", usr)], answer_schema()), &progress, PlanPhase::Steps, i as u32 + 1, n).await?;
                prompt_tokens += resp.prompt_tokens.unwrap_or(0);
                completion_tokens += resp.completion_tokens.unwrap_or(0);
                match compose(Some(&sk), &resp.content, req, Some(lib)) {
                    Ok((draft, _)) => {
                        let steps = draft
                            .yaml
                            .get("stages")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .find(|s| s.get("id").and_then(Value::as_str) == Some(id.as_str()))
                            .and_then(|s| s.get("steps").and_then(Value::as_array))
                            .map(|a| a.iter().filter_map(Value::as_str).map(|s| format!("`{s}`")).collect::<Vec<_>>().join(", "))
                            .unwrap_or_default();
                        written.push(format!("- `{id}` **{name}**: steps {steps}"));
                        sk = draft;
                    }
                    Err(e) => {
                        last_error = e;
                        whole = false;
                        break;
                    }
                }
            }
            if whole {
                attempts = 1;
                progress.emit(|elapsed_ms, tokens| PlanEvent::Checking { attempt: 1, elapsed_ms, tokens });
                let problems = sk.problems(&known_tools);
                if problems.is_empty() || max == 1 {
                    progress.done();
                    return Ok(PlanOutcome {
                        draft: sk,
                        problems,
                        schema_version: crate::migrate::CURRENT_SCHEMA_VERSION.into(),
                        attempts: 2,
                        model_id: self.model_id.clone(),
                        prompt_tokens,
                        completion_tokens,
                        duration_ms: started.elapsed().as_millis() as u64,
                        transcript: progress.transcript(),
                        per_stage: true,
                    });
                }
                // The merged whole is what the model "answered"; the fix round corrects it as one.
                progress.emit(|elapsed_ms, tokens| PlanEvent::Fixing { attempt: 1, problems: problems.clone(), elapsed_ms, tokens });
                let files: Vec<Value> = sk.files.iter().map(|(p, t)| serde_json::json!({ "path": p, "text": t })).collect();
                let answered = serde_json::json!({ "recipe": sk.yaml, "files": files }).to_string();
                best = Some((sk, problems.clone()));
                messages.push(ChatMessage::new("assistant", answered));
                let list = problems.iter().map(|p| format!("- {p}")).collect::<Vec<_>>().join("\n");
                messages.push(ChatMessage::new("user", format!("The draft has these problems:\n{list}\n\nFix them and answer again with the whole corrected JSON object (recipe and every file).")));
                first_attempt = 2;
            }
        }

        for attempt in first_attempt..=max {
            attempts = attempt;
            progress.emit(|elapsed_ms, tokens| PlanEvent::Thinking { attempt, of: max, elapsed_ms, tokens });
            let which = if attempt == 1 { PlanPhase::Steps } else { PlanPhase::Fix };
            let resp = self.call(self.chat(messages.clone(), answer_schema()), &progress, which, attempt, max).await?;
            prompt_tokens += resp.prompt_tokens.unwrap_or(0);
            completion_tokens += resp.completion_tokens.unwrap_or(0);
            if attempt == max {
                progress.done();
            }
            progress.emit(|elapsed_ms, tokens| PlanEvent::Checking { attempt, elapsed_ms, tokens });
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
                if attempt < max {
                    progress.done();
                }
                break;
            }
            if attempt < max {
                progress.emit(|elapsed_ms, tokens| PlanEvent::Fixing { attempt, problems: problems.clone(), elapsed_ms, tokens });
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
            schema_version: crate::migrate::CURRENT_SCHEMA_VERSION.into(),
            attempts: attempts + if skeleton.is_some() { 1 } else { 0 },
            model_id: self.model_id.clone(),
            prompt_tokens,
            completion_tokens,
            duration_ms: started.elapsed().as_millis() as u64,
            transcript: progress.transcript(),
            per_stage,
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
            Ok(ChatResponse { content, tool_calls: vec![], prompt_tokens: Some(10), completion_tokens: Some(5), completion_tokens_per_second: None, load_ms: None, prompt_ms: None })
        }

        /// Streams the answer in three pieces, as a provider would.
        async fn chat_stream(&self, model_id: &str, req: ChatRequest, on_delta: &(dyn for<'a> Fn(&'a str) + Send + Sync)) -> Result<ChatResponse, ModelError> {
            let resp = self.chat(model_id, req).await?;
            let n = resp.content.len();
            let cuts = [0, n / 3, 2 * n / 3, n];
            for w in cuts.windows(2) {
                let piece = &resp.content[w[0]..w[1]];
                if !piece.is_empty() {
                    on_delta(piece);
                }
            }
            Ok(resp)
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
            edit_prompt: crate::prompts::EDIT_PROMPT.into(),
            models: script.clone(),
            model_id: "ollama:test".into(),
            tools: ToolRegistry::builtin().catalog(),
            prompt: crate::prompts::PLANNER_PROMPT.into(),
            timeout: None,
            cancel: None,
            model_defaults: ModelDefaults::default(),
            max_attempts: 3,
            per_stage: false,
        };
        (p, script)
    }

    /// Ask for a change: the model gets the recipe and the instruction, the whole answer replaces
    /// the draft, and the change list says what moved.
    #[tokio::test]
    async fn an_edit_returns_the_whole_recipe_and_says_what_changed() {
        let before = Draft::read(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../recipes/summarize-file")).unwrap();
        let mut after = before.yaml.clone();
        // Drop the summary's description and add a step that counts words, feeding a new result.
        after["steps"].as_array_mut().unwrap().push(json!({ "id": "count", "kind": "tool", "name": "Count the words", "tool": "list.table", "inputs": { "items": [], "columns": [] } }));
        after["outputs"]["count"] = json!({ "from": "${steps.count.output}", "type": "table", "label": "Word count" });
        let stage_n = after["stages"].as_array().map(Vec::len).unwrap_or(0);
        if stage_n > 0 {
            after["stages"][stage_n - 1]["steps"].as_array_mut().unwrap().push(json!("count"));
            after["stages"][stage_n - 1]["outputs"].as_array_mut().unwrap().push(json!("count"));
        }
        let files: Vec<Value> = before.files.iter().map(|(p, t)| json!({ "path": p, "text": if p.ends_with(".md") { format!("{t}\nBe brief.") } else { t.clone() } })).collect();
        let answer = json!({ "recipe": after, "files": files }).to_string();
        let (p, script) = planner(vec![answer]);
        let out = p.edit(&before, "Also count the words and keep the summary brief", None).await.unwrap();
        assert!(out.outcome.problems.is_empty(), "{:?}", out.outcome.problems);
        assert_eq!(out.outcome.draft.yaml["id"], before.yaml["id"], "the id stays");
        assert!(out.outcome.draft.yaml["steps"].as_array().unwrap().iter().any(|s| s["id"] == "count"));
        assert!(out.changes.iter().any(|c| c.starts_with("Result added: Word count (`count`)")), "{:?}", out.changes);
        assert!(out.changes.iter().any(|c| c.starts_with("Step added: Count the words (`count`)")), "{:?}", out.changes);
        assert!(out.changes.iter().any(|c| c.starts_with("Stage changed:")), "{:?}", out.changes);
        assert!(out.changes.iter().any(|c| c.starts_with("File rewritten:") || c.starts_with("Files rewritten:")), "{:?}", out.changes);
        let seen = script.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].messages[0].content.contains("## Changing a recipe") && seen[0].messages[0].content.contains("Keep everything the instruction does not touch"));
        assert!(seen[0].messages[1].content.contains("## The recipe as it is") && seen[0].messages[1].content.contains("## The change asked for\n\nAlso count the words"));
        assert!(seen[0].messages[1].content.contains("### prompts/"), "the files go along");
        // Nothing changed: the list is empty.
        assert!(changes(&before, &before).is_empty());
    }

    /// Per stage: two custom stages become two short calls, the second told what the first wrote,
    /// and the merged whole checks clean in one go.
    #[tokio::test]
    async fn a_small_model_writes_one_custom_stage_per_call() {
        let lib = Arc::new(crate::stagelib::StageLibrary::load(&crate::stagelib::repo_stages_dir()).unwrap());
        let strip = json!({ "stages": [ { "use": "summarize_file", "name": "Read it" }, { "name": "Cards", "why": "Cards from the summary." }, { "name": "Save", "why": "Save the cards." } ] }).to_string();
        let cards = json!({
            "recipe": { "id": "x", "name": "X", "description": "d", "inputs": {}, "modelSlots": {},
                "steps": [ { "id": "cards", "kind": "model", "slot": "writer", "prompt": "prompts/cards.md", "inputs": { "text": "${steps.summary_write.output}" } } ],
                "outputs": { "cards": { "from": "${steps.cards.output}", "type": "markdown" } },
                "stages": [ { "id": "cards", "name": "Cards", "why": "w", "steps": ["cards"], "outputs": ["cards"] } ] },
            "files": [ { "path": "prompts/cards.md", "text": "Make cards." } ]
        })
        .to_string();
        let save = json!({
            "recipe": { "id": "x", "name": "X", "description": "d", "inputs": { "out": { "type": "directory", "label": "Where" } }, "modelSlots": {},
                "permissions": { "filesystem": { "write": ["${inputs.out}"] } },
                "steps": [ { "id": "save", "kind": "tool", "tool": "fs.write", "inputs": { "dir": "${inputs.out}", "name": "cards.md", "text": "${steps.cards.output}" } } ],
                "outputs": { "saved": { "from": "${steps.save.output}", "type": "file" } },
                "stages": [ { "id": "save", "name": "Save", "why": "s", "inputs": ["out"], "steps": ["save"], "outputs": ["saved"] } ] },
            "files": []
        })
        .to_string();
        let (mut p, script) = planner(vec![strip, cards, save]);
        p.library = Some(lib.clone());
        p.per_stage = true;
        let out = p.plan(&req(), None).await.unwrap();
        assert!(out.problems.is_empty(), "{:?}", out.problems);
        assert!(out.per_stage);
        let y = &out.draft.yaml;
        assert_eq!(y["stages"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect::<Vec<_>>(), vec!["summarize_file", "cards", "save"]);
        assert_eq!(y["stages"][1]["steps"], json!(["cards"]));
        assert_eq!(y["stages"][2]["steps"], json!(["save"]));
        assert!(y["inputs"].get("out").is_some() && y["inputs"].get("file").is_some());
        assert!(out.draft.files.contains_key("prompts/cards.md"));
        let seen = script.seen.lock().unwrap();
        assert_eq!(seen.len(), 3, "the strip, then one call per custom stage");
        assert!(seen[1].messages[1].content.contains("Write only this one stage") && seen[1].messages[1].content.contains("`cards` **Cards**"));
        assert!(!seen[1].messages[0].content.contains("Already written by you"));
        assert!(seen[2].messages[0].content.contains("Already written by you") && seen[2].messages[0].content.contains("- `cards` **Cards**: steps `cards`"), "{}", seen[2].messages[0].content);
        assert!(seen[2].messages[1].content.contains("`save` **Save**") && !seen[2].messages[1].content.contains("`cards` **Cards**"));
        assert!(out.transcript.contains("── call 3 (steps) ──"));
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
    async fn try_strip_times_one_call_and_judges_the_strip_only() {
        let lib = Arc::new(crate::stagelib::StageLibrary::load(&crate::stagelib::repo_stages_dir()).unwrap());
        // All ready-made: the expanded draft must check clean.
        let strip = json!({ "stages": [ { "use": "summarize_file", "name": "Read it" } ] }).to_string();
        let (mut p, script) = planner(vec![strip]);
        p.library = Some(lib.clone());
        let t = p.try_strip(&trial_request(), None).await.unwrap();
        assert!(t.clean, "{:?}", t.problems);
        assert_eq!(script.seen.lock().unwrap().len(), 1, "the strip call only");
        let seen = &script.seen.lock().unwrap()[0];
        assert!(seen.messages[1].content.contains(TRIAL_GOAL));
        // A custom stage: the strip is judged, not the recipe it has not written yet.
        let strip = json!({ "stages": [ { "use": "read_folder" }, { "name": "Answer", "why": "Answer from the pages." } ] }).to_string();
        let (mut p, _) = planner(vec![strip]);
        p.library = Some(lib.clone());
        assert!(p.try_strip(&trial_request(), None).await.unwrap().clean);
        // No stages, or not a strip at all: not clean, with the reason; not an error.
        let (mut p, _) = planner(vec![json!({ "stages": [] }).to_string()]);
        p.library = Some(lib.clone());
        let t = p.try_strip(&trial_request(), None).await.unwrap();
        assert!(!t.clean);
        assert_eq!(t.problems, vec!["the answer named no stages"]);
        let (mut p, _) = planner(vec!["I would rather not.".into()]);
        p.library = Some(lib.clone());
        let t = p.try_strip(&trial_request(), None).await.unwrap();
        assert!(!t.clean);
        assert!(t.problems[0].starts_with("the answer was not a list of stages"), "{:?}", t.problems);
        // Without a library there is no strip to try.
        let (p, _) = planner(vec![]);
        assert!(matches!(p.try_strip(&trial_request(), None).await, Err(PlanError::Unusable(_))));
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
        assert_eq!(out.schema_version, "0");
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

    #[tokio::test]
    async fn a_permission_the_planner_may_not_ask_for_is_dropped_and_said_in_words() {
        let mut v: Value = serde_json::from_str(&answer("file.read")).unwrap();
        v["recipe"]["permissions"] = json!({
            "filesystem": { "read": ["input", "~", "${inputs.file}", "${inputs.nope}"], "write": ["output", "/tmp/out"] },
            "network": ["input", "example.com"],
            "cloudModels": true,
            "shell": true,
            "credentials": ["github"]
        });
        let (p, _) = planner(vec![v.to_string(), v.to_string(), v.to_string()]);
        let out = p.plan(&req(), None).await.unwrap();
        assert_eq!(
            out.problems,
            vec![
                "The draft asked to read ~, which a recipe may not do on its own. Removed.",
                "The draft asked to read ${inputs.nope}, which a recipe may not do on its own. Removed.",
                "The draft asked to write to /tmp/out, which a recipe may not do on its own. Removed.",
                "The draft asked to reach example.com, which a recipe may not do on its own. Removed.",
                "The draft asked to use cloud models, which a recipe may not do on its own. Removed.",
                "The draft asked to run programs on this Mac (shell), which a recipe may not do on its own. Removed.",
                "The draft asked to use the credential github, which a recipe may not do on its own. Removed.",
            ]
        );
        assert_eq!(out.draft.yaml["permissions"], json!({ "filesystem": { "read": ["input", "${inputs.file}"], "write": ["output"] }, "network": ["input"] }));
        let known: Vec<String> = ToolRegistry::builtin().catalog().into_iter().map(|t| t.id).collect();
        assert!(out.draft.check(&known).is_empty(), "what is left is a recipe that saves: {:?}", out.draft.check(&known));

        // A ready-made stage's own permissions are not the model's and stay.
        let lib = Arc::new(crate::stagelib::StageLibrary::load(&crate::stagelib::repo_stages_dir()).unwrap());
        let strip = json!({ "stages": [ { "use": "read_links" }, { "name": "Notes", "why": "w" } ] }).to_string();
        let answer = json!({
            "recipe": { "id": "x", "name": "X", "description": "d", "inputs": {}, "modelSlots": {},
                "permissions": { "network": ["news.example"] },
                "steps": [ { "id": "notes", "kind": "model", "slot": "reader", "prompt": "prompts/notes.md", "inputs": { "pages": "${steps.links_pages.output}" } } ],
                "outputs": { "notes": { "from": "${steps.notes.output}", "type": "markdown" } },
                "stages": [ { "id": "notes", "name": "Notes", "why": "w", "steps": ["notes"], "outputs": ["notes"] } ] },
            "files": [ { "path": "prompts/notes.md", "text": "Notes." } ]
        })
        .to_string();
        let (mut p, _) = planner(vec![strip, answer.clone(), answer.clone()]);
        p.library = Some(lib);
        let out = p.plan(&req(), None).await.unwrap();
        assert_eq!(out.problems, vec!["The draft asked to reach news.example, which a recipe may not do on its own. Removed."]);
        assert_eq!(out.draft.yaml["permissions"]["network"], json!(["input"]), "what read_links brought");
    }

    /// A constrained decoder writes an object's properties in the schema's order, so a property
    /// listed before `tool` cannot be written after it. Live, this made every tool step come out
    /// as `{ id, kind, tool, params }` with no `inputs` and no `name`.
    #[test]
    fn the_answer_schema_puts_what_a_step_is_before_its_inputs() {
        let s = answer_schema();
        let keys: Vec<String> = s["properties"]["recipe"]["properties"]["steps"]["items"]["properties"].as_object().unwrap().keys().cloned().collect();
        let pos = |k: &str| keys.iter().position(|x| x == k).unwrap_or_else(|| panic!("{k} missing from {keys:?}"));
        assert_eq!(keys[..4], ["id", "kind", "name", "description"]);
        for k in ["tool", "slot", "prompt", "over", "concurrency", "steps", "items", "title", "show", "allowedTools"] {
            assert!(pos(k) < pos("inputs"), "{k} must come before inputs: {keys:?}");
        }
        assert_eq!(keys.last().map(String::as_str), Some("params"), "params is written after a model step's inputs");
        assert_eq!(keys.iter().filter(|k| *k == "params").count(), 1);
        let stage_keys: Vec<String> = s["properties"]["recipe"]["properties"]["stages"]["items"]["properties"].as_object().unwrap().keys().cloned().collect();
        assert_eq!(stage_keys[0], "use");
        let lib = crate::stagelib::StageLibrary::load(&crate::stagelib::repo_stages_dir()).unwrap();
        let strip_keys: Vec<String> = strip_schema(&lib)["properties"]["stages"]["items"]["properties"].as_object().unwrap().keys().cloned().collect();
        assert_eq!(strip_keys[0], "use");
    }

    #[test]
    fn the_answer_schema_follows_the_recipe_schema() {
        let s = answer_schema();
        let step = &s["properties"]["recipe"]["properties"]["steps"]["items"]["properties"];
        for k in ["id", "kind", "tool", "slot", "prompt", "over", "as", "items", "allowedTools", "outputSchema", "when", "dependsOn"] {
            assert!(step.get(k).is_some(), "step property {k} missing");
        }
        assert_eq!(step["kind"]["enum"].as_array().unwrap().len(), 7, "the kind enum is kept");
        let stage = &s["properties"]["recipe"]["properties"]["stages"]["items"]["properties"];
        for k in ["id", "name", "why", "inputs", "steps", "outputs", "keep", "use"] {
            assert!(stage.get(k).is_some(), "stage property {k} missing");
        }
        assert!(stage.get("source").is_none(), "provenance is the app's to write");
        assert_eq!(stage["keep"]["properties"]["refresh"]["enum"], json!(["always", "when_sources_change", "manual"]));
        assert_eq!(serde_json::to_value(crate::migrate::CURRENT_SCHEMA_VERSION).unwrap(), "0");
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

    #[tokio::test]
    async fn the_stream_is_shown_as_phases_deltas_and_done_and_kept_as_a_transcript() {
        let (p, _) = planner(vec![answer("file.readd"), answer("file.read")]);
        let events = Arc::new(Mutex::new(Vec::<PlanEvent>::new()));
        let sink = events.clone();
        let out = p.plan(&req(), Some(Arc::new(move |e| sink.lock().unwrap().push(e)))).await.unwrap();
        let events = events.lock().unwrap();

        // The order, with the deltas of each call folded into one marker.
        let mut order = Vec::new();
        let mut deltas: Vec<String> = Vec::new();
        for e in events.iter() {
            match e {
                PlanEvent::Delta { text, .. } => {
                    if order.last() != Some(&"delta".to_string()) {
                        order.push("delta".into());
                        deltas.push(String::new());
                    }
                    deltas.last_mut().unwrap().push_str(text);
                }
                PlanEvent::Thinking { attempt, .. } => order.push(format!("thinking {attempt}")),
                PlanEvent::Phase { which, attempt, of, .. } => order.push(format!("phase {} {attempt}/{of}", which.as_str())),
                PlanEvent::Checking { attempt, .. } => order.push(format!("checking {attempt}")),
                PlanEvent::Fixing { attempt, .. } => order.push(format!("fixing {attempt}")),
                PlanEvent::Done { .. } => order.push("done".into()),
                PlanEvent::Choosing { .. } => order.push("choosing".into()),
                PlanEvent::Model { phase, .. } => order.push(format!("model {}", serde_json::to_value(phase).unwrap().as_str().unwrap())),
            }
        }
        assert_eq!(
            order,
            vec!["thinking 1", "phase steps 1/3", "model reading", "model writing", "delta", "checking 1", "fixing 1", "thinking 2", "phase fix 2/3", "model reading", "model writing", "delta", "checking 2", "done"],
            "{events:#?}"
        );
        assert_eq!(deltas, vec![answer("file.readd"), answer("file.read")], "the deltas of a call add up to what the model wrote");
        let delta_count = events.iter().filter(|e| matches!(e, PlanEvent::Delta { .. })).count();
        assert!(delta_count >= 2 && delta_count < 6, "chunks are coalesced, not dropped: {delta_count}");

        // Elapsed never goes backwards; tokens come from the provider's count once a call ends.
        let stamps: Vec<(u64, u64)> = events
            .iter()
            .map(|e| match e {
                PlanEvent::Choosing { elapsed_ms, tokens }
                | PlanEvent::Model { elapsed_ms, tokens, .. }
                | PlanEvent::Thinking { elapsed_ms, tokens, .. }
                | PlanEvent::Checking { elapsed_ms, tokens, .. }
                | PlanEvent::Fixing { elapsed_ms, tokens, .. }
                | PlanEvent::Phase { elapsed_ms, tokens, .. }
                | PlanEvent::Delta { elapsed_ms, tokens, .. }
                | PlanEvent::Done { elapsed_ms, tokens } => (*elapsed_ms, *tokens),
            })
            .collect();
        assert!(stamps.windows(2).all(|w| w[0].0 <= w[1].0), "{stamps:?}");
        assert!(matches!(events.iter().find(|e| matches!(e, PlanEvent::Checking { attempt: 1, .. })), Some(PlanEvent::Checking { tokens: 5, .. })));
        assert!(matches!(events.last(), Some(PlanEvent::Done { tokens: 10, .. })), "{:?}", events.last());

        // The transcript: every call, headed.
        assert!(out.transcript.starts_with("── call 1 (steps) ──\n"), "{}", out.transcript);
        assert!(out.transcript.contains("\n\n── call 2 (fix) ──\n"), "{}", out.transcript);
        assert!(out.transcript.contains(&answer("file.readd")) && out.transcript.ends_with(&answer("file.read")));
    }

    #[test]
    fn events_serialize_for_the_window() {
        let s = |e: PlanEvent| serde_json::to_string(&e).unwrap();
        assert_eq!(s(PlanEvent::Choosing { elapsed_ms: 1, tokens: 0 }), r#"{"kind":"choosing","elapsedMs":1,"tokens":0}"#);
        assert_eq!(s(PlanEvent::Thinking { attempt: 1, of: 3, elapsed_ms: 2, tokens: 0 }), r#"{"kind":"thinking","attempt":1,"of":3,"elapsedMs":2,"tokens":0}"#);
        assert_eq!(s(PlanEvent::Checking { attempt: 1, elapsed_ms: 3, tokens: 5 }), r#"{"kind":"checking","attempt":1,"elapsedMs":3,"tokens":5}"#);
        assert_eq!(s(PlanEvent::Fixing { attempt: 1, problems: vec!["p".into()], elapsed_ms: 4, tokens: 5 }), r#"{"kind":"fixing","attempt":1,"problems":["p"],"elapsedMs":4,"tokens":5}"#);
        assert_eq!(s(PlanEvent::Phase { which: PlanPhase::Strip, attempt: 1, of: 1, elapsed_ms: 5, tokens: 0 }), r#"{"kind":"phase","which":"strip","attempt":1,"of":1,"elapsedMs":5,"tokens":0}"#);
        assert_eq!(s(PlanEvent::Phase { which: PlanPhase::Fix, attempt: 2, of: 3, elapsed_ms: 5, tokens: 9 }), r#"{"kind":"phase","which":"fix","attempt":2,"of":3,"elapsedMs":5,"tokens":9}"#);
        assert_eq!(s(PlanEvent::Delta { text: "{\"a\"".into(), elapsed_ms: 6, tokens: 7 }), r#"{"kind":"delta","text":"{\"a\"","elapsedMs":6,"tokens":7}"#);
        assert_eq!(s(PlanEvent::Done { elapsed_ms: 8, tokens: 9 }), r#"{"kind":"done","elapsedMs":8,"tokens":9}"#);
    }
}
