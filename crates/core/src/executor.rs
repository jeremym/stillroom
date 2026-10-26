//! Executes a recipe: orders steps by their data dependencies, runs tool and model
//! steps, records a trace. Sequential for now; parallel branches come with Flow Studio.

use indexmap::IndexMap;

use crate::model::{ChatMessage, ChatRequest, ModelClient};
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
}

/// Called after each step record changes, so a UI can show live progress.
pub type StepObserver = Arc<dyn Fn(&StepRecord) + Send + Sync>;

pub struct Executor {
    pub tools: Arc<ToolRegistry>,
    pub models: Arc<dyn ModelClient>,
}

impl Executor {
    pub fn new(tools: Arc<ToolRegistry>, models: Arc<dyn ModelClient>) -> Self {
        Self { tools, models }
    }

    /// Topological order from data references plus explicit `dependsOn`.
    pub fn order(recipe: &Recipe) -> Result<Vec<usize>, ExecError> {
        let n = recipe.steps.len();
        let index: HashMap<&str, usize> = recipe.steps.iter().enumerate().map(|(i, s)| (s.id(), i)).collect();
        let mut deps: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
        for (i, step) in recipe.steps.iter().enumerate() {
            let mut names = BTreeSet::new();
            refs::step_refs(&Value::Object(step.inputs().clone().into_iter().collect()), &mut names);
            names.extend(step.depends_on().iter().cloned());
            for name in names {
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
            };
            if let Some(o) = &observe {
                o(&rec);
            }

            let result = self.run_step(step, req, &tool_ctx, &mut rec, &outputs).await;
            let now = Utc::now();
            rec.finished_at = Some(now);
            rec.duration_ms = rec.started_at.map(|s| (now - s).num_milliseconds().max(0) as u64);
            match result {
                Ok(out) => {
                    rec.status = StepStatus::Succeeded;
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
        outputs: &HashMap<String, Value>,
    ) -> Result<Value, ExecError> {
        let ctx = Context { inputs: &req.inputs, step_outputs: outputs };
        let raw = Value::Object(step.inputs().clone().into_iter().collect());
        let inputs = refs::resolve_value(&raw, &ctx).map_err(|source| ExecError::Ref { step: step.id().into(), source })?;
        rec.inputs = inputs.clone();

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

fn validate_against(schema: &Value, value: &Value) -> Result<(), String> {
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
