//! Executes a recipe: orders steps by their data dependencies, runs tool, model, agent,
//! map, and review steps, records a trace. Sequential for now; parallel branches come with
//! Flow Studio.

use indexmap::IndexMap;

use crate::cache::{fingerprint, CachedStep, StepCache};
use crate::model::{ChatMessage, ChatRequest, ChatResponse, ModelClient, ToolSpec};
use crate::propagate::{dependencies, dependents, inputs_fingerprint, StepEdit};
use crate::recipe::{AgentStep, MapStep, Recipe, Step};
use crate::refs::{self, Context};
use crate::resolve::Resolution;
use crate::tools::{NetPolicy, normalize_path, FsPolicy, ToolContext, ToolRegistry};
use crate::trace::{Run, RunStatus, RunTrigger, StepRecord, StepStatus, ToolCallRecord};
use chrono::Utc;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
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
    #[error("step {step}: slot {slot} has no model")]
    UnresolvedSlot { step: String, slot: String },
    #[error("step {step}: prompt {path} not found in recipe bundle")]
    MissingPrompt { step: String, path: String },
    #[error("step {step}: model call failed: {message}")]
    Model { step: String, message: String },
    #[error("step {step}: the answer did not fit the output shape (schema): {message}")]
    BadStructuredOutput { step: String, message: String },
    #[error("step {step}: {kind} steps are not supported yet")]
    Unsupported { step: String, kind: &'static str },
    #[error("output {name}: {source}")]
    Output { name: String, source: refs::RefError },
    #[error("step {step}: edited output does not match its schema: {message}")]
    BadEdit { step: String, message: String },
    #[error("step {step}: the thing to loop over is not a list (got {got})")]
    NotAnArray { step: String, got: String },
    #[error("step {step}: the agent used all {turns} turns without finishing")]
    AgentTurns { step: String, turns: u32 },
    #[error("step {step}: tool {tool} cannot be given to an agent (it has no description)")]
    ToolNotDescribable { step: String, tool: String },
    #[error("stopped by you")]
    Cancelled,
    #[error("step {step}: the model did not answer within {secs} s")]
    Timeout { step: String, secs: u64 },
}

/// A handle to stop a run from outside. Between steps it stops at once; during a model call the
/// call is abandoned and the step records "stopped by you".
#[derive(Clone, Default, Debug)]
pub struct Cancel {
    flag: Arc<AtomicBool>,
    notify: Arc<tokio::sync::Notify>,
}

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
    /// Resolves when cancelled, now or later.
    pub async fn cancelled(&self) {
        let notified = self.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

/// What a model step gets when its recipe says nothing: the user's defaults from Settings.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelDefaults {
    pub context_length: Option<u32>,
    pub think: Option<Value>,
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
    /// The run this one continues (an approval or an edit). Every step that run completed
    /// with the same inputs is reused from its trace, even steps that are not cacheable, so
    /// continuing never repeats a side effect such as filing an email.
    pub resume_from: Option<Box<Run>>,
    /// What started this run, when it was not the user. Recorded on the trace.
    pub trigger: Option<RunTrigger>,
    /// Stop handle; a run without one cannot be stopped from outside.
    pub cancel: Option<Cancel>,
    /// How long one model call may take before the step fails. None means wait forever.
    pub model_timeout: Option<std::time::Duration>,
    /// Context length and thinking for steps whose recipe leaves them unset.
    pub model_defaults: ModelDefaults,
}

/// Called after each step record changes, so a UI can show live progress.
pub type StepObserver = Arc<dyn Fn(&StepRecord) + Send + Sync>;

pub struct Executor {
    pub tools: Arc<ToolRegistry>,
    pub models: Arc<dyn ModelClient>,
    pub cache: Option<Arc<dyn StepCache>>,
}

/// Mutable state of one run while it executes.
struct RunState<'a> {
    req: &'a RunRequest,
    run_id: String,
    tool_ctx: ToolContext,
    /// Outputs of top-level steps.
    outputs: HashMap<String, Value>,
    records: Vec<StepRecord>,
    position: u32,
    observe: Option<StepObserver>,
    /// Top-level steps blocked behind a review that is still pending.
    held: BTreeSet<String>,
    needs_review: bool,
}

/// Where a step runs: map-item bindings and the outputs of sibling steps in the same list.
struct Scope<'a> {
    vars: &'a HashMap<String, Value>,
    local: &'a HashMap<String, Value>,
    parent: Option<&'a str>,
}

/// Build the filesystem policy from `permissions.filesystem` and the actual inputs.
/// `input` means every file, files, or directory input; `output` the run's artifact folder;
/// `${inputs.x}` one input; anything else a literal path. With no permissions block, inputs
/// are readable and nothing outside the artifact folder is writable.
pub fn fs_policy(recipe: &Recipe, inputs: &IndexMap<String, Value>, artifact_dir: &std::path::Path) -> FsPolicy {
    let path_inputs: Vec<PathBufLike> = recipe
        .inputs
        .iter()
        .filter(|(_, spec)| matches!(spec.get("type").and_then(Value::as_str), Some("file" | "files" | "directory")))
        .filter_map(|(name, _)| inputs.get(name).map(|v| (name.clone(), v.clone())))
        .collect();
    let expand = |entry: &str| -> Vec<std::path::PathBuf> {
        match entry {
            "input" => path_inputs.iter().flat_map(|(_, v)| value_paths(v)).collect(),
            "output" => vec![artifact_dir.to_path_buf()],
            e => match refs::parse_ref(e) {
                Some(Ok(refs::Ref::Input { name, .. })) => inputs.get(&name).map(value_paths).unwrap_or_default(),
                Some(_) => Vec::new(),
                None => vec![normalize_path(e)],
            },
        }
    };
    let fs = recipe.permissions.as_ref().and_then(|p| p.get("filesystem"));
    let list = |key: &str| -> Option<Vec<String>> {
        fs?.get(key)?.as_array().map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
    };
    let read = list("read").unwrap_or_else(|| vec!["input".into()]);
    let write = list("write").unwrap_or_default();
    FsPolicy {
        read_roots: read.iter().flat_map(|e| expand(e)).collect(),
        write_roots: write.iter().flat_map(|e| expand(e)).collect(),
    }
}

type PathBufLike = (String, Value);

/// The hosts a run may reach: `permissions.network` entries, where `input` means the hosts of
/// every link the user gave as a `url` or `urls` input and anything else is a host pattern.
pub fn net_policy(recipe: &Recipe, inputs: &IndexMap<String, Value>) -> NetPolicy {
    let link_hosts = || -> Vec<String> {
        recipe
            .inputs
            .iter()
            .filter(|(_, spec)| matches!(spec.get("type").and_then(Value::as_str), Some("url" | "urls")))
            .filter_map(|(name, _)| inputs.get(name))
            .flat_map(|v| match v {
                Value::String(s) => vec![s.clone()],
                Value::Array(a) => a.iter().filter_map(Value::as_str).map(String::from).collect(),
                _ => Vec::new(),
            })
            .filter_map(|s| url::Url::parse(s.trim()).ok().and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase())))
            .collect()
    };
    let entries: Vec<String> = recipe
        .permissions
        .as_ref()
        .and_then(|p| p.get("network"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
        .unwrap_or_default();
    let mut allow = Vec::new();
    for e in entries {
        match e.as_str() {
            "input" => allow.extend(link_hosts()),
            other => match refs::parse_ref(other) {
                Some(Ok(refs::Ref::Input { name, .. })) => {
                    if let Some(v) = inputs.get(&name) {
                        let vals: Vec<&str> = match v {
                            Value::String(s) => vec![s.as_str()],
                            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
                            _ => Vec::new(),
                        };
                        allow.extend(vals.into_iter().filter_map(|s| url::Url::parse(s.trim()).ok().and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))));
                    }
                }
                Some(_) => {}
                None => allow.push(other.to_string()),
            },
        }
    }
    NetPolicy { allow }
}

fn value_paths(v: &Value) -> Vec<std::path::PathBuf> {
    match v {
        Value::String(s) => vec![normalize_path(s)],
        Value::Array(a) => a.iter().filter_map(Value::as_str).map(normalize_path).collect(),
        _ => Vec::new(),
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::Object(_) => true,
    }
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
    /// and for model and agent steps the model id and the prompt text.
    fn cache_key(&self, req: &RunRequest, step: &Step, inputs: &Value) -> Option<String> {
        let def = serde_json::to_string(step).ok()?;
        let inputs = serde_json::to_string(inputs).ok()?;
        let (slot, prompt) = match step {
            Step::Model(m) => (Some(&m.slot), Some(&m.prompt)),
            Step::Agent(a) => (Some(&a.slot), Some(&a.prompt)),
            _ => (None, None),
        };
        let model = slot.and_then(|s| req.resolutions.get(s)).map(|r| r.model_id.clone()).unwrap_or_default();
        let prompt = prompt.and_then(|p| req.prompts.get(p)).cloned().unwrap_or_default();
        Some(fingerprint(&[&req.recipe.id, step.id(), &def, &inputs, &model, &prompt]))
    }

    /// Whether a continued run may take this step's result from the run it continues.
    fn resumable(&self, step: &Step) -> bool {
        let tool = |id: &str| self.tools.get(id);
        match step {
            Step::Review(_) => false,
            Step::Tool(t) => tool(&t.tool).is_none_or(|x| x.cacheable() || x.has_side_effects()),
            Step::Agent(a) => {
                a.allowed_tools.iter().any(|t| tool(t).is_some_and(|x| x.has_side_effects()))
                    || a.allowed_tools.iter().all(|t| tool(t).is_none_or(|x| x.cacheable()))
            }
            _ => true,
        }
    }

    fn reusable(&self, req: &RunRequest, step: &Step) -> bool {
        if !req.reuse || self.cache.is_none() || !step.checkpoint() {
            return false;
        }
        match step {
            Step::Tool(t) => self.tools.get(&t.tool).is_some_and(|tool| tool.cacheable()),
            Step::Model(_) => true,
            // An agent may have acted on the world through its tools; only reuse when every
            // tool it may call is itself cacheable.
            Step::Agent(a) => a.allowed_tools.iter().all(|t| self.tools.get(t).is_some_and(|tool| tool.cacheable())),
            _ => false,
        }
    }

    /// Topological order from data references plus explicit `dependsOn`.
    pub fn order(recipe: &Recipe) -> Result<Vec<usize>, ExecError> {
        Self::order_steps(&recipe.steps)
    }

    fn order_steps(steps: &[Step]) -> Result<Vec<usize>, ExecError> {
        let n = steps.len();
        let index: HashMap<&str, usize> = steps.iter().enumerate().map(|(i, s)| (s.id(), i)).collect();
        let mut deps: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
        for (i, step) in steps.iter().enumerate() {
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
                    let stuck = (0..n).filter(|&i| !done[i]).map(|i| steps[i].id().to_string()).collect();
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
            trigger: req.trigger.clone(),
            continues: req.resume_from.as_ref().map(|r| r.id.clone()),
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

        let artifact_dir = req
            .artifact_base
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().join("stillroom-artifacts"))
            .join(&run_id);
        let tool_ctx = ToolContext {
            run_id: run_id.clone(),
            fs: fs_policy(&req.recipe, &req.inputs, &artifact_dir),
            net: net_policy(&req.recipe, &req.inputs),
            artifact_dir,
        };

        let mut state = RunState {
            req,
            run_id: run_id.clone(),
            tool_ctx,
            outputs: HashMap::new(),
            records: Vec::new(),
            position: 0,
            observe,
            held: BTreeSet::new(),
            needs_review: false,
        };

        let empty_vars = HashMap::new();
        let empty_local = HashMap::new();
        let mut failure = None;
        for &i in &order {
            let step = &req.recipe.steps[i];
            let scope = Scope { vars: &empty_vars, local: &empty_local, parent: None };
            match self.run_one(step, step.id().to_string(), &scope, &mut state).await {
                Ok(out) => {
                    state.outputs.insert(step.id().to_string(), out);
                }
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        }
        run.steps = std::mem::take(&mut state.records);
        if let Some(e) = failure {
            return finish(run, Err(e));
        }

        // Collect declared outputs. While a review is pending, outputs behind it are null.
        let ctx = Context { inputs: &req.inputs, step_outputs: &state.outputs, vars: &empty_vars };
        for (name, spec) in &req.recipe.outputs {
            match refs::resolve_value(&Value::String(spec.from.clone()), &ctx) {
                Ok(v) => {
                    run.outputs.insert(name.clone(), v);
                }
                Err(_) if state.needs_review => {
                    run.outputs.insert(name.clone(), Value::Null);
                }
                Err(source) => return finish(run, Err(ExecError::Output { name: name.clone(), source })),
            }
        }
        if state.needs_review {
            run.finished_at = Some(Utc::now());
            run.status = RunStatus::NeedsReview;
            return run;
        }
        finish(run, Ok(()))
    }

    /// Run one step in a scope and return its output. Records the trace entry.
    fn run_one<'s, 'a: 's>(
        &'s self,
        step: &'a Step,
        record_id: String,
        scope: &'s Scope<'s>,
        state: &'s mut RunState<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ExecError>> + Send + 's>> {
        Box::pin(async move {
            let req = state.req;
            if req.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
                return Err(ExecError::Cancelled);
            }
            let mut rec = StepRecord {
                run_id: state.run_id.clone(),
                step_id: record_id.clone(),
                kind: step.kind_name().to_string(),
                name: step.name().map(str::to_string),
                position: state.position,
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
                parent: scope.parent.map(String::from),
                skip_reason: None,
                tool_calls: Vec::new(),
            };
            state.position += 1;
            if let Some(o) = &state.observe {
                o(&rec);
            }

            // References see this list's outputs first, then the top-level ones. Cloned so the
            // state stays free to mutate while the step runs.
            let mut step_outputs = state.outputs.clone();
            step_outputs.extend(scope.local.iter().map(|(k, v)| (k.clone(), v.clone())));
            let ctx = Context { inputs: &req.inputs, step_outputs: &step_outputs, vars: scope.vars };

            let result: Result<Value, ExecError> = async {
                // Held behind a pending review?
                if scope.parent.is_none() && state.held.contains(step.id()) {
                    rec.skip_reason = Some("waiting for review".into());
                    return Ok(Value::Null);
                }
                // `when` condition.
                if let Some(cond) = step.when() {
                    let v = refs::resolve_value(&Value::String(cond.to_string()), &ctx)
                        .map_err(|source| ExecError::Ref { step: record_id.clone(), source })?;
                    if !truthy(&v) {
                        rec.skip_reason = Some(format!("{cond} was not set"));
                        return Ok(Value::Null);
                    }
                }
                let raw = Value::Object(step.inputs().clone().into_iter().collect());
                let inputs = refs::resolve_value(&raw, &ctx).map_err(|source| ExecError::Ref { step: record_id.clone(), source })?;
                rec.inputs = inputs.clone();

                // Continuing a run: anything it already did with these inputs is done, except
                // steps that only look at the world (a folder listing): those look again.
                // Review steps carry their decisions forward below instead.
                if let Some(prior) = req.resume_from.as_deref() {
                    if !req.edits.contains_key(&record_id) && self.resumable(step) {
                        if let Some(done) = prior_result(prior, &record_id, step, &ctx, &inputs) {
                            // A reused map keeps its inner records in this trace too, so the next
                            // continuation can still find what each item had done.
                            if matches!(step, Step::Map(_)) {
                                let prefix = format!("{record_id}[");
                                for inner in prior.steps.iter().filter(|r| r.step_id.starts_with(&prefix)) {
                                    let mut carried = inner.clone();
                                    carried.run_id = state.run_id.clone();
                                    carried.position = state.position;
                                    carried.reused_from = Some(prior.id.clone());
                                    state.position += 1;
                                    if let Some(o) = &state.observe {
                                        o(&carried);
                                    }
                                    state.records.push(carried);
                                }
                            }
                            rec.reused_from = Some(prior.id.clone());
                            rec.model = done.model.clone();
                            rec.prompt_path = done.prompt_path.clone();
                            rec.tool_calls = done.tool_calls.clone();
                            rec.inputs = done.inputs.clone();
                            return Ok(done.output.clone().unwrap_or(Value::Null));
                        }
                    }
                }

                match step {
                    Step::Map(m) => return self.run_map(m, &record_id, &ctx, &mut rec, state).await,
                    Step::Review(r) => {
                        let items = refs::resolve_value(&Value::String(r.items.clone()), &ctx)
                            .map_err(|source| ExecError::Ref { step: record_id.clone(), source })?;
                        let pinned = Value::Object(serde_json::Map::from_iter([("items".to_string(), items.clone())]));
                        rec.inputs = pinned.clone();
                        let edit = req.edits.get(&record_id).filter(|e| e.inputs_fingerprint == inputs_fingerprint(&pinned));
                        return match edit {
                            Some(edit) => {
                                rec.edited = true;
                                validate_against(&crate::recipe::REVIEW_OUTPUT_SCHEMA, &edit.output)
                                    .map_err(|message| ExecError::BadEdit { step: record_id.clone(), message })?;
                                Ok(edit.output.clone())
                            }
                            None => {
                                // Continuing a run: decisions already made on these very items stand, so a
                                // folder watch only asks about what is new.
                                let prior = req.resume_from.as_deref();
                                let decided_before = prior.and_then(|p| {
                                    p.steps.iter().find(|r| r.step_id == record_id && r.output.as_ref().is_some_and(|o| o["decided"] == Value::Bool(true))).and_then(|r| r.output.clone())
                                });
                                let (mut approved, mut rejected, mut undecided) = (Vec::new(), Vec::new(), 0usize);
                                for item in items.as_array().cloned().unwrap_or_default() {
                                    match &decided_before {
                                        Some(d) if contains(&d["approved"], &item) => approved.push(item),
                                        Some(d) if contains(&d["rejected"], &item) => rejected.push(item),
                                        _ => undecided += 1,
                                    }
                                }
                                if undecided == 0 && decided_before.is_some() {
                                    rec.reused_from = prior.map(|p| p.id.clone());
                                    return Ok(serde_json::json!({ "decided": true, "approved": approved, "rejected": rejected }));
                                }
                                state.needs_review = true;
                                if scope.parent.is_none() {
                                    state.held.extend(dependents(&req.recipe, step.id()));
                                }
                                let before = approved.len() + rejected.len();
                                let plural = |n: usize| if n == 1 { "" } else { "s" };
                                rec.skip_reason = Some(if before > 0 {
                                    format!("{undecided} new item{} waiting for your decision ({before} decided before)", plural(undecided))
                                } else {
                                    format!("{undecided} item{} waiting for your decision", plural(undecided))
                                });
                                Ok(serde_json::json!({ "decided": false, "items": items, "approved": approved, "rejected": rejected }))
                            }
                        };
                    }
                    _ => {}
                }

                // A user edit stands in for the step while the inputs it was made against hold.
                let edit = req.edits.get(&record_id).filter(|e| e.inputs_fingerprint == inputs_fingerprint(&inputs));
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
                        match step.output_schema().map(|s| validate_against(s, &edit.output)) {
                            Some(Err(message)) => Err(ExecError::BadEdit { step: record_id.clone(), message }),
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
                        let out = self.run_step(step, &record_id, req, &state.tool_ctx, &mut rec, &inputs).await?;
                        if let (Some(cache), Some(key), true) = (&self.cache, key.as_deref(), self.reusable(req, step)) {
                            cache.put(
                                key,
                                &CachedStep {
                                    run_id: state.run_id.clone(),
                                    step_id: record_id.clone(),
                                    model: rec.model.clone(),
                                    output: out.clone(),
                                    created_at: Utc::now(),
                                },
                            );
                        }
                        Ok(out)
                    }
                }
            }
            .await;

            let now = Utc::now();
            rec.finished_at = Some(now);
            rec.duration_ms = rec.started_at.map(|s| (now - s).num_milliseconds().max(0) as u64);
            match result {
                Ok(out) => {
                    rec.status = if rec.skip_reason.is_some() { StepStatus::Skipped } else { StepStatus::Succeeded };
                    rec.output = Some(out.clone());
                    if let Some(o) = &state.observe {
                        o(&rec);
                    }
                    state.records.push(rec);
                    Ok(out)
                }
                Err(e) => {
                    rec.status = StepStatus::Failed;
                    rec.error = Some(e.to_string());
                    if let Some(o) = &state.observe {
                        o(&rec);
                    }
                    state.records.push(rec);
                    Err(e)
                }
            }
        })
    }

    /// Run a map step: its inner steps once per item, in dependency order. Output is an array
    /// with one object per item, keyed by inner step id.
    async fn run_map<'a>(
        &self,
        m: &'a MapStep,
        record_id: &str,
        ctx: &Context<'_>,
        rec: &mut StepRecord,
        state: &mut RunState<'a>,
    ) -> Result<Value, ExecError> {
        let over = refs::resolve_value(&Value::String(m.over.clone()), ctx)
            .map_err(|source| ExecError::Ref { step: record_id.to_string(), source })?;
        let items = over
            .as_array()
            .ok_or_else(|| ExecError::NotAnArray { step: record_id.to_string(), got: kind_of(&over) })?
            .clone();
        rec.inputs = serde_json::json!({ "over": over });
        let order = Self::order_steps(&m.steps)?;
        let mut results = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let mut vars = ctx.vars.clone();
            vars.insert(m.item_name.clone(), item.clone());
            let mut local: HashMap<String, Value> = HashMap::new();
            for &j in &order {
                let sub = &m.steps[j];
                if matches!(sub, Step::Map(_) | Step::Review(_)) {
                    return Err(ExecError::Unsupported { step: sub.id().into(), kind: "nested map/review" });
                }
                let scope = Scope { vars: &vars, local: &local, parent: Some(record_id) };
                let sub_id = format!("{record_id}[{i}].{}", sub.id());
                let out = self.run_one(sub, sub_id, &scope, state).await?;
                local.insert(sub.id().to_string(), out);
            }
            let mut obj = serde_json::Map::new();
            obj.insert(m.item_name.clone(), item.clone());
            for sub in &m.steps {
                obj.insert(sub.id().to_string(), local.remove(sub.id()).unwrap_or(Value::Null));
            }
            results.push(Value::Object(obj));
        }
        Ok(Value::Array(results))
    }

    /// One model call with the run's timeout and stop handle applied.
    async fn chat_call(&self, req: &RunRequest, record_id: &str, model_id: &str, chat: ChatRequest) -> Result<ChatResponse, ExecError> {
        let timed = async {
            let r = match req.model_timeout {
                Some(d) => match tokio::time::timeout(d, self.models.chat(model_id, chat)).await {
                    Ok(r) => r,
                    Err(_) => return Err(ExecError::Timeout { step: record_id.into(), secs: d.as_secs() }),
                },
                None => self.models.chat(model_id, chat).await,
            };
            r.map_err(|e| ExecError::Model { step: record_id.into(), message: e.to_string() })
        };
        match &req.cancel {
            Some(c) => tokio::select! {
                r = timed => r,
                _ = c.cancelled() => Err(ExecError::Cancelled),
            },
            None => timed.await,
        }
    }

    async fn run_step(
        &self,
        step: &Step,
        record_id: &str,
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
                    .ok_or_else(|| ExecError::UnknownTool { step: record_id.into(), tool: t.tool.clone() })?;
                tool.call(tool_ctx, inputs).await.map_err(|e| ExecError::Tool { step: record_id.into(), message: e.to_string() })
            }
            Step::Model(m) => {
                let res = req
                    .resolutions
                    .get(&m.slot)
                    .ok_or_else(|| ExecError::UnresolvedSlot { step: record_id.into(), slot: m.slot.clone() })?;
                let system = req
                    .prompts
                    .get(&m.prompt)
                    .ok_or_else(|| ExecError::MissingPrompt { step: record_id.into(), path: m.prompt.clone() })?;
                rec.model = Some(res.model_id.clone());
                rec.prompt_path = Some(m.prompt.clone());

                let user = user_message(&inputs);
                let params = m.params.clone().unwrap_or_default();
                let chat = ChatRequest {
                    messages: vec![ChatMessage::new("system", system.clone()), ChatMessage::new("user", user)],
                    tools: Vec::new(),
                    json_schema: m.output_schema.clone(),
                    temperature: params.temperature,
                    max_tokens: params.max_tokens,
                    context_length: params.context_length.or(req.model_defaults.context_length),
                    think: params.think.clone().or_else(|| req.model_defaults.think.clone()),
                };
                let resp = self.chat_call(req, record_id, &res.model_id, chat).await?;
                rec.prompt_tokens = resp.prompt_tokens;
                rec.completion_tokens = resp.completion_tokens;
                structured(record_id, m.output_schema.as_ref(), resp.content)
            }
            Step::Agent(a) => self.run_agent(a, record_id, req, tool_ctx, rec, &inputs).await,
            other => Err(ExecError::Unsupported { step: record_id.into(), kind: other.kind_name() }),
        }
    }

    /// The agent loop: the model may call allowed tools until it answers without one.
    async fn run_agent(
        &self,
        a: &AgentStep,
        record_id: &str,
        req: &RunRequest,
        tool_ctx: &ToolContext,
        rec: &mut StepRecord,
        inputs: &Value,
    ) -> Result<Value, ExecError> {
        let res = req
            .resolutions
            .get(&a.slot)
            .ok_or_else(|| ExecError::UnresolvedSlot { step: record_id.into(), slot: a.slot.clone() })?;
        let system = req
            .prompts
            .get(&a.prompt)
            .ok_or_else(|| ExecError::MissingPrompt { step: record_id.into(), path: a.prompt.clone() })?;
        rec.model = Some(res.model_id.clone());
        rec.prompt_path = Some(a.prompt.clone());

        let mut specs = Vec::new();
        for id in &a.allowed_tools {
            let tool = self.tools.get(id).ok_or_else(|| ExecError::UnknownTool { step: record_id.into(), tool: id.clone() })?;
            let d = tool.describe().ok_or_else(|| ExecError::ToolNotDescribable { step: record_id.into(), tool: id.clone() })?;
            specs.push(ToolSpec { name: id.clone(), description: d.description, parameters: d.parameters });
        }
        let envelope = format!(
            "{}\n\n{}",
            system.trim_end(),
            AGENT_ENVELOPE.replace("{tools}", &a.allowed_tools.join(", "))
        );
        let params = a.params.clone().unwrap_or_default();
        let mut messages = vec![ChatMessage::new("system", envelope), ChatMessage::new("user", user_message(inputs))];
        let mut prompt_tokens = 0u64;
        let mut completion_tokens = 0u64;
        let mut calls_made = 0u32;

        for _turn in 0..a.max_turns {
            let chat = ChatRequest {
                messages: messages.clone(),
                tools: specs.clone(),
                json_schema: None,
                temperature: params.temperature,
                max_tokens: params.max_tokens,
                context_length: params.context_length.or(req.model_defaults.context_length),
                think: params.think.clone().or_else(|| req.model_defaults.think.clone()),
            };
            let resp = self.chat_call(req, record_id, &res.model_id, chat).await?;
            prompt_tokens += resp.prompt_tokens.unwrap_or(0);
            completion_tokens += resp.completion_tokens.unwrap_or(0);
            rec.prompt_tokens = Some(prompt_tokens);
            rec.completion_tokens = Some(completion_tokens);

            if resp.tool_calls.is_empty() {
                // Final answer. If a schema is required and the text is not valid for it, ask
                // once more with the schema enforced.
                let content = resp.content;
                let Some(schema) = a.output_schema.as_ref() else { return Ok(Value::String(content)) };
                if let Ok(v) = serde_json::from_str::<Value>(&content) {
                    if validate_against(schema, &v).is_ok() {
                        return Ok(v);
                    }
                }
                messages.push(ChatMessage::new("assistant", content));
                messages.push(ChatMessage::new("user", "Now give the final answer as JSON matching the required schema, nothing else."));
                let chat = ChatRequest {
                    messages: messages.clone(),
                    tools: Vec::new(),
                    json_schema: Some(schema.clone()),
                    temperature: params.temperature,
                    max_tokens: params.max_tokens,
                    context_length: params.context_length.or(req.model_defaults.context_length),
                    think: Some(Value::Bool(false)),
                };
                let resp = self.chat_call(req, record_id, &res.model_id, chat).await?;
                rec.prompt_tokens = Some(prompt_tokens + resp.prompt_tokens.unwrap_or(0));
                rec.completion_tokens = Some(completion_tokens + resp.completion_tokens.unwrap_or(0));
                return structured(record_id, Some(schema), resp.content);
            }

            messages.push(ChatMessage {
                role: "assistant".into(),
                content: resp.content.clone(),
                tool_calls: resp.tool_calls.clone(),
                tool_name: None,
            });
            for call in resp.tool_calls {
                let started = std::time::Instant::now();
                let result: Result<Value, String> = if calls_made >= a.max_tool_calls {
                    Err(format!("tool call budget of {} exhausted; finish with what you have", a.max_tool_calls))
                } else if !a.allowed_tools.contains(&call.name) {
                    Err(format!("tool {} is not allowed in this step; allowed: {}", call.name, a.allowed_tools.join(", ")))
                } else {
                    calls_made += 1;
                    match self.tools.get(&call.name) {
                        Some(tool) => tool.call(tool_ctx, call.arguments.clone()).await.map_err(|e| e.to_string()),
                        None => Err(format!("unknown tool {}", call.name)),
                    }
                };
                let content = match &result {
                    Ok(v) => serde_json::to_string(v).unwrap_or_default(),
                    Err(e) => serde_json::json!({ "error": e }).to_string(),
                };
                rec.tool_calls.push(ToolCallRecord {
                    tool: call.name.clone(),
                    inputs: call.arguments,
                    output: result.as_ref().ok().cloned(),
                    error: result.err(),
                    duration_ms: started.elapsed().as_millis() as u64,
                });
                messages.push(ChatMessage { role: "tool".into(), content, tool_calls: Vec::new(), tool_name: Some(call.name) });
            }
        }
        Err(ExecError::AgentTurns { step: record_id.into(), turns: a.max_turns })
    }
}

/// The prior run's record for this step, if it succeeded on the same inputs. Map steps are
/// matched on the resolved array they iterate.
fn prior_result<'p>(prior: &'p Run, record_id: &str, step: &Step, ctx: &Context<'_>, inputs: &Value) -> Option<&'p StepRecord> {
    let current = match step {
        Step::Map(m) => {
            let over = refs::resolve_value(&Value::String(m.over.clone()), ctx).ok()?;
            serde_json::json!({ "over": over })
        }
        _ => inputs.clone(),
    };
    let want = inputs_fingerprint(&current);
    // Steps inside a map are matched by what they did, not by their position: the list they
    // ran over may have grown or been reordered since (a folder watch sees this every time).
    prior
        .steps
        .iter()
        .find(|r| same_step(&r.step_id, record_id) && r.status == StepStatus::Succeeded && r.output.is_some() && inputs_fingerprint(&r.inputs) == want)
}

/// `each[3].parse` and `each[0].parse` are the same step at different positions.
fn same_step(a: &str, b: &str) -> bool {
    a == b || strip_indices(a) == strip_indices(b)
}

fn strip_indices(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    let mut in_index = false;
    for c in id.chars() {
        match c {
            '[' => { in_index = true; out.push(c); }
            ']' => { in_index = false; out.push(c); }
            _ if in_index && c.is_ascii_digit() => {}
            _ => out.push(c),
        }
    }
    out
}

/// Whether a JSON array holds an element equal to `item`.
fn contains(list: &Value, item: &Value) -> bool {
    list.as_array().is_some_and(|a| a.iter().any(|x| x == item))
}

/// Appended to every agent step's prompt. The step envelope in the Prompt Stack.
pub const AGENT_ENVELOPE: &str = "You may call these tools: {tools}. Call a tool when you need information or need to act; \
each call's result comes back to you. Keep going until the task is done, then reply with the final answer and no tool call. \
Do not invent paths or facts the tools did not give you.";

fn kind_of(v: &Value) -> String {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
    .to_string()
}

fn structured(record_id: &str, schema: Option<&Value>, content: String) -> Result<Value, ExecError> {
    match schema {
        Some(schema) => {
            let value = serde_json::from_str::<Value>(&content)
                .map_err(|e| ExecError::BadStructuredOutput { step: record_id.into(), message: e.to_string() })?;
            validate_against(schema, &value).map_err(|message| ExecError::BadStructuredOutput { step: record_id.into(), message })?;
            Ok(value)
        }
        None => Ok(Value::String(content)),
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
        Err(ExecError::Cancelled) => {
            run.status = RunStatus::Cancelled;
            run.error = Some(ExecError::Cancelled.to_string());
        }
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

    #[tokio::test]
    async fn a_cancelled_run_stops_before_its_next_step() {
        let recipe = Recipe::from_yaml(
            r#"
schemaVersion: "0"
id: two
name: Two steps
inputs: {}
steps:
  - id: a
    kind: tool
    tool: list.filter
    inputs: { items: [{ x: 1 }], path: x }
  - id: b
    kind: tool
    tool: list.filter
    inputs: { items: "${steps.a.output.items}", path: x }
outputs:
  out: { from: "${steps.b.output}", type: json }
"#,
        )
        .unwrap();
        let cancel = Cancel::new();
        cancel.cancel();
        let req = RunRequest {
            recipe,
            prompts: HashMap::new(),
            inputs: IndexMap::new(),
            resolutions: BTreeMap::new(),
            artifact_base: None,
            reuse: false,
            edits: BTreeMap::new(),
            resume_from: None,
            trigger: None,
            cancel: Some(cancel),
            model_timeout: None,
            model_defaults: Default::default(),
        };
        let exec = Executor::new(Arc::new(crate::tools::ToolRegistry::builtin()), Arc::new(NoModels));
        let run = exec.run(req, None).await;
        assert_eq!(run.status, RunStatus::Cancelled);
        assert!(run.steps.is_empty(), "no step should have started: {:?}", run.steps);
        assert_eq!(run.error.as_deref(), Some("stopped by you"));
    }

    struct NoModels;
    #[async_trait::async_trait]
    impl ModelClient for NoModels {
        async fn chat(&self, _model_id: &str, _req: ChatRequest) -> Result<crate::model::ChatResponse, crate::model::ModelError> {
            Err(crate::model::ModelError::Remote("no models in this test".into()))
        }
    }
    use crate::model::{ChatResponse, ModelError};
    use crate::resolve::ResolvedVia;
    use serde_json::json;
    use std::io::Write;

    struct EchoModel;
    #[async_trait::async_trait]
    impl ModelClient for EchoModel {
        async fn chat(&self, model_id: &str, req: ChatRequest) -> Result<ChatResponse, ModelError> {
            let user = &req.messages.last().unwrap().content;
            Ok(ChatResponse {
                content: format!("[{model_id}] {}", user.trim()),
                tool_calls: Vec::new(),
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
            resume_from: None,
            trigger: None,
            cancel: None,
            model_timeout: None,
            model_defaults: Default::default(),
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
            resume_from: None,
            trigger: None,
            cancel: None,
            model_timeout: None,
            model_defaults: Default::default(),
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
inputs:
  file: { type: file }
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
            resume_from: None,
            trigger: None,
            cancel: None,
            model_timeout: None,
            model_defaults: Default::default(),
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

    /// A model that classifies by whether the subject has a question mark, drafts a reply,
    /// and as an agent lists a folder before answering.
    struct TriageModel;
    #[async_trait::async_trait]
    impl ModelClient for TriageModel {
        async fn chat(&self, _model_id: &str, req: ChatRequest) -> Result<ChatResponse, ModelError> {
            let system = &req.messages[0].content;
            let last = req.messages.last().unwrap();
            let reply = |content: String, tool_calls: Vec<crate::model::ToolCall>| ChatResponse { content, tool_calls, prompt_tokens: Some(1), completion_tokens: Some(1) };
            if system.starts_with("CLASSIFY") {
                let needs = last.content.contains('?');
                return Ok(reply(format!("{{\"needsReply\": {needs}, \"category\": \"{}\"}}", if needs { "question" } else { "fyi" }), vec![]));
            }
            if system.starts_with("DRAFT") {
                return Ok(reply("Thanks for asking. Yes.".into(), vec![]));
            }
            if system.starts_with("AGENT") {
                // Turn 1: one disallowed call and one allowed call. Turn 2: the answer.
                if last.role == "user" {
                    let dir = last.content.trim().to_string();
                    return Ok(reply(
                        String::new(),
                        vec![
                            crate::model::ToolCall { name: "fs.write".into(), arguments: serde_json::json!({ "path": "/x", "text": "no" }) },
                            crate::model::ToolCall { name: "fs.list".into(), arguments: serde_json::json!({ "path": dir }) },
                        ],
                    ));
                }
                let listing: Value = serde_json::from_str(&last.content).unwrap();
                return Ok(reply(format!("{{\"count\": {}}}", listing["count"]), vec![]));
            }
            Ok(reply("?".into(), vec![]))
        }
    }

    const TRIAGE_YAML: &str = r#"
schemaVersion: "0"
id: triage-mini
name: Triage mini
inputs:
  inbox: { type: directory }
  outbox: { type: directory }
modelSlots:
  triager: { requires: [text] }
permissions:
  filesystem:
    read: [input]
    write: ["${inputs.outbox}"]
steps:
  - id: list
    kind: tool
    tool: fs.list
    inputs: { path: "${inputs.inbox}", extensions: [eml] }
  - id: each
    kind: map
    over: "${steps.list.output.entries}"
    as: email
    steps:
      - id: parse
        kind: tool
        tool: email.parse
        inputs: { path: "${email.path}" }
      - id: classify
        kind: model
        slot: triager
        prompt: prompts/classify.md
        inputs: { email: "${steps.parse.output}" }
        outputSchema: { type: object, required: [needsReply], properties: { needsReply: { type: boolean } } }
      - id: draft
        kind: model
        slot: triager
        prompt: prompts/draft.md
        when: "${steps.classify.output.needsReply}"
        inputs: { email: "${steps.parse.output}" }
  - id: with_drafts
    kind: tool
    tool: list.filter
    inputs: { items: "${steps.each.output}", path: draft }
  - id: review
    kind: review
    title: Replies to send
    items: "${steps.with_drafts.output.items}"
    show: [parse.subject, draft]
  - id: send
    kind: map
    over: "${steps.review.output.approved}"
    as: reply
    steps:
      - id: write
        kind: tool
        tool: email.write_draft
        inputs: { dir: "${inputs.outbox}", to: "${reply.parse.from}", subject: "${reply.parse.subject}", body: "${reply.draft}" }
outputs:
  triage: { from: "${steps.each.output}", type: json }
  sent: { from: "${steps.send.output}", type: json }
"#;

    fn triage_request(dir: &std::path::Path) -> RunRequest {
        RunRequest {
            recipe: Recipe::from_yaml(TRIAGE_YAML).unwrap(),
            prompts: HashMap::from([
                ("prompts/classify.md".to_string(), "CLASSIFY".to_string()),
                ("prompts/draft.md".to_string(), "DRAFT".to_string()),
            ]),
            inputs: IndexMap::from([
                ("inbox".to_string(), Value::String(dir.join("inbox").to_string_lossy().into())),
                ("outbox".to_string(), Value::String(dir.join("outbox").to_string_lossy().into())),
            ]),
            resolutions: BTreeMap::from([(
                "triager".to_string(),
                Resolution { slot: "triager".into(), model_id: "test:triage".into(), via: ResolvedVia::Capability },
            )]),
            artifact_base: Some(dir.to_path_buf()),
            reuse: true,
            edits: BTreeMap::new(),
            resume_from: None,
            trigger: None,
            cancel: None,
            model_timeout: None,
            model_defaults: Default::default(),
        }
    }

    fn eml(from: &str, subject: &str, body: &str) -> String {
        format!("From: {from}\r\nTo: me@example.com\r\nSubject: {subject}\r\nMessage-ID: <{}@x>\r\nContent-Type: text/plain\r\n\r\n{body}\r\n", subject.len())
    }

    #[tokio::test]
    async fn map_when_review_and_approval_flow() {
        use crate::cache::MemoryCache;
        let dir = std::env::temp_dir().join(format!("stillroom-triage-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::create_dir_all(dir.join("outbox")).unwrap();
        std::fs::write(dir.join("inbox/a.eml"), eml("ana@example.com", "Is invoice 42 paid?", "Hi")).unwrap();
        std::fs::write(dir.join("inbox/b.eml"), eml("news@example.com", "October newsletter", "Read all about it")).unwrap();
        std::fs::write(dir.join("inbox/notes.txt"), "not an email").unwrap();
        let exec = Executor::new(Arc::new(ToolRegistry::builtin()), Arc::new(TriageModel)).with_cache(Arc::new(MemoryCache::default()));

        let first = exec.run(triage_request(&dir), None).await;
        assert_eq!(first.status, RunStatus::NeedsReview, "{:?}", first.error);
        let by_id = |run: &Run, id: &str| run.steps.iter().find(|s| s.step_id == id).cloned().unwrap_or_else(|| panic!("no step {id} in {:?}", run.steps.iter().map(|s| s.step_id.clone()).collect::<Vec<_>>()));
        assert_eq!(by_id(&first, "each").output.as_ref().unwrap().as_array().unwrap().len(), 2, "two emails, not the txt");
        assert_eq!(by_id(&first, "each[0].parse").parent.as_deref(), Some("each"));
        assert_eq!(by_id(&first, "each[0].draft").status, StepStatus::Succeeded);
        assert_eq!(by_id(&first, "each[1].draft").status, StepStatus::Skipped, "newsletter needs no reply");
        assert!(by_id(&first, "each[1].draft").skip_reason.as_deref().unwrap().contains("needsReply"));
        assert_eq!(by_id(&first, "with_drafts").output.as_ref().unwrap()["kept"], 1);
        let review = by_id(&first, "review");
        assert_eq!(review.status, StepStatus::Skipped);
        assert_eq!(review.output.as_ref().unwrap()["decided"], false);
        assert_eq!(review.output.as_ref().unwrap()["items"].as_array().unwrap().len(), 1);
        assert_eq!(by_id(&first, "send").skip_reason.as_deref(), Some("waiting for review"));
        assert_eq!(first.outputs["sent"], Value::Null);
        assert!(std::fs::read_dir(dir.join("outbox")).unwrap().next().is_none(), "nothing written before approval");

        // The user approves the one reply.
        let item = review.output.as_ref().unwrap()["items"][0].clone();
        let decision = serde_json::json!({ "decided": true, "approved": [item], "rejected": [] });
        let mut req = triage_request(&dir);
        req.edits = crate::propagate::edits_for_rerun(&first, "review", decision).unwrap();
        req.resume_from = Some(Box::new(first.clone()));
        let second = exec.run(req, None).await;
        assert_eq!(second.status, RunStatus::Succeeded, "{:?}", second.error);
        assert!(by_id(&second, "review").edited);
        assert_eq!(by_id(&second, "each").reused_from.as_deref(), Some(first.id.as_str()), "the whole map is resumed, not re-run");
        let inner: Vec<&StepRecord> = second.steps.iter().filter(|s| s.step_id.starts_with("each[")).collect();
        assert!(!inner.is_empty() && inner.iter().all(|s| s.reused_from.as_deref() == Some(first.id.as_str())), "inner steps carried over, none re-executed");
        assert!(by_id(&second, "list").reused_from.is_none(), "the folder is listed again: it may have changed");
        let sent = &second.outputs["sent"];
        assert_eq!(sent.as_array().unwrap().len(), 1, "{sent}");
        let written = std::path::PathBuf::from(sent[0]["write"]["path"].as_str().unwrap());
        assert!(written.starts_with(dir.join("outbox").canonicalize().unwrap()), "{written:?}");
        assert!(std::fs::read_to_string(&written).unwrap().contains("Thanks for asking. Yes."));
        std::fs::remove_dir_all(dir).ok();
    }

    /// A folder watch runs the same recipe over a folder again and again. Each run continues the
    /// last: steps that handled a file before are reused wherever the file now sits in the list,
    /// and the review only asks about items it has not decided.
    #[tokio::test]
    async fn continuing_over_a_grown_folder_reuses_by_content_and_asks_only_about_new_items() {
        use crate::cache::MemoryCache;
        let dir = std::env::temp_dir().join(format!("stillroom-watchrun-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::create_dir_all(dir.join("outbox")).unwrap();
        std::fs::write(dir.join("inbox/a.eml"), eml("ana@example.com", "Is invoice 42 paid?", "Hi")).unwrap();
        std::fs::write(dir.join("inbox/b.eml"), eml("news@example.com", "October newsletter", "Read all about it")).unwrap();
        let exec = Executor::new(Arc::new(ToolRegistry::builtin()), Arc::new(TriageModel)).with_cache(Arc::new(MemoryCache::default()));
        let by_id = |run: &Run, id: &str| run.steps.iter().find(|s| s.step_id == id).cloned().unwrap_or_else(|| panic!("no step {id}"));

        let first = exec.run(triage_request(&dir), None).await;
        assert_eq!(first.status, RunStatus::NeedsReview);
        let item_a = by_id(&first, "review").output.as_ref().unwrap()["items"][0].clone();
        let mut req = triage_request(&dir);
        req.edits = crate::propagate::edits_for_rerun(&first, "review", serde_json::json!({ "decided": true, "approved": [item_a.clone()], "rejected": [] })).unwrap();
        req.resume_from = Some(Box::new(first.clone()));
        let second = exec.run(req, None).await;
        assert_eq!(second.status, RunStatus::Succeeded, "{:?}", second.error);
        assert_eq!(std::fs::read_dir(dir.join("outbox")).unwrap().count(), 1);

        // A new email lands, and it sorts first, so every earlier item moves down one slot.
        std::fs::write(dir.join("inbox/0c.eml"), eml("cat@example.com", "Can we meet?", "Soon?")).unwrap();
        let mut req = triage_request(&dir);
        req.resume_from = Some(Box::new(second.clone()));
        let third = exec.run(req, None).await;
        assert_eq!(third.status, RunStatus::NeedsReview, "{:?}", third.error);
        let entries = by_id(&third, "list").output.as_ref().unwrap()["entries"].clone();
        assert!(entries[0]["path"].as_str().unwrap().ends_with("0c.eml"), "{entries}");
        assert!(by_id(&third, "each[0].parse").reused_from.is_none(), "the new email is parsed");
        assert!(by_id(&third, "each[1].parse").reused_from.is_none(), "a.eml is read again (it may have changed)");
        assert_eq!(by_id(&third, "each[1].classify").reused_from.as_deref(), Some(second.id.as_str()), "a.eml's sorting reused from its new position");
        assert_eq!(by_id(&third, "each[2].classify").reused_from.as_deref(), Some(second.id.as_str()), "b.eml too");
        let review = by_id(&third, "review");
        assert_eq!(review.skip_reason.as_deref(), Some("1 new item waiting for your decision (1 decided before)"));
        let out = review.output.as_ref().unwrap();
        assert_eq!(out["approved"].as_array().unwrap(), &vec![item_a.clone()], "the earlier decision stands");
        assert_eq!(out["items"].as_array().unwrap().len(), 2);

        // Approve the new one too: only its draft is written, a's draft is not written twice.
        let item_c = out["items"].as_array().unwrap().iter().find(|i| **i != item_a).unwrap().clone();
        let mut req = triage_request(&dir);
        req.edits = crate::propagate::edits_for_rerun(&third, "review", serde_json::json!({ "decided": true, "approved": [item_a.clone(), item_c], "rejected": [] })).unwrap();
        // The paused run held `send` back, so its records come from the run before it.
        assert_eq!(third.continues.as_deref(), Some(second.id.as_str()));
        let mut base = third.clone();
        base.absorb(&second);
        req.resume_from = Some(Box::new(base));
        let fourth = exec.run(req, None).await;
        assert_eq!(fourth.status, RunStatus::Succeeded, "{:?}", fourth.error);
        assert_eq!(by_id(&fourth, "send[0].write").reused_from.as_deref(), Some(third.id.as_str()), "a's draft reused through the chain");
        assert_eq!(by_id(&fourth, "send[0].write").inputs["subject"], "Is invoice 42 paid?");
        assert!(by_id(&fourth, "send[1].write").reused_from.is_none());
        assert_eq!(std::fs::read_dir(dir.join("outbox")).unwrap().count(), 2);

        // Nothing new: the review carries every decision forward and the run finishes on its own.
        let mut req = triage_request(&dir);
        req.resume_from = Some(Box::new(fourth.clone()));
        let fifth = exec.run(req, None).await;
        assert_eq!(fifth.status, RunStatus::Succeeded, "{:?}", fifth.error);
        assert_eq!(by_id(&fifth, "review").reused_from.as_deref(), Some(fourth.id.as_str()));
        assert_eq!(std::fs::read_dir(dir.join("outbox")).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn agent_calls_allowed_tools_and_is_refused_others() {
        let dir = std::env::temp_dir().join(format!("stillroom-agent-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(dir.join("archive/Clients")).unwrap();
        std::fs::create_dir_all(dir.join("archive/Finance")).unwrap();
        let yaml = r#"
schemaVersion: "0"
id: agent-mini
name: Agent mini
inputs:
  archive: { type: directory }
modelSlots:
  filer: { requires: [text, tool_calling] }
steps:
  - id: look
    kind: agent
    slot: filer
    prompt: prompts/agent.md
    allowedTools: [fs.list]
    maxTurns: 3
    inputs: { dir: "${inputs.archive}" }
    outputSchema: { type: object, required: [count], properties: { count: { type: integer } } }
outputs:
  result: { from: "${steps.look.output}", type: json }
"#;
        let req = RunRequest {
            recipe: Recipe::from_yaml(yaml).unwrap(),
            prompts: HashMap::from([("prompts/agent.md".to_string(), "AGENT".to_string())]),
            inputs: IndexMap::from([("archive".to_string(), Value::String(dir.join("archive").to_string_lossy().into()))]),
            resolutions: BTreeMap::from([(
                "filer".to_string(),
                Resolution { slot: "filer".into(), model_id: "test:agent".into(), via: ResolvedVia::Capability },
            )]),
            artifact_base: Some(dir.clone()),
            reuse: false,
            edits: BTreeMap::new(),
            resume_from: None,
            trigger: None,
            cancel: None,
            model_timeout: None,
            model_defaults: Default::default(),
        };
        let exec = Executor::new(Arc::new(ToolRegistry::builtin()), Arc::new(TriageModel));
        let run = exec.run(req, None).await;
        assert_eq!(run.status, RunStatus::Succeeded, "{:?}", run.error);
        let rec = &run.steps[0];
        assert_eq!(rec.tool_calls.len(), 2);
        assert!(rec.tool_calls[0].error.as_deref().unwrap().contains("not allowed"));
        assert_eq!(rec.tool_calls[1].tool, "fs.list");
        assert_eq!(run.outputs["result"]["count"], 2);
        assert_eq!(rec.prompt_tokens, Some(2), "tokens summed over turns");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn net_policy_takes_hosts_from_link_inputs_and_patterns() {
        let yaml = r#"
schemaVersion: "0"
id: n
name: N
inputs:
  links: { type: urls }
  home: { type: url }
permissions:
  network: [input, "*.example.org", "${inputs.home}"]
steps:
  - id: a
    kind: tool
    tool: http.fetch
    inputs: { url: "${inputs.home}" }
outputs:
  a: { from: "${steps.a.output}", type: json }
"#;
        let recipe = Recipe::from_yaml(yaml).unwrap();
        let mut inputs = IndexMap::new();
        inputs.insert("links".to_string(), json!(["https://En.Wikipedia.org/wiki/A", "http://docs.rs/x"]));
        inputs.insert("home".to_string(), json!("https://stillroom.test/"));
        let p = net_policy(&recipe, &inputs);
        assert!(p.allows("en.wikipedia.org"));
        assert!(p.allows("docs.rs"));
        assert!(p.allows("stillroom.test"));
        assert!(p.allows("api.example.org"));
        assert!(!p.allows("example.com"));
        let none = net_policy(&Recipe::from_yaml(&yaml.replace("permissions:\n  network: [input, \"*.example.org\", \"${inputs.home}\"]\n", "")).unwrap(), &inputs);
        assert!(!none.allows("en.wikipedia.org"), "no network permission means no hosts");
    }

    #[test]
    fn fs_policy_expands_input_output_and_refs() {
        let yaml = r#"
schemaVersion: "0"
id: p
name: P
inputs:
  inbox: { type: directory }
  archive: { type: directory }
  note: { type: string }
permissions:
  filesystem:
    read: [input]
    write: ["${inputs.archive}", output, "~/Drafts"]
steps:
  - id: a
    kind: tool
    tool: fs.list
    inputs: { path: "${inputs.inbox}" }
outputs:
  o: { from: "${steps.a.output}", type: json }
"#;
        let recipe = Recipe::from_yaml(yaml).unwrap();
        let inputs = IndexMap::from([
            ("inbox".to_string(), Value::String("/tmp/in".into())),
            ("archive".to_string(), Value::String("/tmp/arc".into())),
            ("note".to_string(), Value::String("/etc".into())),
        ]);
        let p = fs_policy(&recipe, &inputs, std::path::Path::new("/tmp/art"));
        assert_eq!(p.read_roots.len(), 2, "only path-typed inputs: {:?}", p.read_roots);
        assert!(!p.read_roots.iter().any(|r| r.ends_with("etc")));
        assert_eq!(p.write_roots.len(), 3);
        assert!(p.write_roots.iter().any(|r| r.ends_with("Drafts")));
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
