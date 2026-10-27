//! Executes a recipe: orders steps by their data dependencies, runs tool and model
//! steps, records a trace. Sequential for now; parallel branches come with Flow Studio.

use indexmap::IndexMap;

use crate::cache::{fingerprint, CachedStep, StepCache};
use crate::model::{ChatMessage, ChatRequest, ModelClient};
use crate::propagate::{dependencies, inputs_fingerprint, StepEdit};
use crate::recipe::{Recipe, Step};
use crate::refs::{self, Context};
use crate::resolve::Resolution;
use crate::tools::{ToolContext, ToolRegistry};
use crate::trace::{Run, RunStatus, StepRecord, StepStatus};
use chrono::Utc;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("dependency cycle involving steps: {0:?}")]
    Cycle(Vec<String>),
    #[error("step {step}: {source}")]
    Ref { step: String, source: refs::RefError },
    #[error("step {step}: unknown tool {tool}")]
    UnknownTool { step: String, tool: String },
    #[error("step {step}: tool failed: {message}")]
    Tool { step: String, message: String },
    #[error("step {step}: slot {slot} was not resolved")]
    UnresolvedSlot { step: String, slot: String },
    #[error("step {step}: prompt {path} not found in recipe bundle")]
    MissingPrompt { step: String, path: String },
    #[error("step {step}: model call failed: {message}")]
    Model { step: String, message: String },
    #[error("step {step}: output was not valid JSON for its schema: {message}")]
    BadStructuredOutput { step: String, message: String },
    #[error("step {step}: {kind} steps are not supported yet")]
    Unsupported { step: String, kind: &'static str },
    #[error("output {name}: {source}")]
    Output { name: String, source: refs::RefError },
    #[error("step {step}: edited output does not match its schema: {message}")]
    BadEdit { step: String, message: String },
}

pub struct RunRequest {
    pub recipe: Recipe,
    /// Prompt files by their path inside the recipe bundle (e.g. `prompts/summarize.md`).
    pub prompts: HashMap<String, String>,
    pub inputs: IndexMap<String, Value>,
    pub resolutions: BTreeMap<String, Resolution>,
    /// Base directory for artifact files; the run gets a subdirectory named by its id.
    /// Defaults to the OS temp dir.
    pub artifact_base: Option<std::path::PathBuf>,
    /// Reuse outputs of unchanged steps from earlier runs when a cache is attached.
    pub reuse: bool,
    /// User edits to apply as step outputs, by step id (change propagation).
    pub edits: BTreeMap<String, StepEdit>,
}

/// Called after each step record changes, so a UI can show live progress.
pub type StepObserver = Arc<dyn Fn(&StepRecord) + Send + Sync>;

pub struct Executor {
    pub tools: Arc<ToolRegistry>,
    pub models: Arc<dyn ModelClient>,
    pub cache: Option<Arc<dyn StepCache>>,
}

impl Executor {
    pub fn new(tools: Arc<ToolRegistry>, models: Arc<dyn ModelClient>) -> Self {
        Self { tools, models, cache: None }
    }

    pub fn with_cache(mut self, cache: Arc<dyn StepCache>) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Everything that determines a step's output: its definition, the resolved inputs,
    /// and for model steps the model id and the prompt text.
    fn cache_key(&self, req: &RunRequest, step: &Step, inputs: &Value) -> Option<String> {
        let def = serde_json::to_string(step).ok()?;
        let inputs = serde_json::to_string(inputs).ok()?;
        let (model, prompt) = match step {
            Step::Model(m) => (
                req.resolutions.get(&m.slot).map(|r| r.model_id.clone()).unwrap_or_default(),
                req.prompts.get(&m.prompt).cloned().unwrap_or_default(),
            ),
            _ => (String::new(), String::new()),
        };
        Some(fingerprint(&[&req.recipe.id, step.id(), &def, &inputs, &model, &prompt]))
    }

    fn reusable(&self, req: &RunRequest, step: &Step) -> bool {
        if !req.reuse || self.cache.is_none() {
            return false;
        }
        match step {
            Step::Tool(t) => t.checkpoint && self.tools.get(&t.tool).is_some_and(|tool| tool.cacheable()),
            Step::Model(m) => m.checkpoint,
            _ => false,
        }
    }

    /// Topological order from data references plus explicit `dependsOn`.
    pub fn order(recipe: &Recipe) -> Result<Vec<usize>, ExecError> {
        let n = recipe.steps.len();
        let index: HashMap<&str, usize> = recipe.steps.iter().enumerate().map(|(i, s)| (s.id(), i)).collect();
        let mut deps: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
        for (i, step) in recipe.steps.iter().enumerate() {
            for name in dependencies(step) {
                if let Some(&j) = index.get(name.as_str()) {
                    deps[i].insert(j);
                }
            }
        }
        let mut done = vec![false; n];
        let mut order = Vec::with_capacity(n);
        while order.len() < n {
            let next = (0..n).find(|&i| !done[i] && deps[i].iter().all(|&j| done[j]));
            match next {
                Some(i) => {
                    done[i] = true;
                    order.push(i);
                }
                None => {
                    let stuck = (0..n).filter(|&i| !done[i]).map(|i| recipe.steps[i].id().to_string()).collect();
                    return Err(ExecError::Cycle(stuck));
                }
            }
        }
        Ok(order)
    }

    pub async fn run(&self, req: RunRequest, observe: Option<StepObserver>) -> Run {
        let run_id = uuid::Uuid::now_v7().to_string();
        let mut run = Run {
            id: run_id.clone(),
            recipe_id: req.recipe.id.clone(),
            recipe_name: req.recipe.name.clone(),
            recipe_version: req.recipe.version.clone(),
            status: RunStatus::Running,
            started_at: Utc::now(),
            finished_at: None,
            inputs: req.inputs.clone(),
            resolutions: serde_json::to_value(&req.resolutions).unwrap_or(Value::Null),
            steps: Vec::new(),
            outputs: IndexMap::new(),
            error: None,
            edits: req.edits.clone(),
        };

        let order = match Self::order(&req.recipe) {
            Ok(o) => o,
            Err(e) => return finish(run, Err(e)),
        };

        // Optional inputs the caller left out resolve to their default (or null).
        let mut req = req;
        for (name, default) in req.recipe.optional_inputs() {
            req.inputs.entry(name).or_insert(default);
        }
        let req = &req;

        let tool_ctx = ToolContext {
            run_id: run_id.clone(),
            artifact_dir: req
                .artifact_base
                .clone()
                .unwrap_or_else(|| std::env::temp_dir().join("stillroom-artifacts"))
                .join(&run_id),
        };

        let mut outputs: HashMap<String, Value> = HashMap::new();
        for (position, &i) in order.iter().enumerate() {
            let step = &req.recipe.steps[i];
            let mut rec = StepRecord {
                run_id: run_id.clone(),
                step_id: step.id().to_string(),
                kind: step.kind_name().to_string(),
                name: step.name().map(str::to_string),
                position: position as u32,
                status: StepStatus::Running,
                started_at: Some(Utc::now()),
                finished_at: None,
                duration_ms: None,
                model: None,
                prompt_path: None,
                prompt_tokens: None,
                completion_tokens: None,
                inputs: Value::Null,
                output: None,
                error: None,
                reused_from: None,
                edited: false,
            };
            if let Some(o) = &observe {
                o(&rec);
            }

            // Resolve inputs up front so the cache key can see them.
            let ctx = Context { inputs: &req.inputs, step_outputs: &outputs };
            let raw = Value::Object(step.inputs().clone().into_iter().collect());
            let resolved = refs::resolve_value(&raw, &ctx).map_err(|source| ExecError::Ref { step: step.id().into(), source });
            let mut cache_key = None;
            let result = match resolved {
                Err(e) => Err(e),
                Ok(inputs) => {
                    rec.inputs = inputs.clone();
                    // A user edit stands in for the step while the inputs it was made against hold.
                    let edit = req
                        .edits
                        .get(step.id())
                        .filter(|e| e.inputs_fingerprint == inputs_fingerprint(&inputs));
                    let key = self.cache_key(req, step, &inputs);
                    let hit = if edit.is_none() && self.reusable(req, step) {
                        key.as_deref().and_then(|k| self.cache.as_ref()?.get(k))
                    } else {
                        None
                    };
                    match (edit, hit) {
                        (Some(edit), _) => {
                            rec.edited = true;
                            if let Step::Model(m) = step {
                                rec.prompt_path = Some(m.prompt.clone());
                            }
                            let schema = match step {
                                Step::Model(m) => m.output_schema.as_ref(),
                                Step::Tool(t) => t.output_schema.as_ref(),
                                _ => None,
                            };
                            match schema.map(|s| validate_against(s, &edit.output)) {
                                Some(Err(message)) => Err(ExecError::BadEdit { step: step.id().into(), message }),
                                _ => Ok(edit.output.clone()),
                            }
                        }
                        (None, Some(cached)) => {
                            rec.reused_from = Some(cached.run_id.clone());
                            rec.model = cached.model.clone();
                            if let Step::Model(m) = step {
                                rec.prompt_path = Some(m.prompt.clone());
                            }
                            Ok(cached.output)
                        }
                        (None, None) => {
                            cache_key = key;
                            self.run_step(step, req, &tool_ctx, &mut rec, &inputs).await
                        }
                    }
                }
            };
            let now = Utc::now();
            rec.finished_at = Some(now);
            rec.duration_ms = rec.started_at.map(|s| (now - s).num_milliseconds().max(0) as u64);
            match result {
                Ok(out) => {
                    rec.status = StepStatus::Succeeded;
                    if let (Some(cache), Some(key), true) = (&self.cache, cache_key.as_deref(), self.reusable(req, step)) {
                        cache.put(
                            key,
                            &CachedStep {
                                run_id: run_id.clone(),
                                step_id: step.id().to_string(),
                                model: rec.model.clone(),
                                output: out.clone(),
                                created_at: now,
                            },
                        );
                    }
                    outputs.insert(step.id().to_string(), out.clone());
                    rec.output = Some(out);
                    if let Some(o) = &observe {
                        o(&rec);
                    }
                    run.steps.push(rec);
                }
                Err(e) => {
                    rec.status = StepStatus::Failed;
                    rec.error = Some(e.to_string());
                    if let Some(o) = &observe {
                        o(&rec);
                    }
                    run.steps.push(rec);
                    return finish(run, Err(e));
                }
            }
        }

        // Collect declared outputs.
        let ctx = Context { inputs: &req.inputs, step_outputs: &outputs };
        let mut run = run;
        for (name, spec) in &req.recipe.outputs {
            match refs::resolve_value(&Value::String(spec.from.clone()), &ctx) {
                Ok(v) => {
                    run.outputs.insert(name.clone(), v);
                }
                Err(source) => return finish(run, Err(ExecError::Output { name: name.clone(), source })),
            }
        }
        finish(run, Ok(()))
    }

    async fn run_step(
        &self,
        step: &Step,
        req: &RunRequest,
        tool_ctx: &ToolContext,
        rec: &mut StepRecord,
        inputs: &Value,
    ) -> Result<Value, ExecError> {
        let inputs = inputs.clone();
        match step {
            Step::Tool(t) => {
                let tool = self
                    .tools
                    .get(&t.tool)
                    .ok_or_else(|| ExecError::UnknownTool { step: t.id.clone(), tool: t.tool.clone() })?;
                tool.call(tool_ctx, inputs).await.map_err(|e| ExecError::Tool { step: t.id.clone(), message: e.to_string() })
            }
            Step::Model(m) => {
                let res = req
                    .resolutions
                    .get(&m.slot)
                    .ok_or_else(|| ExecError::UnresolvedSlot { step: m.id.clone(), slot: m.slot.clone() })?;
                let system = req
                    .prompts
                    .get(&m.prompt)
                    .ok_or_else(|| ExecError::MissingPrompt { step: m.id.clone(), path: m.prompt.clone() })?;
                rec.model = Some(res.model_id.clone());
                rec.prompt_path = Some(m.prompt.clone());

                let user = user_message(&inputs);
                let params = m.params.clone().unwrap_or_default();
                let chat = ChatRequest {
                    messages: vec![
                        ChatMessage { role: "system".into(), content: system.clone() },
                        ChatMessage { role: "user".into(), content: user },
                    ],
                    json_schema: m.output_schema.clone(),
                    temperature: params.temperature,
                    max_tokens: params.max_tokens,
                    context_length: params.context_length,
                    think: params.think.clone(),
                };
                let resp = self
                    .models
                    .chat(&res.model_id, chat)
                    .await
                    .map_err(|e| ExecError::Model { step: m.id.clone(), message: e.to_string() })?;
                rec.prompt_tokens = resp.prompt_tokens;
                rec.completion_tokens = resp.completion_tokens;
                match &m.output_schema {
                    Some(schema) => {
                        let value = serde_json::from_str::<Value>(&resp.content)
                            .map_err(|e| ExecError::BadStructuredOutput { step: m.id.clone(), message: e.to_string() })?;
                        validate_against(schema, &value)
                            .map_err(|message| ExecError::BadStructuredOutput { step: m.id.clone(), message })?;
                        Ok(value)
                    }
                    None => Ok(Value::String(resp.content)),
                }
            }
            other => Err(ExecError::Unsupported { step: other.id().into(), kind: other.kind_name() }),
        }
    }
}

/// Build the user message from resolved inputs. One string input is passed verbatim;
/// otherwise each non-null input becomes a titled section, strings raw and other values as JSON.
fn user_message(inputs: &Value) -> String {
    let Some(obj) = inputs.as_object() else { return inputs.to_string() };
    let present: Vec<(&String, &Value)> = obj.iter().filter(|(_, v)| !v.is_null()).collect();
    if let [(_, v)] = present.as_slice() {
        if let Some(s) = v.as_str() {
            return s.to_string();
        }
    }
    let mut out = String::new();
    for (k, v) in present {
        out.push_str(&format!("## {k}\n\n"));
        match v {
            Value::String(s) => out.push_str(s),
            other => out.push_str(&format!("```json\n{}\n```", serde_json::to_string_pretty(other).unwrap_or_default())),
        }
        out.push_str("\n\n");
    }
    out.trim_end().to_string()
}

pub(crate) fn validate_against(schema: &Value, value: &Value) -> Result<(), String> {
    let validator = jsonschema::validator_for(schema).map_err(|e| format!("bad output schema: {e}"))?;
    let errors: Vec<String> = validator.iter_errors(value).map(|e| format!("{} at {}", e, e.instance_path)).collect();
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}

fn finish(mut run: Run, result: Result<(), ExecError>) -> Run {
    run.finished_at = Some(Utc::now());
    match result {
        Ok(()) => run.status = RunStatus::Succeeded,
        Err(e) => {
            run.status = RunStatus::Failed;
            run.error = Some(e.to_string());
        }
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChatResponse, ModelError};
    use crate::resolve::ResolvedVia;
    use std::io::Write;

    struct EchoModel;
    #[async_trait::async_trait]
    impl ModelClient for EchoModel {
        async fn chat(&self, model_id: &str, req: ChatRequest) -> Result<ChatResponse, ModelError> {
            let user = &req.messages.last().unwrap().content;
            Ok(ChatResponse {
                content: format!("[{model_id}] {}", user.trim()),
                prompt_tokens: Some(10),
                completion_tokens: Some(5),
            })
        }
    }

    #[tokio::test]
    async fn runs_the_builtin_summarize_recipe_end_to_end() {
        let yaml = include_str!("../../../recipes/summarize-file/recipe.yaml");
        let prompt = include_str!("../../../recipes/summarize-file/prompts/summarize.md");
        let recipe = Recipe::from_yaml(yaml).unwrap();

        let dir = std::env::temp_dir().join(format!("stillroom-test-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notes.txt");
        writeln!(std::fs::File::create(&file).unwrap(), "hello world").unwrap();

        let req = RunRequest {
            recipe,
            prompts: HashMap::from([("prompts/summarize.md".to_string(), prompt.to_string())]),
            inputs: IndexMap::from([("file".to_string(), Value::String(file.to_string_lossy().into()))]),
            resolutions: BTreeMap::from([(
                "writer".to_string(),
                Resolution { slot: "writer".into(), model_id: "test:echo".into(), via: ResolvedVia::Capability },
            )]),
            artifact_base: Some(dir.clone()),
            reuse: true,
            edits: BTreeMap::new(),
        };
        let exec = Executor::new(Arc::new(ToolRegistry::builtin()), Arc::new(EchoModel));
        let run = exec.run(req, None).await;
        assert_eq!(run.status, RunStatus::Succeeded, "{:?}", run.error);
        assert_eq!(run.steps.len(), 2);
        assert_eq!(run.steps[0].step_id, "read");
        assert_eq!(run.steps[1].model.as_deref(), Some("test:echo"));
        assert_eq!(run.outputs["summary"], Value::String("[test:echo] hello world".into()));
        assert_eq!(run.summary().prompt_tokens, 10);
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn second_run_reuses_unchanged_model_steps() {
        use crate::cache::MemoryCache;
        let yaml = include_str!("../../../recipes/summarize-file/recipe.yaml");
        let prompt = include_str!("../../../recipes/summarize-file/prompts/summarize.md");
        let dir = std::env::temp_dir().join(format!("stillroom-reuse-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notes.txt");
        std::fs::write(&file, "same text").unwrap();
        let make = || RunRequest {
            recipe: Recipe::from_yaml(yaml).unwrap(),
            prompts: HashMap::from([("prompts/summarize.md".to_string(), prompt.to_string())]),
            inputs: IndexMap::from([("file".to_string(), Value::String(file.to_string_lossy().into()))]),
            resolutions: BTreeMap::from([(
                "writer".to_string(),
                Resolution { slot: "writer".into(), model_id: "test:echo".into(), via: ResolvedVia::Capability },
            )]),
            artifact_base: Some(dir.clone()),
            reuse: true,
            edits: BTreeMap::new(),
        };
        let exec = Executor::new(Arc::new(ToolRegistry::builtin()), Arc::new(EchoModel)).with_cache(Arc::new(MemoryCache::default()));
        let first = exec.run(make(), None).await;
        assert!(first.steps.iter().all(|s| s.reused_from.is_none()));
        let second = exec.run(make(), None).await;
        assert_eq!(second.status, RunStatus::Succeeded);
        assert!(second.steps[0].reused_from.is_none(), "file.read is not cacheable");
        assert_eq!(second.steps[1].reused_from.as_deref(), Some(first.id.as_str()), "summarize reused");
        assert_eq!(second.outputs["summary"], first.outputs["summary"]);
        // A changed prompt invalidates the model step.
        let mut changed = make();
        changed.prompts.insert("prompts/summarize.md".into(), "different prompt".into());
        let third = exec.run(changed, None).await;
        assert!(third.steps[1].reused_from.is_none());
        std::fs::remove_dir_all(dir).ok();
    }

    /// read -> a -> b, and c reads `read` directly. Editing `a` must regenerate b only.
    const FAN_YAML: &str = r#"
schemaVersion: "0"
id: fan
name: Fan
modelSlots:
  writer: { requires: [text] }
steps:
  - id: read
    kind: tool
    tool: file.read
    inputs: { path: "${inputs.file}" }
  - id: a
    kind: model
    slot: writer
    prompt: prompts/a.md
    inputs: { text: "${steps.read.output.text}" }
  - id: b
    kind: model
    slot: writer
    prompt: prompts/b.md
    inputs: { text: "${steps.a.output}" }
  - id: c
    kind: model
    slot: writer
    prompt: prompts/c.md
    inputs: { text: "${steps.read.output.text}" }
outputs:
  b: { from: "${steps.b.output}", type: text }
  c: { from: "${steps.c.output}", type: text }
"#;

    fn fan_request(file: &std::path::Path, dir: &std::path::Path) -> RunRequest {
        RunRequest {
            recipe: Recipe::from_yaml(FAN_YAML).unwrap(),
            prompts: HashMap::from([
                ("prompts/a.md".to_string(), "A".to_string()),
                ("prompts/b.md".to_string(), "B".to_string()),
                ("prompts/c.md".to_string(), "C".to_string()),
            ]),
            inputs: IndexMap::from([("file".to_string(), Value::String(file.to_string_lossy().into()))]),
            resolutions: BTreeMap::from([(
                "writer".to_string(),
                Resolution { slot: "writer".into(), model_id: "test:echo".into(), via: ResolvedVia::Capability },
            )]),
            artifact_base: Some(dir.to_path_buf()),
            reuse: true,
            edits: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn an_edit_regenerates_only_its_dependents() {
        use crate::cache::MemoryCache;
        use crate::propagate::{edits_for_rerun, inputs_fingerprint};
        let dir = std::env::temp_dir().join(format!("stillroom-edit-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("in.txt");
        std::fs::write(&file, "v1").unwrap();
        let exec = Executor::new(Arc::new(ToolRegistry::builtin()), Arc::new(EchoModel)).with_cache(Arc::new(MemoryCache::default()));

        let first = exec.run(fan_request(&file, &dir), None).await;
        assert_eq!(first.status, RunStatus::Succeeded, "{:?}", first.error);
        assert_eq!(first.outputs["b"], Value::String("[test:echo] [test:echo] v1".into()));

        // The user rewrites a's output.
        let mut req = fan_request(&file, &dir);
        req.edits = edits_for_rerun(&first, "a", Value::String("edited".into())).unwrap();
        let second = exec.run(req, None).await;
        assert_eq!(second.status, RunStatus::Succeeded, "{:?}", second.error);
        let by_id = |run: &Run, id: &str| run.steps.iter().find(|s| s.step_id == id).cloned().unwrap();
        assert!(by_id(&second, "a").edited);
        assert!(by_id(&second, "a").reused_from.is_none());
        assert!(by_id(&second, "b").reused_from.is_none(), "b depends on the edit, so it ran again");
        assert_eq!(second.outputs["b"], Value::String("[test:echo] edited".into()));
        assert_eq!(by_id(&second, "c").reused_from.as_deref(), Some(first.id.as_str()), "c did not change");
        assert_eq!(second.edits.len(), 1);

        // Rerunning without the edit gets the generated output back: edits are never cached.
        let third = exec.run(fan_request(&file, &dir), None).await;
        assert_eq!(third.outputs["b"], first.outputs["b"]);
        assert_eq!(by_id(&third, "a").reused_from.as_deref(), Some(first.id.as_str()));

        // The source changes under the edit: the edit no longer matches a's inputs and is set aside.
        std::fs::write(&file, "v2").unwrap();
        let mut req = fan_request(&file, &dir);
        req.edits = edits_for_rerun(&second, "a", Value::String("edited".into())).unwrap();
        assert_eq!(req.edits["a"].inputs_fingerprint, inputs_fingerprint(&by_id(&second, "a").inputs));
        let fourth = exec.run(req, None).await;
        assert_eq!(fourth.status, RunStatus::Succeeded, "{:?}", fourth.error);
        assert!(!by_id(&fourth, "a").edited, "stale edit dropped");
        assert_eq!(fourth.outputs["b"], Value::String("[test:echo] [test:echo] v2".into()));
        assert!(fourth.edits.contains_key("a"), "the run still records the edit it was asked to apply");
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn an_edit_that_breaks_the_schema_fails_the_step() {
        let dir = std::env::temp_dir().join(format!("stillroom-badedit-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("in.txt");
        std::fs::write(&file, "v1").unwrap();
        let exec = Executor::new(Arc::new(ToolRegistry::builtin()), Arc::new(EchoModel));
        let mut req = fan_request(&file, &dir);
        if let Step::Model(m) = &mut req.recipe.steps[1] {
            m.output_schema = Some(serde_json::json!({ "type": "object" }));
        }
        let first = exec.run(fan_request(&file, &dir), None).await;
        req.edits = crate::propagate::edits_for_rerun(&first, "a", Value::String("not an object".into())).unwrap();
        let run = exec.run(req, None).await;
        assert_eq!(run.status, RunStatus::Failed);
        assert!(run.error.unwrap().contains("edited output does not match"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn user_message_shapes() {
        use serde_json::json;
        assert_eq!(user_message(&json!({ "text": "hi" })), "hi");
        assert_eq!(user_message(&json!({ "text": "hi", "focus": null })), "hi");
        let m = user_message(&json!({ "text": "hi", "focus": "tokens" }));
        assert!(m.starts_with("## text\n\nhi\n\n## focus\n\ntokens"), "declared order is kept: {m}");
        let j = user_message(&json!({ "spine": { "a": 1 } }));
        assert!(j.contains("```json"), "{j}");
    }

    #[test]
    fn detects_cycles() {
        let yaml = r#"
schemaVersion: "0"
id: cyc
name: Cyc
steps:
  - id: a
    kind: tool
    tool: file.read
    inputs: { path: "${steps.b.output.path}" }
  - id: b
    kind: tool
    tool: file.read
    inputs: { path: "${steps.a.output.path}" }
outputs:
  o: { from: "${steps.b.output}", type: text }
"#;
        let recipe = Recipe::from_yaml(yaml).unwrap();
        assert!(matches!(Executor::order(&recipe), Err(ExecError::Cycle(_))));
    }
}
