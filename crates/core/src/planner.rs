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
                            "required": ["id", "name", "why"],
                            "properties": {
                                "id": { "type": "string" },
                                "name": { "type": "string" },
                                "why": { "type": "string" },
                                "inputs": { "type": "array", "items": { "type": "string" } },
                                "steps": { "type": "array", "items": { "type": "string" } },
                                "outputs": { "type": "array", "items": { "type": "string" } },
                                "keep": { "type": "object", "properties": { "refresh": { "type": "string", "enum": ["always", "when_sources_change", "manual"] } } }
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

/// The planner prompt with the live tool list appended, so adding a tool teaches the planner.
pub fn assemble_system(prompt: &str, tools: &[ToolInfo]) -> String {
    let mut s = String::from(prompt.trim_end());
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
    declare_keeps(&mut yaml);
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
    Ok(Draft { yaml: Value::Object(yaml), files })
}

/// A stage whose steps write a keep (`keep.put`, `keep.remove`) gets `keep: { refresh: always }`
/// when the model left it out. Models reach for the tools and forget the declaration; a log is
/// the common case, and `always` is its right refresh rule. The checker still guards recipes
/// written by hand.
fn declare_keeps(yaml: &mut serde_json::Map<String, Value>) {
    let writers: Vec<String> = yaml
        .get("steps")
        .and_then(Value::as_array)
        .map(|steps| {
            steps
                .iter()
                .filter(|s| {
                    let direct = s.get("tool").and_then(Value::as_str).is_some_and(|t| t == "keep.put" || t == "keep.remove");
                    let inner = s.get("steps").and_then(Value::as_array).is_some_and(|inner| {
                        inner.iter().any(|x| x.get("tool").and_then(Value::as_str).is_some_and(|t| t == "keep.put" || t == "keep.remove"))
                    });
                    direct || inner
                })
                .filter_map(|s| s.get("id").and_then(Value::as_str).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    if writers.is_empty() {
        return;
    }
    if let Some(stages) = yaml.get_mut("stages").and_then(Value::as_array_mut) {
        for st in stages.iter_mut() {
            let uses = st.get("steps").and_then(Value::as_array).is_some_and(|ids| ids.iter().filter_map(Value::as_str).any(|id| writers.iter().any(|w| w == id)));
            if uses && !st.get("keep").is_some_and(Value::is_object) {
                if let Some(o) = st.as_object_mut() {
                    o.insert("keep".into(), json!({ "refresh": "always" }));
                }
            }
        }
    }
}

pub struct Planner {
    pub models: Arc<dyn ModelClient>,
    pub model_id: String,
    pub tools: Vec<ToolInfo>,
    /// The planner prompt in use (original or the user's revision).
    pub prompt: String,
    pub timeout: Option<Duration>,
    pub cancel: Option<Cancel>,
    pub model_defaults: ModelDefaults,
    /// Model calls at most, counting the fixes. Default 3.
    pub max_attempts: u32,
}

pub type PlanObserver = Arc<dyn Fn(PlanEvent) + Send + Sync>;

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

    pub async fn plan(&self, req: &PlanRequest, observer: Option<PlanObserver>) -> Result<PlanOutcome, PlanError> {
        let started = std::time::Instant::now();
        let known_tools: Vec<String> = self.tools.iter().map(|t| t.id.clone()).collect();
        let emit = |e: PlanEvent| {
            if let Some(o) = &observer {
                o(e);
            }
        };
        let mut messages = vec![
            ChatMessage::new("system", assemble_system(&self.prompt, &self.tools)),
            ChatMessage::new("user", user_message(req)),
        ];
        let max = self.max_attempts.max(1);
        let (mut prompt_tokens, mut completion_tokens) = (0u64, 0u64);
        let mut best: Option<(Draft, Vec<String>)> = None;
        let mut last_error = String::new();
        let mut attempts = 0;
        for attempt in 1..=max {
            attempts = attempt;
            emit(PlanEvent::Thinking { attempt, of: max });
            let resp = self
                .call(ChatRequest {
                    messages: messages.clone(),
                    tools: Vec::new(),
                    json_schema: Some(answer_schema()),
                    temperature: Some(0.2),
                    max_tokens: None,
                    context_length: self.model_defaults.context_length.or(Some(32768)),
                    think: self.model_defaults.think.clone(),
                })
                .await?;
            prompt_tokens += resp.prompt_tokens.unwrap_or(0);
            completion_tokens += resp.completion_tokens.unwrap_or(0);
            emit(PlanEvent::Checking { attempt });
            let problems = match parse_answer(&resp.content, req) {
                Ok(draft) => {
                    let problems = draft.check(&known_tools);
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
            attempts,
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
