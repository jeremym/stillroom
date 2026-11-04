//! Stillroom desktop shell: wires the recipe library, executor, model router, and store to the webview.

mod plan_journal;
mod schedules;
mod updates;
mod watches;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use stillroom_core::model::{Hardware, ModelClient};
use stillroom_core::store::{ModelUsage, PlannerMeasurement, RunEstimate};
use stillroom_core::tools::{ToolInfo, ToolRegistry};
use stillroom_core::trace::RunSummary;
use stillroom_core::keep::{keep_dir, JournalEntry, Keep, KeepRecord};
use stillroom_core::planner::{planner_recipe, trial_request, PLANNER_SLOT};
use stillroom_core::{
    consent, edits_for_rerun, pins_from_run, planning_reason, rank_for_planning, resolve_slots_with, stage_fingerprint, stage_models, validate_edit, Cancel, Draft, Executor, Finding, Grant, Measured,
    ModelDefaults, PlanEvent, PlanOutcome, PlanRequest, Planner, PlanningFacts, PromptState, RecipeBundle, RecipeInfo, RecipeLibrary, RecipeSource, ResolveDefaults, ResolvedVia, Run, RunTrigger,
    Schedule, ShelfState, StepEdit, StepRecord, Store, SystemPrompts, Watch,
};
use stillroom_models::codex::Codex;
use stillroom_models::ollama::Ollama;
use stillroom_models::openai::OpenAi;
use stillroom_models::{InstalledModel, ModelRouter, ProviderStatus};
use tauri::{AppHandle, Emitter, Manager, State};
use schedules::{ScheduleRuntime, ScheduleView};
use watches::{WatchRuntime, WatchView};

pub(crate) struct AppState {
    store: Arc<Mutex<Store>>,
    /// Secrets read from the keychain once per process, so a run does not knock on it every time.
    secrets: Mutex<BTreeMap<String, Option<String>>>,
    /// The stop handle of the run in flight, if any. One run at a time.
    current_cancel: Mutex<Option<Cancel>>,
    /// The stop handle of the planner drafting a recipe, if any. Its own slot: stopping a plan
    /// must not take the run's handle with it, nor leave the run unstoppable.
    plan_cancel: Mutex<Option<Cancel>>,
    tools: Arc<ToolRegistry>,
    library: RecipeLibrary,
    /// The engine's own prompts (the planner), with the user's revisions on disk.
    prompts: SystemPrompts,
    db_path: String,
    artifacts_dir: PathBuf,
    /// Where run keeps live: `<data>/recipe-data/<recipe id>/<stage id>`.
    keep_base: PathBuf,
    /// Ready-made stages the planner may use.
    stage_library: Arc<stillroom_core::StageLibrary>,
    /// One run at a time: a watch firing while the user runs something waits its turn.
    run_gate: tokio::sync::Mutex<()>,
    /// Where a recipe being got from a folder or zip waits, checked, for the user's yes.
    quarantine_dir: PathBuf,
    /// Physical RAM of this Mac, read once at startup; `None` when it could not be read.
    memory_bytes: Option<u64>,
    /// The planner's journal: one Markdown file per plan, `<data>/plans/`.
    plans_dir: PathBuf,
}

/// The setting under `models` that holds what each model managed last time: `speed.<model id>`.
const MODELS_SETTING: &str = "models";
const SPEED_FIELD: &str = "speed";

/// What one model managed last time it answered on this Mac.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelSpeed {
    tokens_per_second: f64,
    /// When, RFC 3339.
    at: String,
}

/// Every model call the shell makes goes through this: it answers from the router and, when the
/// provider said how fast the model wrote, remembers it under `models.speed.<model id>`. One place,
/// so a run, a plan, Try, and Try planning all keep the table fresh.
struct SpeedRecorder {
    inner: Arc<ModelRouter>,
    store: Arc<Mutex<Store>>,
}

impl SpeedRecorder {
    fn record(&self, model_id: &str, resp: &stillroom_core::model::ChatResponse) {
        let Some(tps) = resp.completion_tokens_per_second.filter(|t| t.is_finite() && *t > 0.0) else { return };
        if let Ok(store) = self.store.lock() {
            if let Err(e) = record_speed(&store, model_id, tps) {
                tracing::warn!("could not record {model_id}'s speed: {e}");
            }
        }
    }
}

/// Write `models.speed.<model id>` without touching the rest of `models`.
fn record_speed(store: &Store, model_id: &str, tokens_per_second: f64) -> Result<(), stillroom_core::store::StoreError> {
    let mut models = match store.get_setting(MODELS_SETTING)? {
        Some(Value::Object(o)) => o,
        _ => serde_json::Map::new(),
    };
    let speed = models.entry(SPEED_FIELD).or_insert_with(|| Value::Object(Default::default()));
    if !speed.is_object() {
        *speed = Value::Object(Default::default());
    }
    let entry = ModelSpeed { tokens_per_second: (tokens_per_second * 10.0).round() / 10.0, at: chrono::Utc::now().to_rfc3339() };
    speed.as_object_mut().expect("just made an object").insert(model_id.to_string(), serde_json::to_value(entry)?);
    store.set_setting(MODELS_SETTING, &Value::Object(models))
}

/// `models.speed`, by model id.
fn speeds_of(store: &Store) -> BTreeMap<String, ModelSpeed> {
    store
        .get_setting(MODELS_SETTING)
        .ok()
        .flatten()
        .and_then(|m| m.get(SPEED_FIELD).cloned())
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

#[async_trait::async_trait]
impl ModelClient for SpeedRecorder {
    async fn chat(&self, model_id: &str, req: stillroom_core::model::ChatRequest) -> Result<stillroom_core::model::ChatResponse, stillroom_core::model::ModelError> {
        let resp = self.inner.chat(model_id, req).await?;
        self.record(model_id, &resp);
        Ok(resp)
    }
    async fn chat_stream(
        &self,
        model_id: &str,
        req: stillroom_core::model::ChatRequest,
        on_delta: &(dyn for<'a> Fn(&'a str) + Send + Sync),
    ) -> Result<stillroom_core::model::ChatResponse, stillroom_core::model::ModelError> {
        let resp = self.inner.chat_stream(model_id, req, on_delta).await?;
        self.record(model_id, &resp);
        Ok(resp)
    }
    async fn embed(&self, model_id: &str, texts: Vec<String>) -> Result<Vec<Vec<f32>>, stillroom_core::model::ModelError> {
        self.inner.embed(model_id, texts).await
    }
}

impl AppState {
    /// Every setting as one object; missing keys mean defaults.
    fn settings(&self) -> CmdResult<serde_json::Map<String, Value>> {
        self.store.lock().map_err(err)?.settings().map_err(err)
    }

    fn ollama_url(&self) -> String {
        self.settings()
            .ok()
            .and_then(|s| s.get("ollama").and_then(|o| o.get("baseUrl")).and_then(Value::as_str).map(String::from))
            .filter(|u| !u.trim().is_empty())
            .unwrap_or_else(|| "http://localhost:11434".to_string())
    }

    fn ollama(&self) -> Ollama {
        Ollama::with_key(self.ollama_url(), self.secret(OLLAMA_KEY))
    }

    /// A secret from the keychain, cached after the first read. `None` when unset or unreadable.
    fn secret(&self, name: &str) -> Option<String> {
        if let Ok(cache) = self.secrets.lock() {
            if let Some(v) = cache.get(name) {
                return v.clone();
            }
        }
        let v = keyring::Entry::new(KEYCHAIN_SERVICE, name).ok().and_then(|e| e.get_password().ok()).filter(|k| !k.is_empty());
        if let Ok(mut cache) = self.secrets.lock() {
            cache.insert(name.to_string(), v.clone());
        }
        v
    }

    /// The Codex CLI as configured (`codex.path`, `codex.models`), whether or not it is turned on.
    fn codex(&self) -> Codex {
        let settings = self.settings().ok();
        let codex = settings.as_ref().and_then(|s| s.get("codex"));
        let path = codex.and_then(|c| c.get("path")).and_then(Value::as_str).map(str::trim).filter(|p| !p.is_empty()).map(PathBuf::from);
        let models = codex
            .and_then(|c| c.get("models"))
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default();
        Codex::new(path, models)
    }

    /// `codex.enabled`: the user's opt-in. Off, Codex is not a provider at all.
    fn codex_enabled(&self) -> bool {
        self.settings().ok().and_then(|s| s.get("codex").and_then(|c| c.get("enabled")).and_then(Value::as_bool)).unwrap_or(false)
    }

    /// `openai.enabled`, `openai.baseUrl`, `openai.models`; the key comes from the keychain.
    fn openai(&self) -> Option<OpenAi> {
        let settings = self.settings().ok()?;
        let o = settings.get("openai")?;
        if !o.get("enabled").and_then(Value::as_bool).unwrap_or(false) {
            return None;
        }
        let url = o.get("baseUrl").and_then(Value::as_str).map(str::trim).filter(|u| !u.is_empty()).unwrap_or(stillroom_models::openai::DEFAULT_URL);
        let models = o.get("models").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default();
        Some(OpenAi::new(url, self.secret(OPENAI_KEY), models))
    }

    /// The providers as configured right now. Cheap to build; a run keeps the one it started with.
    fn router(&self) -> Arc<ModelRouter> {
        let mut providers: Vec<Arc<dyn stillroom_models::Provider>> = vec![Arc::new(self.ollama())];
        if self.codex_enabled() {
            providers.push(Arc::new(self.codex()));
        }
        if let Some(o) = self.openai() {
            providers.push(Arc::new(o));
        }
        Arc::new(ModelRouter::new(providers))
    }

    /// The router as the engine should call it: every answer's speed is remembered on the way back.
    fn models(&self, router: Arc<ModelRouter>) -> Arc<dyn ModelClient> {
        Arc::new(SpeedRecorder { inner: router, store: self.store.clone() })
    }

    /// Seconds one model call may take; `models.timeoutSecs`, default 600, 0 means no limit.
    fn model_timeout(&self) -> Option<std::time::Duration> {
        let secs = self
            .settings()
            .ok()
            .and_then(|s| s.get("models").and_then(|m| m.get("timeoutSecs")).and_then(Value::as_u64))
            .unwrap_or(600);
        (secs > 0).then(|| std::time::Duration::from_secs(secs))
    }

    /// Context length and thinking for steps whose recipe leaves them unset (`models.contextLength`, `models.think`).
    fn model_defaults(&self) -> ModelDefaults {
        self.settings()
            .ok()
            .and_then(|s| s.get("models").cloned())
            .and_then(|m| serde_json::from_value(m).ok())
            .unwrap_or_default()
    }

    /// The user's model defaults from Settings › Models, plus the facts the planner's Auto
    /// uses: what Try planning measured and this Mac's RAM.
    fn resolve_defaults(&self) -> ResolveDefaults {
        let mut d: ResolveDefaults = self
            .settings()
            .ok()
            .and_then(|s| s.get("models").cloned())
            .and_then(|m| serde_json::from_value(m).ok())
            .unwrap_or_default();
        d.planning = self.planning_facts();
        d
    }

    fn planning_facts(&self) -> PlanningFacts {
        let measured = self
            .store
            .lock()
            .ok()
            .and_then(|s| s.planner_measured().ok())
            .unwrap_or_default()
            .into_iter()
            .map(|(id, m)| (id, Measured { ms: m.ms, clean: m.clean }))
            .collect();
        PlanningFacts { measured, memory_bytes: self.memory_bytes }
    }

    /// Where a recipe stands with the user: its literal permission entries against the store's grants.
    fn consent_of(&self, bundle: &RecipeBundle) -> CmdResult<consent::Consent> {
        let granted = self.store.lock().map_err(err)?.grants(&bundle.recipe.id).map_err(err)?;
        Ok(consent::consent_for(&bundle.recipe, bundle.source, &granted))
    }

    /// Refuse, in words, while the recipe still needs the user's yes for anything.
    fn check_consent(&self, bundle: &RecipeBundle) -> CmdResult<()> {
        match consent::refusal(&self.consent_of(bundle)?) {
            Some(text) => Err(text),
            None => Ok(()),
        }
    }

    /// A recipe as the Library lists it: its placement, where a copy came from, and its consent.
    fn info_of(&self, bundle: &RecipeBundle) -> CmdResult<RecipeInfo> {
        let scan = self.library.scan();
        let (placed, granted) = {
            let store = self.store.lock().map_err(err)?;
            (store.shelf_state(&bundle.recipe.id).map_err(err)?, store.grants(&bundle.recipe.id).map_err(err)?)
        };
        Ok(bundle
            .info()
            .with_shelf(placed)
            .with_from(stillroom_core::catalog::provenance(bundle, &scan.bundles))
            .with_consent(consent::consent_for(&bundle.recipe, bundle.source, &granted)))
    }
}

const KEYCHAIN_SERVICE: &str = "com.jeremym.stillroom";
/// The keychain item for a remote Ollama's access key (sent as a bearer token).
const OLLAMA_KEY: &str = "ollama.apiKey";
/// The keychain item for an OpenAI-compatible service's API key.
const OPENAI_KEY: &str = "openai.apiKey";
const KNOWN_SECRETS: &[&str] = &[OLLAMA_KEY, OPENAI_KEY];

fn known_secret(name: &str) -> CmdResult<&'static str> {
    KNOWN_SECRETS.iter().copied().find(|k| *k == name).ok_or_else(|| format!("unknown secret {name}"))
}

/// Store a secret in the OS keychain. Never in the settings store, never in a file.
#[tauri::command]
fn set_secret(app: AppHandle, state: State<'_, AppState>, name: String, value: String) -> CmdResult<()> {
    let name = known_secret(&name)?;
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, name).map_err(err)?;
    if value.trim().is_empty() {
        let _ = entry.delete_credential();
    } else {
        entry.set_password(value.trim()).map_err(err)?;
    }
    if let Ok(mut cache) = state.secrets.lock() {
        cache.remove(name);
    }
    let _ = app.emit("secrets:changed", name);
    Ok(())
}

/// Whether a secret is set. The value itself never goes back to the webview.
#[tauri::command]
fn has_secret(state: State<'_, AppState>, name: String) -> CmdResult<bool> {
    let name = known_secret(&name)?;
    Ok(state.secret(name).is_some())
}




/// Everything a run needs, from the user's Run button, an approval or edit, or a watch.
pub(crate) struct ExecArgs {
    pub recipe_id: String,
    pub inputs: indexmap::IndexMap<String, Value>,
    pub pins: BTreeMap<String, String>,
    pub reuse: bool,
    pub edits: BTreeMap<String, StepEdit>,
    pub resume_from: Option<Run>,
    pub trigger: Option<RunTrigger>,
    /// Stages whose run keep is emptied before the run (Rebuild).
    pub rebuild: Vec<String>,
    /// Called once with the run's id when the run really begins: after its turn at the gate,
    /// on the first step record. A watch uses it to say "started" no earlier than that.
    pub on_started: Option<Box<dyn Fn(&str) + Send + Sync>>,
}

/// `run:started`: the run has its turn and its first step is under way.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RunStarted {
    run_id: String,
    recipe_id: String,
    /// Set when something other than the user (a watch) set the run off.
    trigger: Option<RunTrigger>,
}

/// `run:finished`: the summary, with the run's id repeated as `runId` so every run event names
/// its run the same way.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RunFinished {
    run_id: String,
    #[serde(flatten)]
    summary: RunSummary,
}

/// Resolve models, run the recipe with live step events, save the run, announce it.
pub(crate) async fn execute(app: &AppHandle, state: &AppState, args: ExecArgs) -> CmdResult<Run> {
    let bundle = state
        .library
        .find(&args.recipe_id)
        .ok_or_else(|| format!("recipe {} not found", args.recipe_id))?;
    // Consent first: a recipe that is not built-in runs only once every literal permission entry
    // has the user's yes. Watches come through here too, so they obey as well.
    state.check_consent(&bundle)?;
    let router = state.router();
    let installed = router.list_all().await;
    let resolutions = resolve_slots_with(&bundle.recipe, &installed, &args.pins, &state.resolve_defaults()).map_err(err)?;

    // One run at a time. Nothing is announced as started until this turn is ours: a watch that
    // fires while the user runs something waits here, quietly.
    let _turn = state.run_gate.lock().await;
    let observer = {
        let app = app.clone();
        let recipe_id = args.recipe_id.clone();
        let trigger = args.trigger.clone();
        let on_started = args.on_started;
        let announced = std::sync::atomic::AtomicBool::new(false);
        Arc::new(move |rec: &StepRecord| {
            // The executor mints the run id; the first record is the first time the shell sees it.
            if announced.compare_exchange(false, true, std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst).is_ok() {
                let _ = app.emit("run:started", RunStarted { run_id: rec.run_id.clone(), recipe_id: recipe_id.clone(), trigger: trigger.clone() });
                if let Some(f) = &on_started {
                    f(&rec.run_id);
                }
            }
            let _ = app.emit("run:step", rec.clone());
        })
    };
    let executor = Executor::new(state.tools.clone(), state.models(router)).with_cache(state.store.clone());
    let mut request = bundle.request(args.inputs, resolutions);
    request.artifact_base = Some(state.artifacts_dir.clone());
    request.keep_base = Some(state.keep_base.clone());
    request.rebuild = args.rebuild;
    request.reuse = args.reuse;
    request.edits = args.edits;
    request.resume_from = args.resume_from.map(Box::new);
    request.trigger = args.trigger;
    let cancel = Cancel::new();
    *state.current_cancel.lock().map_err(err)? = Some(cancel.clone());
    request.cancel = Some(cancel);
    request.model_timeout = state.model_timeout();
    request.model_defaults = state.model_defaults();
    let run = executor.run(request, Some(observer)).await;
    *state.current_cancel.lock().map_err(err)? = None;

    state.store.lock().map_err(err)?.save_run(&run).map_err(err)?;
    let _ = app.emit("run:finished", RunFinished { run_id: run.id.clone(), summary: run.summary() });
    Ok(run)
}

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeStatus {
    /// The version of Stillroom that is running, as the bundle says.
    version: String,
    schema_title: String,
    db_path: String,
    tools: Vec<String>,
    run_count: i64,
    recipe_roots: Vec<String>,
}

#[tauri::command]
fn runtime_status(state: State<'_, AppState>) -> CmdResult<RuntimeStatus> {
    let schema: Value = serde_json::from_str(stillroom_core::RECIPE_SCHEMA).map_err(err)?;
    let store = state.store.lock().map_err(err)?;
    let run_count = store.list_runs(10_000).map_err(err)?.len() as i64;
    let mut tools: Vec<String> = state.tools.ids().into_iter().map(String::from).collect();
    tools.sort();
    Ok(RuntimeStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        schema_title: schema["title"].as_str().unwrap_or("?").to_string(),
        db_path: state.db_path.clone(),
        tools,
        run_count,
        recipe_roots: state.library.roots().iter().map(|(p, _)| p.to_string_lossy().into_owned()).collect(),
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelsResponse {
    providers: Vec<ProviderStatus>,
    models: Vec<InstalledModel>,
    /// What each model managed last time on this Mac, by model id; absent when it has not answered yet.
    speeds: BTreeMap<String, ModelSpeed>,
}

#[tauri::command]
async fn list_models(state: State<'_, AppState>) -> CmdResult<ModelsResponse> {
    let router = state.router();
    let speeds = state.store.lock().map(|s| speeds_of(&s)).unwrap_or_default();
    Ok(ModelsResponse { providers: router.statuses().await, models: router.list_all().await, speeds })
}

/// This Mac's hardware, read once and kept: the chip, the memory the CPU and GPU share, the
/// cores, and the room a model has before it is swapped through memory. Never fails; what could
/// not be read is null.
#[tauri::command]
async fn hardware() -> CmdResult<Hardware> {
    static HARDWARE: std::sync::OnceLock<Hardware> = std::sync::OnceLock::new();
    if let Some(h) = HARDWARE.get() {
        return Ok(h.clone());
    }
    // system_profiler can take seconds; keep it off the UI thread.
    let h = tokio::task::spawn_blocking(stillroom_core::model::detect_hardware).await.unwrap_or_default();
    Ok(HARDWARE.get_or_init(|| h).clone())
}

/// What Settings shows about the Codex CLI, independent of the opt-in.
#[tauri::command]
async fn codex_status(state: State<'_, AppState>) -> CmdResult<stillroom_models::CodexStatus> {
    Ok(state.codex().status().await)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TryResult {
    ok: bool,
    duration_ms: u64,
    answer: Option<String>,
    error: Option<String>,
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    /// How fast the model wrote, when the provider said (Ollama does).
    completion_tokens_per_second: Option<f64>,
}

/// One tiny call to a model, so Settings can show it answers and how fast. Two minutes at most.
#[tauri::command]
async fn try_model(state: State<'_, AppState>, model_id: String) -> CmdResult<TryResult> {
    use stillroom_core::model::{ChatMessage, ChatRequest};
    let router = state.models(state.router());
    let req = ChatRequest {
        messages: vec![ChatMessage::new("user", "Reply with the single word pong and nothing else.")],
        think: Some(Value::Bool(false)),
        ..Default::default()
    };
    let started = std::time::Instant::now();
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(120), router.chat(&model_id, req)).await;
    let duration_ms = started.elapsed().as_millis() as u64;
    Ok(match outcome {
        Ok(Ok(r)) => TryResult {
            ok: true,
            duration_ms,
            answer: Some(r.content.trim().chars().take(120).collect()),
            error: None,
            prompt_tokens: r.prompt_tokens,
            completion_tokens: r.completion_tokens,
            completion_tokens_per_second: r.completion_tokens_per_second,
        },
        Ok(Err(e)) => TryResult { ok: false, duration_ms, answer: None, error: Some(e.to_string()), prompt_tokens: None, completion_tokens: None, completion_tokens_per_second: None },
        Err(_) => TryResult { ok: false, duration_ms, answer: None, error: Some("no answer within 120 s".into()), prompt_tokens: None, completion_tokens: None, completion_tokens_per_second: None },
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviewArgs {
    recipe_id: String,
    #[serde(default)]
    pins: BTreeMap<String, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResolutionPreview {
    /// Slot → what a run started now would use.
    resolutions: BTreeMap<String, stillroom_core::Resolution>,
    /// Slot → why it cannot be filled.
    problems: BTreeMap<String, String>,
}

/// What each slot would get if the recipe ran right now, with these pins, the user's defaults,
/// and the models installed at this moment. Nothing runs.
#[tauri::command]
async fn preview_resolution(state: State<'_, AppState>, args: PreviewArgs) -> CmdResult<ResolutionPreview> {
    let bundle = state.library.find(&args.recipe_id).ok_or_else(|| format!("recipe {} not found", args.recipe_id))?;
    let installed = state.router().list_all().await;
    let defaults = state.resolve_defaults();
    let mut out = ResolutionPreview { resolutions: BTreeMap::new(), problems: BTreeMap::new() };
    for (name, slot) in &bundle.recipe.model_slots {
        let mut one = bundle.recipe.clone();
        one.model_slots = [(name.clone(), slot.clone())].into_iter().collect();
        match resolve_slots_with(&one, &installed, &args.pins, &defaults) {
            Ok(mut r) => {
                if let Some(res) = r.remove(name) {
                    out.resolutions.insert(name.clone(), res);
                }
            }
            Err(e) => {
                out.problems.insert(name.clone(), e.to_string());
            }
        }
    }
    Ok(out)
}

/// Stop what is in flight: `which` is `"run"` (a recipe running), `"plan"` (the planner drafting a
/// recipe), or absent, which stops both. Returns whether anything was running.
#[tauri::command]
fn stop_run(state: State<'_, AppState>, which: Option<String>) -> CmdResult<bool> {
    let slots: &[&Mutex<Option<Cancel>>] = match which.as_deref() {
        Some("run") => &[&state.current_cancel],
        Some("plan") => &[&state.plan_cancel],
        None => &[&state.current_cancel, &state.plan_cancel],
        Some(other) => return Err(format!("nothing called {other} can be stopped; say run or plan")),
    };
    let mut stopped = false;
    for slot in slots {
        if let Some(c) = slot.lock().map_err(err)?.as_ref() {
            c.cancel();
            stopped = true;
        }
    }
    Ok(stopped)
}

/// Every setting as one JSON object. Keys: look, ollama, models, pins, firstRun.
#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> CmdResult<Value> {
    Ok(Value::Object(state.settings()?))
}

/// The top-level settings Stillroom knows. Anything else in a patch is refused.
const SETTING_KEYS: &[&str] = &["look", "ollama", "models", "pins", "firstRun", "codex", "openai", "planner", updates::SETTING];

/// A service address as a setting: a web address (http or https) with a host, kept without any
/// user name or password that was typed into it, and without a trailing slash. Empty means unset.
fn checked_service_url(what: &str, raw: &str) -> CmdResult<Option<String>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let mut url = tauri::Url::parse(raw).map_err(|e| format!("{what}: {raw:?} is not a web address ({e})"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("{what}: only http and https addresses work; {raw:?} is {}", url.scheme()));
    }
    if url.host_str().is_none() {
        return Err(format!("{what}: {raw:?} names no host"));
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_fragment(None);
    Ok(Some(url.as_str().trim_end_matches('/').to_string()))
}

/// A program path as a setting: an existing file this user may run. Empty means "find it".
fn checked_program(what: &str, raw: &str) -> CmdResult<Option<String>> {
    use std::os::unix::fs::PermissionsExt;
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let meta = std::fs::metadata(raw).map_err(|e| format!("{what}: {raw} cannot be used ({e})"))?;
    if !meta.is_file() || meta.permissions().mode() & 0o111 == 0 {
        return Err(format!("{what}: {raw} is not a program Stillroom can run"));
    }
    Ok(Some(raw.to_string()))
}

/// One setting as the webview sent it, checked and tidied, or refused in words. `Null` removes.
fn checked_setting(key: &str, value: &Value) -> CmdResult<Value> {
    if !SETTING_KEYS.contains(&key) {
        return Err(format!("{key} is not a setting Stillroom knows"));
    }
    if value.is_null() {
        return Ok(Value::Null);
    }
    let want_object = |v: &Value| v.as_object().cloned().ok_or_else(|| format!("{key} must be an object"));
    let tidy = |mut o: serde_json::Map<String, Value>, field: &str, check: &dyn Fn(&str) -> CmdResult<Option<String>>| -> CmdResult<Value> {
        if let Some(v) = o.get(field) {
            match v {
                Value::Null => {
                    o.remove(field);
                }
                Value::String(s) => match check(s)? {
                    Some(fixed) => {
                        o.insert(field.into(), Value::String(fixed));
                    }
                    None => {
                        o.remove(field);
                    }
                },
                _ => return Err(format!("{key}.{field} must be text")),
            }
        }
        Ok(Value::Object(o))
    };
    match key {
        "firstRun" => value.as_bool().map(Value::Bool).ok_or_else(|| "firstRun must be true or false".to_string()),
        "updates" => {
            let o = want_object(value)?;
            if let Some(d) = o.get("daily") {
                if !d.is_boolean() {
                    return Err("updates.daily must be true or false".to_string());
                }
            }
            Ok(Value::Object(o))
        }
        "codex" => tidy(want_object(value)?, "path", &|s| checked_program("codex.path", s)),
        "ollama" => tidy(want_object(value)?, "baseUrl", &|s| checked_service_url("ollama.baseUrl", s)),
        "openai" => tidy(want_object(value)?, "baseUrl", &|s| checked_service_url("openai.baseUrl", s)),
        _ => want_object(value).map(Value::Object),
    }
}

/// Merge a patch of top-level settings (a key set to null is removed), then tell every window.
/// Every key is one Stillroom knows, `codex.path` is a program that exists, and the service
/// addresses are web addresses with any user name or password dropped; otherwise nothing is
/// written and the reason comes back in words.
#[tauri::command]
fn set_settings(app: AppHandle, state: State<'_, AppState>, patch: Value) -> CmdResult<Value> {
    let patch = patch.as_object().ok_or("settings patch must be an object")?;
    let mut checked = Vec::with_capacity(patch.len());
    for (k, v) in patch {
        checked.push((k.clone(), checked_setting(k, v)?));
    }
    {
        let store = state.store.lock().map_err(err)?;
        for (k, v) in &checked {
            // What the engine measured (Try planning under `planner.measured`, each model's speed
            // under `models.speed`) is written by the engine, not the screen: a patch that leaves
            // it out keeps it.
            let engine_field = match k.as_str() {
                stillroom_core::store::PLANNER_SETTING => Some(stillroom_core::store::PLANNER_MEASURED_FIELD),
                MODELS_SETTING => Some(SPEED_FIELD),
                updates::SETTING => Some(updates::LAST_FIELD),
                _ => None,
            };
            let v = match (engine_field, v) {
                (Some(field), Value::Object(o)) if !o.contains_key(field) => {
                    let mut o = o.clone();
                    if let Some(kept) = store.get_setting(k).map_err(err)?.and_then(|p| p.get(field).cloned()) {
                        o.insert(field.into(), kept);
                    }
                    Value::Object(o)
                }
                _ => v.clone(),
            };
            store.set_setting(k, &v).map_err(err)?;
        }
    }
    let all = Value::Object(state.settings()?);
    let _ = app.emit("settings:changed", all.clone());
    Ok(all)
}

#[cfg(test)]
mod planner_preview_tests {
    use super::*;
    use serde_json::json;

    /// The contract the New recipe screen and Settings › Models build on.
    #[test]
    fn planner_preview_and_try_planning_serialize_in_camel_case() {
        let preview = PlannerPreview {
            resolution: Some(PlannerResolution {
                slot: "planner".into(),
                model_id: "ollama:glm-4.7-flash".into(),
                via: ResolvedVia::Capability,
                reason: "fits in memory, answers in a fixed shape (JSON), planned clean in 48 s".into(),
            }),
            problem: None,
            memory_bytes: Some(36 << 30),
            candidates: vec![
                PlannerCandidate {
                    model_id: "ollama:glm-4.7-flash".into(),
                    name: "glm-4.7-flash".into(),
                    parameter_size: Some("29.9B".into()),
                    size_bytes: Some(20 << 30),
                    fits: true,
                    rank: 1,
                    measured_ms: Some(48_000),
                    measured_clean: Some(true),
                    reason: "fits in memory, answers in a fixed shape (JSON), planned clean in 48 s".into(),
                    local: true,
                },
                PlannerCandidate {
                    model_id: "ollama:qwen3-coder:80b".into(),
                    name: "qwen3-coder:80b".into(),
                    parameter_size: Some("80B".into()),
                    size_bytes: Some(49 << 30),
                    fits: false,
                    rank: 2,
                    measured_ms: None,
                    measured_clean: None,
                    reason: "larger than this Mac can hold comfortably, answers in a fixed shape (JSON), not yet measured".into(),
                    local: true,
                },
            ],
        };
        assert_eq!(
            serde_json::to_value(&preview).unwrap(),
            json!({
                "resolution": { "slot": "planner", "modelId": "ollama:glm-4.7-flash", "via": "capability", "reason": "fits in memory, answers in a fixed shape (JSON), planned clean in 48 s" },
                "problem": null,
                "memoryBytes": 38654705664u64,
                "candidates": [
                    { "modelId": "ollama:glm-4.7-flash", "name": "glm-4.7-flash", "parameterSize": "29.9B", "sizeBytes": 21474836480u64, "fits": true, "rank": 1, "measuredMs": 48000, "measuredClean": true,
                      "reason": "fits in memory, answers in a fixed shape (JSON), planned clean in 48 s", "local": true },
                    { "modelId": "ollama:qwen3-coder:80b", "name": "qwen3-coder:80b", "parameterSize": "80B", "sizeBytes": 52613349376u64, "fits": false, "rank": 2, "measuredMs": null, "measuredClean": null,
                      "reason": "larger than this Mac can hold comfortably, answers in a fixed shape (JSON), not yet measured", "local": true }
                ]
            })
        );
        let trial = TryPlanning { ms: 48_000, clean: false, problems: vec!["the answer named no stages".into()] };
        assert_eq!(serde_json::to_value(&trial).unwrap(), json!({ "ms": 48000, "clean": false, "problems": ["the answer named no stages"] }));
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unknown_keys_and_wrong_shapes_are_refused() {
        assert_eq!(checked_setting("secrets", &json!({})).unwrap_err(), "secrets is not a setting Stillroom knows");
        assert_eq!(checked_setting("firstRun", &json!("yes")).unwrap_err(), "firstRun must be true or false");
        assert_eq!(checked_setting("updates", &json!({ "daily": "yes" })).unwrap_err(), "updates.daily must be true or false");
        assert_eq!(checked_setting("updates", &json!({ "daily": true })).unwrap(), json!({ "daily": true }));
        assert_eq!(checked_setting("look", &json!("green")).unwrap_err(), "look must be an object");
        assert_eq!(checked_setting("look", &json!({ "theme": "green" })).unwrap(), json!({ "theme": "green" }));
        assert_eq!(checked_setting("codex", &Value::Null).unwrap(), Value::Null, "null removes");
        assert_eq!(checked_setting("codex", &json!({ "path": 3 })).unwrap_err(), "codex.path must be text");
    }

    #[test]
    fn codex_path_must_be_a_program_that_exists() {
        assert_eq!(checked_setting("codex", &json!({ "enabled": true, "path": "/bin/ls" })).unwrap(), json!({ "enabled": true, "path": "/bin/ls" }));
        assert_eq!(checked_setting("codex", &json!({ "path": "  " })).unwrap(), json!({}), "empty means find it");
        assert_eq!(checked_setting("codex", &json!({ "path": null, "models": [] })).unwrap(), json!({ "models": [] }));
        let err = checked_setting("codex", &json!({ "path": "/etc/hosts" })).unwrap_err();
        assert_eq!(err, "codex.path: /etc/hosts is not a program Stillroom can run");
        let err = checked_setting("codex", &json!({ "path": "/nowhere/codex" })).unwrap_err();
        assert!(err.starts_with("codex.path: /nowhere/codex cannot be used"), "{err}");
        let err = checked_setting("codex", &json!({ "path": "/bin" })).unwrap_err();
        assert!(err.ends_with("is not a program Stillroom can run"), "{err}");
    }

    /// The engine's speed table lives under `models.speed` beside the user's model defaults and
    /// survives them being rewritten.
    #[test]
    fn a_models_speed_is_recorded_and_read_back_without_touching_the_rest() {
        let store = Store::open_in_memory().unwrap();
        store.set_setting(MODELS_SETTING, &json!({ "preference": "speed", "timeoutSecs": 30 })).unwrap();
        record_speed(&store, "ollama:qwen3:8b", 66.44).unwrap();
        record_speed(&store, "ollama:gemma3:12b", 41.0).unwrap();
        let models = store.get_setting(MODELS_SETTING).unwrap().unwrap();
        assert_eq!(models["preference"], "speed", "the user's defaults stay");
        assert_eq!(models["timeoutSecs"], 30);
        assert_eq!(models["speed"]["ollama:qwen3:8b"]["tokensPerSecond"], 66.4, "one decimal");
        let speeds = speeds_of(&store);
        assert_eq!(speeds.len(), 2);
        assert_eq!(speeds["ollama:gemma3:12b"].tokens_per_second, 41.0);
        assert!(speeds["ollama:gemma3:12b"].at.starts_with("20"));
        // A later answer replaces the earlier one for that model.
        record_speed(&store, "ollama:qwen3:8b", 70.0).unwrap();
        assert_eq!(speeds_of(&store)["ollama:qwen3:8b"].tokens_per_second, 70.0);
        assert!(speeds_of(&Store::open_in_memory().unwrap()).is_empty(), "nothing recorded yet: an empty table");
    }

    #[test]
    fn service_addresses_are_web_addresses_without_credentials() {
        assert_eq!(checked_setting("ollama", &json!({ "baseUrl": "http://localhost:11434" })).unwrap(), json!({ "baseUrl": "http://localhost:11434" }));
        assert_eq!(checked_setting("ollama", &json!({ "baseUrl": "http://user:secret@ollama.local:11434/" })).unwrap(), json!({ "baseUrl": "http://ollama.local:11434" }), "a typed-in password never reaches the store");
        assert_eq!(checked_setting("openai", &json!({ "enabled": true, "baseUrl": "https://api.example.com/v1/#x" })).unwrap(), json!({ "enabled": true, "baseUrl": "https://api.example.com/v1" }));
        assert_eq!(checked_setting("ollama", &json!({ "baseUrl": "" })).unwrap(), json!({}), "empty means the default");
        for bad in ["ftp://x", "localhost:11434", "file:///etc", "http://", "not a url"] {
            let err = checked_setting("ollama", &json!({ "baseUrl": bad })).unwrap_err();
            assert!(err.starts_with("ollama.baseUrl:"), "{bad}: {err}");
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PullEvent {
    name: String,
    status: String,
    total: Option<u64>,
    completed: Option<u64>,
    done: bool,
    error: Option<String>,
}

/// Download a model through Ollama. Progress arrives as `model:pull` events; the command returns when it is done.
#[tauri::command]
async fn pull_model(app: AppHandle, state: State<'_, AppState>, name: String) -> CmdResult<()> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("name a model to pull, like qwen3:8b".into());
    }
    let ollama = state.ollama();
    let emitter = app.clone();
    let n = name.clone();
    let result = ollama
        .pull(&name, move |p| {
            let _ = emitter.emit("model:pull", PullEvent { name: n.clone(), status: p.status.clone(), total: p.total, completed: p.completed, done: p.status == "success", error: None });
        })
        .await;
    match result {
        Ok(()) => {
            let _ = app.emit("model:pull", PullEvent { name, status: "success".into(), total: None, completed: None, done: true, error: None });
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            let _ = app.emit("model:pull", PullEvent { name, status: "error".into(), total: None, completed: None, done: true, error: Some(msg.clone()) });
            Err(msg)
        }
    }
}

/// Whether a window may navigate to `url`: only the app's own pages. In production that is the
/// `tauri://localhost` origin (`http://tauri.localhost` on Windows); in development, the Vite dev
/// server as well. Everything else is refused and the page stays where it is; links in results open
/// outside through `open_link` instead.
fn is_app_page(app: &AppHandle, url: &tauri::Url) -> bool {
    match url.scheme() {
        "tauri" => true,
        "http" | "https" => {
            if url.host_str() == Some("tauri.localhost") {
                return true;
            }
            if cfg!(dev) {
                if let Some(dev) = &app.config().build.dev_url {
                    return url.scheme() == dev.scheme() && url.host_str() == dev.host_str() && url.port_or_known_default() == dev.port_or_known_default();
                }
            }
            false
        }
        _ => false,
    }
}

/// Open a link from a result in the browser (or the mail app). Only web and mail addresses: anything
/// else is refused, so a result cannot launch an arbitrary scheme handler.
#[tauri::command]
fn open_link(app: AppHandle, url: String) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    let parsed = tauri::Url::parse(url.trim()).map_err(|e| format!("not a link: {e}"))?;
    match parsed.scheme() {
        "http" | "https" | "mailto" => {}
        other => return Err(format!("{other}: links are only opened for web (http, https) and mail (mailto) addresses")),
    }
    app.opener().open_url(parsed.as_str(), None::<&str>).map_err(err)
}

/// Open the Settings window, or bring it forward if it is already open.
#[tauri::command]
fn open_settings(app: AppHandle) -> CmdResult<()> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    let nav = app.clone();
    WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("index.html?window=settings".into()))
        .title("Stillroom Settings")
        .inner_size(960.0, 760.0)
        .min_inner_size(760.0, 560.0)
        .on_navigation(move |url| is_app_page(&nav, url))
        .build()
        .map_err(err)?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecipesResponse {
    recipes: Vec<RecipeInfo>,
    errors: Vec<RecipeLoadError>,
    /// Ids with runs in the history but no recipe folder any more. A new recipe never takes one.
    reserved: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecipeLoadError {
    path: String,
    error: String,
}

#[tauri::command]
fn list_recipes(state: State<'_, AppState>) -> CmdResult<RecipesResponse> {
    let scan = state.library.scan();
    let (placed, grants, ran) = {
        let store = state.store.lock().map_err(err)?;
        (store.shelf_all().map_err(err)?, store.grants_all().map_err(err)?, store.run_recipe_ids().map_err(err)?)
    };
    let none = Vec::new();
    let reserved = ran.into_iter().filter(|id| !scan.bundles.iter().any(|b| &b.recipe.id == id)).collect();
    Ok(RecipesResponse {
        reserved,
        recipes: scan
            .bundles
            .iter()
            .map(|b| {
                b.info()
                    .with_shelf(placed.get(&b.recipe.id).copied())
                    .with_from(stillroom_core::catalog::provenance(b, &scan.bundles))
                    .with_consent(consent::consent_for(&b.recipe, b.source, grants.get(&b.recipe.id).unwrap_or(&none)))
            })
            .collect(),
        errors: scan
            .errors
            .into_iter()
            .map(|(p, e)| RecipeLoadError { path: p.to_string_lossy().into_owned(), error: e })
            .collect(),
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunArgs {
    recipe_id: String,
    inputs: indexmap::IndexMap<String, Value>,
    #[serde(default)]
    pins: BTreeMap<String, String>,
    /// Reuse outputs of unchanged steps from earlier runs (selective rerun). Default true.
    #[serde(default = "default_true")]
    reuse: bool,
    /// Stages whose run keep is emptied first (Rebuild).
    #[serde(default)]
    rebuild: Vec<String>,
}

fn default_true() -> bool {
    true
}

#[tauri::command]
async fn run_recipe(app: AppHandle, state: State<'_, AppState>, args: RunArgs) -> CmdResult<Run> {
    execute(
        &app,
        &state,
        ExecArgs {
            recipe_id: args.recipe_id,
            inputs: args.inputs,
            pins: args.pins,
            reuse: args.reuse,
            edits: BTreeMap::new(),
            resume_from: None,
            trigger: None,
            rebuild: args.rebuild,
            on_started: None,
        },
    )
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditArgs {
    /// The run whose artifact was edited.
    run_id: String,
    /// The step that produced the edited artifact.
    step_id: String,
    /// The edited output, which replaces that step's output.
    output: Value,
    #[serde(default = "default_true")]
    reuse: bool,
}

/// Change propagation: rerun a finished run with one step's output replaced by the user's
/// edit. Same inputs, same models. Steps downstream of the edit are regenerated; everything
/// else is reused. Earlier edits on that run are carried forward while their inputs still hold.
#[tauri::command]
async fn rerun_with_edit(app: AppHandle, state: State<'_, AppState>, args: EditArgs) -> CmdResult<Run> {
    let (prior, base) = {
        let store = state.store.lock().map_err(err)?;
        let prior = store.get_run(&args.run_id).map_err(err)?.ok_or_else(|| format!("run {} not found", args.run_id))?;
        // Resume from the whole chain of runs this one continues, so a step a pause held back
        // is still found done.
        let base = store.continuation_base(&args.run_id).map_err(err)?.unwrap_or_else(|| prior.clone());
        (prior, base)
    };
    let bundle = state
        .library
        .find(&prior.recipe_id)
        .ok_or_else(|| format!("recipe {} not found", prior.recipe_id))?;
    validate_edit(&bundle.recipe, &args.step_id, &args.output).map_err(err)?;
    let edits = edits_for_rerun(&prior, &args.step_id, args.output).map_err(err)?;
    let run = execute(
        &app,
        &state,
        ExecArgs {
            rebuild: Vec::new(),
            recipe_id: prior.recipe_id.clone(),
            inputs: prior.inputs.clone(),
            pins: pins_from_run(&prior),
            reuse: args.reuse,
            edits,
            resume_from: Some(base),
            trigger: None,
            on_started: None,
        },
    )
    .await?;
    // A watch that would have continued from the old run continues from this one.
    if let Ok(store) = state.store.lock() {
        if store.relink_watches(&prior.id, &run.id).map_err(err)? > 0 {
            let _ = app.emit("watch:changed", ());
        }
    }
    Ok(run)
}

#[tauri::command]
fn list_runs(state: State<'_, AppState>, limit: Option<usize>) -> CmdResult<Vec<RunSummary>> {
    state.store.lock().map_err(err)?.list_runs(limit.unwrap_or(50)).map_err(err)
}

#[tauri::command]
fn get_run(state: State<'_, AppState>, id: String) -> CmdResult<Option<Run>> {
    state.store.lock().map_err(err)?.get_run(&id).map_err(err)
}

#[tauri::command]
fn usage(state: State<'_, AppState>) -> CmdResult<Vec<ModelUsage>> {
    state.store.lock().map_err(err)?.usage_by_model().map_err(err)
}

/// What a run with these inputs would touch, one row per permission entry, with every chosen
/// folder tried for reading or writing (a probe file, created and removed). Set up shows the rows
/// and will not run while a row says cannot.
#[tauri::command]
fn probe_permissions(state: State<'_, AppState>, recipe_id: String, inputs: indexmap::IndexMap<String, Value>) -> CmdResult<Vec<stillroom_core::probe::PermissionRow>> {
    let bundle = state.library.find(&recipe_id).ok_or_else(|| format!("recipe {recipe_id} not found"))?;
    Ok(stillroom_core::probe::probe_permissions(&bundle.recipe, &inputs))
}

#[tauri::command]
fn estimate(state: State<'_, AppState>, recipe_id: String) -> CmdResult<Option<RunEstimate>> {
    let stages = state.library.find(&recipe_id).map(|b| b.info().stages).unwrap_or_default();
    state.store.lock().map_err(err)?.estimate_for_recipe(&recipe_id, &stages).map_err(err)
}

/// Write an artifact to a path the user chose in a save dialog.
#[tauri::command]
fn list_tools(state: State<'_, AppState>) -> CmdResult<Vec<ToolInfo>> {
    Ok(state.tools.catalog())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DraftResponse {
    id: String,
    source: RecipeSource,
    path: String,
    draft: Draft,
}

/// Every system prompt with its revisions.
#[tauri::command]
fn list_system_prompts(state: State<'_, AppState>) -> CmdResult<Vec<PromptState>> {
    Ok(state.prompts.states())
}

#[tauri::command]
fn read_system_prompt(state: State<'_, AppState>, name: String) -> CmdResult<PromptState> {
    state.prompts.state(&name).map_err(err)
}

/// Save the text as a new revision and use it from now on.
#[tauri::command]
fn save_system_prompt(app: AppHandle, state: State<'_, AppState>, name: String, text: String) -> CmdResult<PromptState> {
    let s = state.prompts.save(&name, &text).map_err(err)?;
    let _ = app.emit("prompts:changed", &name);
    Ok(s)
}

/// Use revision `n`; 0 is the original (reset). Nothing is deleted.
#[tauri::command]
fn use_system_prompt_version(app: AppHandle, state: State<'_, AppState>, name: String, n: u32) -> CmdResult<PromptState> {
    let s = state.prompts.use_version(&name, n).map_err(err)?;
    let _ = app.emit("prompts:changed", &name);
    Ok(s)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlannerPreview {
    /// The model a plan started now would use, how it was chosen, and why in words.
    resolution: Option<PlannerResolution>,
    problem: Option<String>,
    /// Physical RAM of this Mac; null when it could not be read (every model then counts as fitting).
    memory_bytes: Option<u64>,
    /// Every model that could plan, best first: the local ones in Auto's order, then any cloud
    /// model you have turned on (never an automatic pick), for the picker.
    candidates: Vec<PlannerCandidate>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlannerResolution {
    slot: String,
    model_id: String,
    via: ResolvedVia,
    /// Why, from the facts: "fits in memory, answers in a fixed shape (JSON), planned clean in 48 s".
    reason: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlannerCandidate {
    model_id: String,
    /// The bare name and the provider's size word ("29.9B"), for the picker's label.
    name: String,
    parameter_size: Option<String>,
    /// The weights on disk; null when the provider does not say (cloud models).
    size_bytes: Option<u64>,
    /// Whether the weights fit comfortably in this Mac's memory (at most 60 % of RAM).
    fits: bool,
    /// Auto's order, 1 first.
    rank: u32,
    /// What Try planning measured, if it ran: how long, and whether the answer checked clean.
    measured_ms: Option<u64>,
    measured_clean: Option<bool>,
    reason: String,
    /// True when the model runs on this Mac. A cloud model is listed only because you turned it on.
    local: bool,
}

impl AppState {
    /// The planner's pin from Settings (`planner.model`), unless the caller names one.
    fn planner_pins(&self, model: Option<String>) -> BTreeMap<String, String> {
        let pin = model.or_else(|| {
            self.settings().ok().and_then(|s| s.get("planner").and_then(|p| p.get("model")).and_then(Value::as_str).map(String::from))
        });
        pin.filter(|m| !m.trim().is_empty()).map(|m| [(PLANNER_SLOT.to_string(), m)].into_iter().collect()).unwrap_or_default()
    }
}

/// Which model would plan right now: the pin you pass, else Settings, else Auto (local only, by
/// the planning rank), with the reason in words and the cost of each candidate said up front.
#[tauri::command]
async fn preview_planner(state: State<'_, AppState>, model: Option<String>) -> CmdResult<PlannerPreview> {
    let installed = state.router().list_all().await;
    let defaults = state.resolve_defaults();
    let facts = &defaults.planning;
    let slot = stillroom_core::planner::planner_slot();
    let candidate = |m: &InstalledModel, rank: u32, reason: String| PlannerCandidate {
        model_id: m.id.clone(),
        name: m.name.clone(),
        parameter_size: m.parameter_size.clone(),
        size_bytes: m.size_bytes,
        fits: stillroom_core::model::fits_in_memory(m.size_bytes, facts.memory_bytes),
        rank,
        measured_ms: facts.measured.get(&m.id).map(|x| x.ms),
        measured_clean: facts.measured.get(&m.id).map(|x| x.clean),
        reason,
        local: m.local,
    };
    let mut candidates: Vec<PlannerCandidate> =
        rank_for_planning(&installed, &facts.measured, facts.memory_bytes).into_iter().map(|(m, rank, reason)| candidate(&m, rank, reason)).collect();
    let mut next = candidates.len() as u32 + 1;
    for m in installed.iter().filter(|m| !m.local && slot.requires.iter().all(|c| m.capabilities.contains(c))) {
        candidates.push(candidate(m, next, planning_reason(m, facts.memory_bytes, facts.measured.get(&m.id))));
        next += 1;
    }
    let pins = state.planner_pins(model);
    Ok(match resolve_slots_with(&planner_recipe(), &installed, &pins, &defaults) {
        Ok(mut r) => {
            let resolution = r.remove(PLANNER_SLOT).map(|res| {
                let facts_words = installed
                    .iter()
                    .find(|m| m.id == res.model_id)
                    .map(|m| planning_reason(m, facts.memory_bytes, facts.measured.get(&m.id)))
                    .unwrap_or_default();
                let reason = match res.via {
                    ResolvedVia::Pinned => format!("chosen by you; {facts_words}"),
                    ResolvedVia::Default => format!("your default model; {facts_words}"),
                    ResolvedVia::Recommended => format!("recommended; {facts_words}"),
                    ResolvedVia::Capability => facts_words,
                };
                PlannerResolution { slot: res.slot, model_id: res.model_id, via: res.via, reason }
            });
            PlannerPreview { resolution, problem: None, memory_bytes: facts.memory_bytes, candidates }
        }
        Err(e) => PlannerPreview { resolution: None, problem: Some(e.to_string()), memory_bytes: facts.memory_bytes, candidates },
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TryPlanning {
    /// How long the strip call took.
    ms: u64,
    /// Whether the answer checked clean.
    clean: bool,
    /// Why not, in the checker's words. Empty when clean.
    problems: Vec<String>,
}

/// How long Try planning waits for the strip call before giving up.
const TRY_PLANNING_CAP: std::time::Duration = std::time::Duration::from_secs(300);

/// Try planning: the planner's first call (choosing the stages) on a fixed sample goal, with
/// `model_id`, timed and checked. The result is recorded under `planner.measured.<model id>`,
/// so Auto can prefer a model that planned clean and quickly. Stop ("plan") cancels it; five
/// minutes at most, and a model that takes longer is recorded as not clean.
#[tauri::command]
async fn try_planning(app: AppHandle, state: State<'_, AppState>, model_id: String) -> CmdResult<TryPlanning> {
    let router = state.router();
    let installed = router.list_all().await;
    let pins: BTreeMap<String, String> = [(PLANNER_SLOT.to_string(), model_id)].into_iter().collect();
    let resolutions = resolve_slots_with(&planner_recipe(), &installed, &pins, &state.resolve_defaults()).map_err(err)?;
    let model_id = resolutions.get(PLANNER_SLOT).map(|r| r.model_id.clone()).ok_or("no model for the planner")?;
    let cancel = Cancel::new();
    *state.plan_cancel.lock().map_err(err)? = Some(cancel.clone());
    let planner = Planner {
        library: Some(state.stage_library.clone()),
        models: state.models(router),
        model_id: model_id.clone(),
        tools: state.tools.catalog(),
        prompt: state.prompts.current_text("planner").map_err(err)?,
        strip_prompt: state.prompts.current_text("strip").map_err(err)?,
        timeout: Some(TRY_PLANNING_CAP),
        cancel: Some(cancel),
        model_defaults: state.model_defaults(),
        max_attempts: 1,
    };
    let out = planner.try_strip(&trial_request(), None).await;
    *state.plan_cancel.lock().map_err(err)? = None;
    let trial = match out {
        Ok(t) => TryPlanning { ms: t.ms, clean: t.clean, problems: t.problems },
        Err(stillroom_core::PlanError::Timeout(secs)) => TryPlanning { ms: secs * 1000, clean: false, problems: vec![format!("no answer within {secs} s")] },
        Err(e) => return Err(e.to_string()),
    };
    {
        let store = state.store.lock().map_err(err)?;
        store
            .record_planner_measurement(&model_id, &PlannerMeasurement { ms: trial.ms, clean: trial.clean, at: chrono::Utc::now().to_rfc3339() })
            .map_err(err)?;
    }
    let _ = app.emit("settings:changed", Value::Object(state.settings()?));
    Ok(trial)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlanArgs {
    #[serde(flatten)]
    request: PlanRequest,
    /// A model to plan with this time; otherwise Settings, then Auto.
    #[serde(default)]
    model: Option<String>,
}

/// Draft a recipe from a goal. Progress arrives as `plan:progress` events, including what the
/// model is writing as it writes it (Phase, Delta, Done); Stop cancels it mid-stream.
/// Nothing is written: the draft comes back for the user to save.
#[tauri::command]
async fn plan_recipe(app: AppHandle, state: State<'_, AppState>, args: PlanArgs) -> CmdResult<PlanOutcome> {
    let router = state.router();
    let installed = router.list_all().await;
    let pins = state.planner_pins(args.model);
    let resolutions = resolve_slots_with(&planner_recipe(), &installed, &pins, &state.resolve_defaults()).map_err(err)?;
    let model_id = resolutions.get(PLANNER_SLOT).map(|r| r.model_id.clone()).ok_or("no model for the planner")?;
    let cancel = Cancel::new();
    *state.plan_cancel.lock().map_err(err)? = Some(cancel.clone());
    let planner = Planner {
        library: Some(state.stage_library.clone()),
        models: state.models(router),
        model_id: model_id.clone(),
        tools: state.tools.catalog(),
        prompt: state.prompts.current_text("planner").map_err(err)?,
        strip_prompt: state.prompts.current_text("strip").map_err(err)?,
        timeout: state.model_timeout(),
        cancel: Some(cancel),
        model_defaults: state.model_defaults(),
        max_attempts: 3,
    };
    // The journal watches the same events the window does, so a plan that never returns (stopped,
    // out of time) still leaves its transcript and the checker's problems behind.
    let log = Arc::new(Mutex::new(plan_journal::EventLog::default()));
    let observer = {
        let app = app.clone();
        let log = log.clone();
        Arc::new(move |e: PlanEvent| {
            if let Ok(mut l) = log.lock() {
                l.see(&e);
            }
            let _ = app.emit("plan:progress", e);
        })
    };
    let started_at = chrono::Local::now();
    let out = planner.plan(&args.request, Some(observer)).await;
    *state.plan_cancel.lock().map_err(err)? = None;
    let revisions: Vec<(String, u32)> = ["strip", "planner"].iter().filter_map(|n| state.prompts.state(n).ok().map(|s| (n.to_string(), s.current))).collect();
    let log = log.lock().map(|mut l| std::mem::take(&mut *l)).unwrap_or_default();
    let record = match &out {
        Ok(o) => plan_journal::PlanRecord::from_outcome(&args.request, o, revisions, log.fix_rounds, started_at),
        Err(e) => plan_journal::PlanRecord {
            request: args.request.clone(),
            model_id,
            prompt_revisions: revisions,
            attempts: log.attempts,
            duration_ms: log.last_elapsed_ms,
            prompt_tokens: 0,
            completion_tokens: log.tokens,
            fix_rounds: log.fix_rounds,
            result: plan_journal::PlanResult::Failed(e.to_string()),
            transcript: log.transcript,
            started_at,
        },
    };
    if let Err(e) = plan_journal::write(&state.plans_dir, &record) {
        tracing::warn!("could not write the planner's journal: {e}");
    }
    out.map_err(err)
}

/// Show the planner's journal (`<data>/plans/`, one Markdown file per plan) in the Finder.
#[tauri::command]
fn reveal_plans_folder(state: State<'_, AppState>) -> CmdResult<()> {
    std::fs::create_dir_all(&state.plans_dir).map_err(err)?;
    tauri_plugin_opener::reveal_item_in_dir(&state.plans_dir).map_err(err)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct KeepState {
    stage: String,
    records: u64,
    built_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether a run started now with these inputs would skip the stage. None when it is not built.
    fresh: Option<bool>,
    refresh: String,
    last: Option<JournalEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct KeepStateArgs {
    recipe_id: String,
    #[serde(default)]
    inputs: indexmap::IndexMap<String, Value>,
    /// The user's model choices for this recipe, so the preview agrees with the run about which model a stage would use.
    #[serde(default)]
    pins: BTreeMap<String, String>,
}

/// What each kept stage of a recipe holds and whether it is fresh for these inputs.
#[tauri::command]
async fn keep_states(state: State<'_, AppState>, args: KeepStateArgs) -> CmdResult<Vec<KeepState>> {
    let bundle = state.library.find(&args.recipe_id).ok_or_else(|| format!("recipe {} not found", args.recipe_id))?;
    // The models a run would use now; when nothing fits, freshness is judged by sources alone.
    let installed = state.router().list_all().await;
    let resolutions = resolve_slots_with(&bundle.recipe, &installed, &args.pins, &state.resolve_defaults()).ok();
    let mut inputs = args.inputs;
    for (name, default) in bundle.recipe.optional_inputs() {
        inputs.entry(name).or_insert(default);
    }
    let mut out = Vec::new();
    for st in bundle.recipe.stages.iter().filter(|s| s.keep.is_some()) {
        let keep = Keep::open(keep_dir(&state.keep_base, &bundle.recipe.id, st.keep_id())).map_err(err)?;
        let manifest = keep.manifest().map_err(err)?;
        let built = manifest.built_at.is_some() && st.steps.iter().all(|s| manifest.outputs.contains_key(s));
        let refresh = st.keep.as_ref().map(|k| k.refresh).unwrap_or_default();
        let fresh = built.then(|| match refresh {
            stillroom_core::recipe::Refresh::Always => false,
            stillroom_core::recipe::Refresh::Manual => true,
            stillroom_core::recipe::Refresh::WhenSourcesChange => {
                keep.format().ok().flatten() == Some(stillroom_core::FINGERPRINT_FORMAT)
                    && manifest.source_fingerprint.as_deref() == Some(stage_fingerprint(&bundle.recipe, &bundle.prompts, st, &inputs).as_str())
                    && resolutions.as_ref().is_none_or(|r| manifest.models == stage_models(&bundle.recipe, st, r))
            }
        });
        out.push(KeepState {
            stage: st.id.clone(),
            records: keep.count().map_err(err)?,
            built_at: manifest.built_at,
            fresh,
            refresh: serde_json::to_value(refresh).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default(),
            last: keep.journal(1).map_err(err)?.into_iter().next(),
        });
    }
    Ok(out)
}

/// Open the keep of one stage of a known recipe. The stage id is checked against the recipe before
/// it becomes part of a path, so the webview cannot name a folder the recipe does not have.
fn open_keep(state: &AppState, recipe_id: &str, stage: &str) -> CmdResult<Keep> {
    let bundle = state.library.find(recipe_id).ok_or_else(|| format!("recipe {recipe_id} not found"))?;
    let Some(st) = bundle.recipe.stages.iter().find(|s| s.id == stage) else {
        return Err(format!("recipe {recipe_id} has no stage named {stage}"));
    };
    Keep::open(keep_dir(&state.keep_base, &bundle.recipe.id, st.keep_id())).map_err(err)
}

/// Empty a stage's run keep now; the next run builds it again from scratch.
#[tauri::command]
fn rebuild_keep(state: State<'_, AppState>, recipe_id: String, stage: String) -> CmdResult<()> {
    open_keep(&state, &recipe_id, &stage)?.clear("you").map_err(err)
}

/// The journal of a stage's keep, newest first.
#[tauri::command]
fn keep_journal(state: State<'_, AppState>, recipe_id: String, stage: String, limit: Option<usize>) -> CmdResult<Vec<JournalEntry>> {
    open_keep(&state, &recipe_id, &stage)?.journal(limit.unwrap_or(50)).map_err(err)
}

/// Records in a stage's keep, newest first.
#[tauri::command]
fn keep_records(state: State<'_, AppState>, recipe_id: String, stage: String, limit: Option<usize>) -> CmdResult<Vec<KeepRecord>> {
    open_keep(&state, &recipe_id, &stage)?.query(None, true, limit.unwrap_or(200)).map_err(err)
}

/// The recipe as it is on disk, for editing.
#[tauri::command]
fn read_recipe_draft(state: State<'_, AppState>, recipe_id: String) -> CmdResult<DraftResponse> {
    let bundle = state.library.find(&recipe_id).ok_or_else(|| format!("no recipe {recipe_id}"))?;
    let draft = Draft::read(&bundle.root).map_err(err)?;
    Ok(DraftResponse { id: bundle.recipe.id, source: bundle.source, path: bundle.root.to_string_lossy().into_owned(), draft })
}

fn known_tools(state: &AppState) -> Vec<String> {
    state.tools.ids().into_iter().map(String::from).collect()
}

/// Everything the checker has to say about a draft, in words: Problems (which stop a save) and
/// Notes (which do not, and may carry a fix id for `fix_recipe_draft`). Empty means it would
/// load and run as it is.
#[tauri::command]
fn check_recipe_draft(state: State<'_, AppState>, draft: Draft) -> CmdResult<Vec<Finding>> {
    Ok(draft.check_with(&known_tools(&state), Some(&state.stage_library)))
}

/// Apply the named fixes (the ids on the Notes `check_recipe_draft` returned) and hand back the
/// rewritten draft. Nothing is written to disk. An unknown fix id is an error.
#[tauri::command]
fn fix_recipe_draft(state: State<'_, AppState>, draft: Draft, fixes: Vec<String>) -> CmdResult<Draft> {
    draft.fix_with(&fixes, Some(&state.stage_library))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveResponse {
    recipe: RecipeInfo,
    path: String,
    /// True when a catalog recipe (built-in, community, or sample) with the same id is now hidden behind this one.
    shadows_builtin: bool,
}

/// Write the draft into the user's recipes folder as `<id>/`. A user recipe with that id is
/// replaced; a catalog recipe with that id stays on disk but the saved copy is the one listed.
/// `expect_new` says the caller is making a recipe, not editing one: a folder that already
/// exists is then refused instead of written over.
#[tauri::command]
fn save_recipe_draft(state: State<'_, AppState>, draft: Draft, expect_new: Option<bool>) -> CmdResult<SaveResponse> {
    let id = draft.id().ok_or("the recipe needs an id")?.to_string();
    let (user_root, _) = state
        .library
        .roots()
        .iter()
        .find(|(_, s)| *s == RecipeSource::User)
        .ok_or("no folder for your recipes")?;
    let dir = user_root.join(&id);
    if expect_new.unwrap_or(false) && dir.exists() {
        return Err(format!("A recipe with the id \u{201c}{id}\u{201d} already exists; pick another."));
    }
    let shadows_builtin = state.library.find(&id).map(|b| !b.source.is_user()).unwrap_or(false);
    draft.write(&dir, &known_tools(&state)).map_err(err)?;
    let bundle = state.library.find(&id).ok_or("saved, but the recipe did not load back")?;
    Ok(SaveResponse { recipe: bundle.info(), path: dir.to_string_lossy().into_owned(), shadows_builtin })
}

/// Copy a recipe into the user's folder as `new_id` (Duplicate): every file comes along,
/// `recipe.yaml` is rewritten with the new id, name, and icon (omitted keeps the original's),
/// and a copy of a catalog recipe records where it came from in `x-source`. An id that is
/// taken, or not in the id pattern, is refused in words.
#[tauri::command]
fn duplicate_recipe(app: AppHandle, state: State<'_, AppState>, recipe_id: String, new_id: String, name: String, icon: Option<String>) -> CmdResult<RecipeInfo> {
    let copy = stillroom_core::catalog::duplicate(&state.library, &recipe_id, &new_id, &name, icon.as_deref(), &known_tools(&state)).map_err(err)?;
    let _ = app.emit("shelf:changed", ShelfChanged { recipe_id: new_id, state: Some(ShelfState::Shelf) });
    Ok(copy.info())
}

/// Delete one of the user's recipes (its folder), its run keeps, and any watches and schedules
/// bound to it. Catalog recipes (built-in, community, sample) cannot be deleted; they leave the
/// shelf instead.
#[tauri::command]
fn delete_recipe(
    app: AppHandle,
    state: State<'_, AppState>,
    rt: State<'_, WatchRuntime>,
    schedules_rt: State<'_, ScheduleRuntime>,
    recipe_id: String,
) -> CmdResult<()> {
    let bundle = state.library.find(&recipe_id).ok_or_else(|| format!("recipe {recipe_id} not found"))?;
    if !bundle.source.is_user() {
        return Err("Recipes from the catalog cannot be deleted; take one off the shelf instead.".into());
    }
    let (user_root, _) = state
        .library
        .roots()
        .iter()
        .find(|(_, s)| *s == RecipeSource::User)
        .ok_or("no folder for your recipes")?;
    let dir = user_root.join(&recipe_id);
    if dir.canonicalize().map_err(err)? != bundle.root.canonicalize().map_err(err)? {
        return Err("that recipe is not in your recipes folder".into());
    }
    {
        let store = state.store.lock().map_err(err)?;
        for w in store.list_watches().map_err(err)? {
            if w.recipe_id == recipe_id {
                rt.stop(&w.id);
                store.delete_watch(&w.id).map_err(err)?;
            }
        }
        for s in store.list_schedules().map_err(err)? {
            if s.recipe_id == recipe_id {
                schedules_rt.stop(&s.id);
                store.delete_schedule(&s.id).map_err(err)?;
            }
        }
    }
    std::fs::remove_dir_all(&dir).map_err(err)?;
    // Set up promises that deleting the recipe deletes what its kept stages built (recipe-data/<id>/).
    match std::fs::remove_dir_all(state.keep_base.join(&bundle.recipe.id)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("the recipe is deleted, but its run keeps could not be removed: {e}")),
    }
    // Its place on the shelf goes with it; a catalog recipe of the same id, now unshadowed, starts over in the catalog.
    {
        let store = state.store.lock().map_err(err)?;
        let _ = store.remove_shelf(&recipe_id).map_err(err)?;
        let _ = store.revoke_grants(&recipe_id).map_err(err)?;
        // The runs stay (they hold results) but say the recipe is gone, and the id stays reserved.
        let _ = store.mark_runs_deleted(&recipe_id).map_err(err)?;
    }
    let _ = app.emit("watch:changed", ());
    let _ = app.emit("schedule:changed", ());
    let _ = app.emit("shelf:changed", ShelfChanged { recipe_id, state: None });
    Ok(())
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ShelfChanged {
    recipe_id: String,
    /// The new place, or `None` when the recipe is gone.
    state: Option<ShelfState>,
}

/// Put a recipe on the shelf, into the archive, or (a catalog recipe only) back in the catalog.
/// Works for every source. Other windows hear `shelf:changed` and list again.
#[tauri::command]
fn set_shelf(app: AppHandle, app_state: State<'_, AppState>, recipe_id: String, state: ShelfState) -> CmdResult<()> {
    let bundle = app_state.library.find(&recipe_id).ok_or_else(|| format!("recipe {recipe_id} not found"))?;
    if bundle.source.is_user() && state == ShelfState::Catalog {
        return Err("Your own recipes stay on the shelf or in the archive; delete one to remove it.".into());
    }
    app_state.store.lock().map_err(err)?.set_shelf(&recipe_id, state).map_err(err)?;
    let _ = app.emit("shelf:changed", ShelfChanged { recipe_id, state: Some(state) });
    Ok(())
}

/// Record the user's yes to permission entries of a recipe (the consent step at Set up or on
/// import). Only entries the recipe really lists as literal are recorded; the rest are ignored.
/// Every window hears `recipes:changed`.
#[tauri::command]
fn grant_permissions(app: AppHandle, state: State<'_, AppState>, recipe_id: String, entries: Vec<Grant>) -> CmdResult<RecipeInfo> {
    let bundle = state.library.find(&recipe_id).ok_or_else(|| format!("recipe {recipe_id} not found"))?;
    let literal = consent::literal_entries(&bundle.recipe);
    let accepted: Vec<Grant> = literal.into_iter().filter(|l| entries.iter().any(|e| e.kind == l.kind && e.entry == l.entry)).collect();
    state.store.lock().map_err(err)?.grant(&recipe_id, &accepted).map_err(err)?;
    let _ = app.emit("recipes:changed", ());
    state.info_of(&bundle)
}

/// What `import_recipe` hands back: the recipe as it would be listed (source `user`, every
/// literal permission still needed), what the checker said, and its weight. Nothing is
/// installed until `confirm_import`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportPreview {
    token: String,
    info: RecipeInfo,
    findings: Vec<Finding>,
    size_bytes: u64,
}

/// Get a recipe from a folder or a `.zip`: the dialog runs in Rust, the bundle is copied into
/// quarantine with links, escaping paths, and hidden entries refused, brought up to the current
/// format, and checked. `kind` is `"folder"` for a folder dialog; anything else asks for a zip
/// (the two cannot share one dialog). `None` when the user cancelled. A recipe with Problems is
/// refused here, in words, and nothing is kept.
#[tauri::command]
async fn import_recipe(app: AppHandle, state: State<'_, AppState>, kind: Option<String>) -> CmdResult<Option<ImportPreview>> {
    use tauri_plugin_dialog::DialogExt;
    let picked = if kind.as_deref() == Some("folder") {
        app.dialog().file().set_title("Get a recipe from a folder").blocking_pick_folder()
    } else {
        app.dialog().file().set_title("Get a recipe from a zip").add_filter("Zip archive", &["zip"]).blocking_pick_file()
    };
    let Some(picked) = picked else { return Ok(None) };
    let src = picked.into_path().map_err(err)?;
    let got = stillroom_core::catalog::import(&src, &state.quarantine_dir).map_err(err)?;
    let discard = |root: &PathBuf| {
        let _ = std::fs::remove_dir_all(root);
    };
    let tools = known_tools(&state);
    let draft = match Draft::read(&got.root) {
        Ok(d) => d,
        Err(e) => {
            discard(&got.root);
            return Err(format!("This recipe could not be read: {e}"));
        }
    };
    let mut findings = draft.check_with(&tools, Some(&state.stage_library));
    let problems: Vec<String> = findings.iter().filter(|f| f.is_problem()).map(|f| f.text.clone()).collect();
    if !problems.is_empty() {
        discard(&got.root);
        return Err(format!("This recipe cannot be imported as it is:\n{}", problems.join("\n")));
    }
    // Keep the migrated form, so what the user confirms is what is installed.
    if let Err(e) = draft.write(&got.root, &tools) {
        discard(&got.root);
        return Err(e.to_string());
    }
    let bundle = match RecipeBundle::load_for(&got.root, RecipeSource::User, state.library.app_version()) {
        Ok(b) => b,
        Err(e) => {
            discard(&got.root);
            return Err(e.to_string());
        }
    };
    if state.library.find(&bundle.recipe.id).is_some() {
        findings.push(Finding::problem(stillroom_core::CatalogError::Taken(bundle.recipe.id.clone()).to_string()));
    }
    Ok(Some(ImportPreview { token: got.token, info: bundle.info(), findings, size_bytes: got.size_bytes }))
}

/// Move a quarantined import into the user's recipes folder, record the grants the user ticked,
/// and put it on the shelf. An id that exists already is refused in the catalog's words.
#[tauri::command]
fn confirm_import(app: AppHandle, state: State<'_, AppState>, token: String, grants: Vec<Grant>) -> CmdResult<RecipeInfo> {
    let bundle = stillroom_core::catalog::install(&state.quarantine_dir, &token, &state.library).map_err(err)?;
    let id = bundle.recipe.id.clone();
    let literal = consent::literal_entries(&bundle.recipe);
    let accepted: Vec<Grant> = literal.into_iter().filter(|l| grants.iter().any(|g| g.kind == l.kind && g.entry == l.entry)).collect();
    {
        let store = state.store.lock().map_err(err)?;
        store.grant(&id, &accepted).map_err(err)?;
        store.set_shelf(&id, ShelfState::Shelf).map_err(err)?;
    }
    let _ = app.emit("shelf:changed", ShelfChanged { recipe_id: id, state: Some(ShelfState::Shelf) });
    let _ = app.emit("recipes:changed", ());
    state.info_of(&bundle)
}

/// Share a recipe as `<id>.zip` where the user chooses (the dialog runs in Rust): recipe.yaml,
/// prompts, schemas, skills, assets, and the example, under a folder named after the id. Keeps,
/// runs, pins, and shelf state live outside the bundle and are never in it; `runs/`, `keeps/`,
/// hidden files, links, and anything a `.stillroomignore` names are left out. `None` when the
/// user cancelled.
#[tauri::command]
async fn export_recipe(app: AppHandle, state: State<'_, AppState>, recipe_id: String) -> CmdResult<Option<String>> {
    let bundle = state.library.find(&recipe_id).ok_or_else(|| format!("recipe {recipe_id} not found"))?;
    let Some(dst) = ask_where_to_save(&app, &bundle.recipe.id, "zip")? else { return Ok(None) };
    stillroom_core::catalog::export_zip(&bundle, &dst).map_err(err)?;
    Ok(Some(dst.to_string_lossy().into_owned()))
}

/// Forget a quarantined import.
#[tauri::command]
fn discard_import(state: State<'_, AppState>, token: String) -> CmdResult<()> {
    let dir = stillroom_core::catalog::quarantine_dir(&state.quarantine_dir, &token).map_err(err)?;
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// The recipe's saved example run (`example/result.json` in the bundle), its file paths pointing
/// into the bundle's `example/` folder, so the Recipe page can show a result without a model.
#[tauri::command]
fn example_result(state: State<'_, AppState>, recipe_id: String) -> CmdResult<Run> {
    let bundle = state.library.find(&recipe_id).ok_or_else(|| format!("recipe {recipe_id} not found"))?;
    stillroom_core::catalog::example_result(&bundle).map_err(err)
}

/// Forget every grant for a recipe; it needs the consent step again before it runs.
#[tauri::command]
fn revoke_permissions(app: AppHandle, state: State<'_, AppState>, recipe_id: String) -> CmdResult<RecipeInfo> {
    let bundle = state.library.find(&recipe_id).ok_or_else(|| format!("recipe {recipe_id} not found"))?;
    state.store.lock().map_err(err)?.revoke_grants(&recipe_id).map_err(err)?;
    let _ = app.emit("recipes:changed", ());
    state.info_of(&bundle)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PermissionEntry {
    id: String,
    label: String,
    /// What it is for, in plain words.
    why: String,
    /// required, optional, or not_needed
    need: &'static str,
    /// granted, denied, unknown, not_needed, in_use, not_in_use
    status: &'static str,
    /// A System Settings privacy pane to open, when one applies.
    pane: Option<&'static str>,
}

/// What macOS may ask Stillroom for, and where each stands. With `probe`, the Files and Folders
/// entries are tested by listing the folder, which is what makes macOS ask in the first place.
#[tauri::command]
fn permission_status(app: AppHandle, state: State<'_, AppState>, probe: bool) -> CmdResult<Vec<PermissionEntry>> {
    let home = app.path().home_dir().map_err(err)?;
    let mut out = Vec::new();
    for (name, pane) in [("Desktop", "Privacy_DesktopFolder"), ("Documents", "Privacy_DocumentsFolder"), ("Downloads", "Privacy_DownloadsFolder")] {
        let status = if probe {
            match std::fs::read_dir(home.join(name)) {
                Ok(_) => "granted",
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => "denied",
                Err(_) => "unknown",
            }
        } else {
            "unknown"
        };
        out.push(PermissionEntry {
            id: format!("folder-{}", name.to_lowercase()),
            label: format!("Files and Folders · {name}"),
            why: format!("Recipes read what you choose and watches look at folders. If you pick something in {name}, macOS asks once."),
            need: "optional",
            status,
            pane: Some(pane),
        });
    }
    out.push(PermissionEntry {
        id: "keychain".into(),
        label: "Keychain".into(),
        why: "Holds the access key for a remote Ollama. macOS may ask to allow Stillroom the first time it reads it.".into(),
        need: "optional",
        status: if state.secret(OLLAMA_KEY).is_some() { "in_use" } else { "not_in_use" },
        pane: None,
    });
    out.push(PermissionEntry {
        id: "local-network".into(),
        label: "Local Network".into(),
        why: "Only if Ollama runs on another machine on your network. A local Ollama needs nothing.".into(),
        need: "optional",
        status: "unknown",
        pane: Some("Privacy_LocalNetwork"),
    });
    for (id, label) in [
        ("accessibility", "Accessibility"), ("screen-recording", "Screen Recording"), ("microphone", "Microphone"),
        ("camera", "Camera"), ("full-disk", "Full Disk Access"), ("automation", "Automation"), ("contacts-calendars", "Contacts and Calendars"),
    ] {
        out.push(PermissionEntry {
            id: id.into(),
            label: label.into(),
            why: "Stillroom does not use this.".into(),
            need: "not_needed",
            status: "not_needed",
            pane: None,
        });
    }
    Ok(out)
}

/// Open one System Settings privacy pane. Only the panes Stillroom can need.
#[tauri::command]
fn open_privacy_pane(app: AppHandle, pane: String) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    const PANES: &[&str] = &["Privacy_DesktopFolder", "Privacy_DocumentsFolder", "Privacy_DownloadsFolder", "Privacy_LocalNetwork", "Privacy"];
    if !PANES.contains(&pane.as_str()) {
        return Err(format!("unknown pane {pane}"));
    }
    app.opener()
        .open_url(format!("x-apple.systempreferences:com.apple.preference.security?{pane}"), None::<&str>)
        .map_err(err)
}

/// Show the user's recipes folder in the Finder.
#[tauri::command]
fn reveal_recipes_folder(state: State<'_, AppState>) -> CmdResult<()> {
    let (user_root, _) = state
        .library
        .roots()
        .iter()
        .find(|(_, s)| *s == RecipeSource::User)
        .ok_or("no folder for your recipes")?;
    tauri_plugin_opener::reveal_item_in_dir(user_root).map_err(err)
}

/// Show the sample folders (`samples/`: the sample recipe and the inbox, archive, and outbox it
/// works on) in the Finder, so the user can copy them somewhere of their own before running it.
#[tauri::command]
fn reveal_sample_folders(app: AppHandle) -> CmdResult<()> {
    let dir = catalog_dir(&app, "samples");
    if !dir.is_dir() {
        return Err("The sample folders did not ship with this build.".into());
    }
    tauri_plugin_opener::reveal_item_in_dir(dir).map_err(err)
}

#[tauri::command]
fn list_watches(state: State<'_, AppState>, rt: State<'_, WatchRuntime>) -> CmdResult<Vec<WatchView>> {
    let watches = state.store.lock().map_err(err)?.list_watches().map_err(err)?;
    Ok(watches.into_iter().map(|w| rt.view(w)).collect())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddWatchArgs {
    recipe_id: String,
    /// The folder input to watch.
    input: String,
    inputs: indexmap::IndexMap<String, Value>,
    #[serde(default)]
    pins: BTreeMap<String, String>,
}

/// Save a watch, start watching, and run once to catch up with what is already there.
#[tauri::command]
fn add_watch(app: AppHandle, state: State<'_, AppState>, rt: State<'_, WatchRuntime>, args: AddWatchArgs) -> CmdResult<WatchView> {
    let bundle = state.library.find(&args.recipe_id).ok_or_else(|| format!("recipe {} not found", args.recipe_id))?;
    state.check_consent(&bundle)?;
    let folder = args.inputs.get(&args.input).and_then(Value::as_str).ok_or_else(|| format!("choose a folder for {}", args.input))?;
    let mut watch = Watch::new(&bundle.recipe, &args.input, folder, args.inputs.clone(), args.pins).map_err(err)?;
    {
        let store = state.store.lock().map_err(err)?;
        // Continue from a run already done on these very inputs, so nothing is done twice.
        watch.last_run_id = store.latest_run_with_inputs(&watch.recipe_id, &watch.inputs).map_err(err)?.map(|r| r.id);
        store.save_watch(&watch).map_err(err)?;
    }
    rt.start(&app, &watch);
    watches::fire(&app, &watch.id, Vec::new());
    let _ = app.emit("watch:changed", ());
    Ok(rt.view(watch))
}

#[tauri::command]
fn set_watch_enabled(app: AppHandle, state: State<'_, AppState>, rt: State<'_, WatchRuntime>, id: String, enabled: bool) -> CmdResult<WatchView> {
    let mut watch = state.store.lock().map_err(err)?.get_watch(&id).map_err(err)?.ok_or("that watch is gone")?;
    watch.enabled = enabled;
    state.store.lock().map_err(err)?.save_watch(&watch).map_err(err)?;
    if enabled {
        rt.start(&app, &watch);
    } else {
        rt.stop(&id);
    }
    let _ = app.emit("watch:changed", ());
    Ok(rt.view(watch))
}

#[tauri::command]
fn remove_watch(app: AppHandle, state: State<'_, AppState>, rt: State<'_, WatchRuntime>, id: String) -> CmdResult<()> {
    rt.stop(&id);
    state.store.lock().map_err(err)?.delete_watch(&id).map_err(err)?;
    let _ = app.emit("watch:changed", ());
    Ok(())
}

/// Run a watch's recipe now, without waiting for a change.
#[tauri::command]
fn fire_watch(app: AppHandle, state: State<'_, AppState>, rt: State<'_, WatchRuntime>, id: String) -> CmdResult<()> {
    let watch = state.store.lock().map_err(err)?.get_watch(&id).map_err(err)?.ok_or("that watch is gone")?;
    if !watch.enabled {
        return Err("the watch is paused; resume it first".into());
    }
    if rt.view(watch.clone()).phase == watches::Phase::Failed {
        rt.start(&app, &watch);
    }
    watches::fire(&app, &id, Vec::new());
    Ok(())
}

#[tauri::command]
fn list_schedules(state: State<'_, AppState>, rt: State<'_, ScheduleRuntime>) -> CmdResult<Vec<ScheduleView>> {
    let schedules = state.store.lock().map_err(err)?.list_schedules().map_err(err)?;
    Ok(schedules.into_iter().map(|s| rt.view(s)).collect())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddScheduleArgs {
    recipe_id: String,
    inputs: indexmap::IndexMap<String, Value>,
    #[serde(default)]
    pins: BTreeMap<String, String>,
    /// Local time of day, 24-hour, `HH:MM`.
    time: String,
    /// `mon`…`sun`; empty means every day.
    #[serde(default)]
    days: Vec<String>,
}

/// Save a schedule and count it in. It first fires when its time next comes; nothing runs now.
/// Refused like a run or a watch while the recipe lacks the user's yes to its permissions.
#[tauri::command]
fn add_schedule(app: AppHandle, state: State<'_, AppState>, rt: State<'_, ScheduleRuntime>, args: AddScheduleArgs) -> CmdResult<ScheduleView> {
    let bundle = state.library.find(&args.recipe_id).ok_or_else(|| format!("recipe {} not found", args.recipe_id))?;
    state.check_consent(&bundle)?;
    let schedule = Schedule::new(&bundle.recipe, &args.time, &args.days, args.inputs, args.pins).map_err(err)?;
    state.store.lock().map_err(err)?.save_schedule(&schedule).map_err(err)?;
    rt.start(&schedule);
    let _ = app.emit("schedule:changed", ());
    Ok(rt.view(schedule))
}

#[tauri::command]
fn set_schedule_enabled(app: AppHandle, state: State<'_, AppState>, rt: State<'_, ScheduleRuntime>, id: String, enabled: bool) -> CmdResult<ScheduleView> {
    let mut schedule = state.store.lock().map_err(err)?.get_schedule(&id).map_err(err)?.ok_or("that schedule is gone")?;
    schedule.enabled = enabled;
    state.store.lock().map_err(err)?.save_schedule(&schedule).map_err(err)?;
    if enabled {
        rt.start(&schedule);
    } else {
        rt.stop(&id);
    }
    let _ = app.emit("schedule:changed", ());
    Ok(rt.view(schedule))
}

#[tauri::command]
fn delete_schedule(app: AppHandle, state: State<'_, AppState>, rt: State<'_, ScheduleRuntime>, id: String) -> CmdResult<()> {
    rt.stop(&id);
    state.store.lock().map_err(err)?.delete_schedule(&id).map_err(err)?;
    let _ = app.emit("schedule:changed", ());
    Ok(())
}

/// Run a schedule's recipe now, without waiting for its time, and hand back the run.
#[tauri::command]
async fn fire_schedule(app: AppHandle, state: State<'_, AppState>, rt: State<'_, ScheduleRuntime>, id: String) -> CmdResult<Run> {
    let schedule = state.store.lock().map_err(err)?.get_schedule(&id).map_err(err)?.ok_or("that schedule is gone")?;
    if !schedule.enabled {
        return Err("the schedule is paused; resume it first".into());
    }
    rt.start(&schedule);
    schedules::fire(&app, &id, false).await
}

/// A file extension as the webview may suggest it for a save: short, letters and digits only.
fn checked_extension(ext: &str) -> CmdResult<&str> {
    let ext = ext.trim().trim_start_matches('.');
    if ext.is_empty() || ext.len() > 8 || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!("{ext:?} is not a file extension Stillroom can save as"));
    }
    Ok(ext)
}

/// Ask the user where to save, from Rust, so the webview never names a path. The suggested name is
/// only a suggestion: path separators are dropped and the extension is appended. `None` when the
/// user cancelled.
fn ask_where_to_save(app: &AppHandle, name: &str, ext: &str) -> CmdResult<Option<PathBuf>> {
    use tauri_plugin_dialog::DialogExt;
    let ext = checked_extension(ext)?;
    let stem: String = name.chars().filter(|c| !matches!(c, '/' | '\\' | ':' | '\0')).collect();
    let stem = stem.trim().trim_start_matches('.');
    let stem = if stem.is_empty() { "artifact" } else { stem };
    let picked = app.dialog().file().set_file_name(format!("{stem}.{ext}")).add_filter(ext.to_uppercase(), &[ext]).blocking_save_file();
    match picked {
        None => Ok(None),
        Some(p) => p.into_path().map(Some).map_err(err),
    }
}

/// Save text where the user chooses (the dialog runs in Rust). Returns the path written, or `None`
/// when the user cancelled.
#[tauri::command]
async fn save_text_as(app: AppHandle, name: String, ext: String, content: String) -> CmdResult<Option<String>> {
    let Some(dst) = ask_where_to_save(&app, &name, &ext)? else { return Ok(None) };
    std::fs::write(&dst, content).map_err(|e| format!("{}: {e}", dst.display()))?;
    Ok(Some(dst.to_string_lossy().into_owned()))
}

/// Save binary data (base64 over IPC) where the user chooses. Returns the path written, or `None`
/// when the user cancelled.
#[tauri::command]
async fn save_binary_as(app: AppHandle, name: String, ext: String, data_base64: String) -> CmdResult<Option<String>> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data_base64).map_err(err)?;
    let Some(dst) = ask_where_to_save(&app, &name, &ext)? else { return Ok(None) };
    std::fs::write(&dst, bytes).map_err(|e| format!("{}: {e}", dst.display()))?;
    Ok(Some(dst.to_string_lossy().into_owned()))
}

/// Whether a run's record names `path` as a file it produced: an object with `"path": <path>`
/// somewhere in its outputs or step outputs.
fn run_names_file(run: &Run, path: &str) -> bool {
    fn names(v: &Value, path: &str) -> bool {
        match v {
            Value::Object(m) => m.get("path").and_then(Value::as_str) == Some(path) || m.values().any(|v| names(v, path)),
            Value::Array(a) => a.iter().any(|v| names(v, path)),
            _ => false,
        }
    }
    run.outputs.values().any(|v| names(v, path)) || run.steps.iter().filter_map(|s| s.output.as_ref()).any(|v| names(v, path))
}

/// Copy a file a run produced (audio, say) to where the user chooses. The source is not taken as a
/// free path: it must be named by the run's record and lie inside Stillroom's results folder.
#[tauri::command]
async fn save_artifact_as(app: AppHandle, state: State<'_, AppState>, run_id: String, artifact_path: String, name: String, ext: String) -> CmdResult<Option<String>> {
    let src = {
        let run = state.store.lock().map_err(err)?.get_run(&run_id).map_err(err)?.ok_or("that run is gone from the history")?;
        if !run_names_file(&run, &artifact_path) {
            return Err("that file is not one of the run's results".into());
        }
        let src = PathBuf::from(&artifact_path).canonicalize().map_err(|e| format!("the result file is gone: {e}"))?;
        let base = state.artifacts_dir.canonicalize().map_err(err)?;
        if !src.starts_with(&base) || !src.is_file() {
            return Err("that file is not in Stillroom's results folder".into());
        }
        src
    };
    let Some(dst) = ask_where_to_save(&app, &name, &ext)? else { return Ok(None) };
    std::fs::copy(&src, &dst).map_err(|e| format!("{} -> {}: {e}", src.display(), dst.display()))?;
    Ok(Some(dst.to_string_lossy().into_owned()))
}

/// Forget every cached step output, so the next run recomputes everything.
#[tauri::command]
fn clear_step_cache(state: State<'_, AppState>) -> CmdResult<usize> {
    state.store.lock().map_err(err)?.cache_clear().map_err(err)
}

/// The native menu. Every item emits a `menu` event with its id; the webview decides what it means.
fn build_menu(app: &AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};
    let item = |id: &str, text: &str, accel: Option<&str>| -> tauri::Result<tauri::menu::MenuItem<tauri::Wry>> {
        let mut b = MenuItemBuilder::with_id(id, text);
        if let Some(a) = accel {
            b = b.accelerator(a);
        }
        b.build(app)
    };
    let app_menu = SubmenuBuilder::new(app, "Stillroom")
        .item(&item("about", "About Stillroom", None)?)
        .separator()
        .item(&item("settings", "Settings…", Some("CmdOrCtrl+,"))?)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;
    let file = SubmenuBuilder::new(app, "File")
        .item(&item("new-recipe", "New Recipe…", Some("CmdOrCtrl+N"))?)
        .item(&item("get-recipe", "Get a recipe…", None)?)
        .item(&item("reveal-recipes", "Show Recipes Folder", None)?)
        .separator()
        .item(&PredefinedMenuItem::close_window(app, Some("Close Window"))?)
        .build()?;
    let edit = SubmenuBuilder::new(app, "Edit").undo().redo().separator().cut().copy().paste().select_all().build()?;
    let view = SubmenuBuilder::new(app, "View")
        .item(&item("place-library", "Library", Some("CmdOrCtrl+0"))?)
        .item(&item("place-browse", "Catalog", Some("CmdOrCtrl+B"))?)
        .item(&item("place-recipe", "Recipe", Some("CmdOrCtrl+1"))?)
        .item(&item("place-setup", "Set Up", Some("CmdOrCtrl+2"))?)
        .item(&item("place-running", "Running", Some("CmdOrCtrl+3"))?)
        .item(&item("place-results", "Results", Some("CmdOrCtrl+4"))?)
        .item(&item("place-door", "Recipe Card", Some("CmdOrCtrl+5"))?)
        .separator()
        .item(&item("theme-green", "Green Theme", None)?)
        .item(&item("theme-amber", "Amber Theme", None)?)
        .item(&item("theme-paper", "Paper Theme", None)?)
        .separator()
        .item(&item("text-larger", "Larger Text", Some("CmdOrCtrl+Plus"))?)
        .item(&item("text-smaller", "Smaller Text", Some("CmdOrCtrl+-"))?)
        .build()?;
    let run = SubmenuBuilder::new(app, "Run")
        .item(&item("run", "Run Recipe", Some("CmdOrCtrl+R"))?)
        .item(&item("watch-folder", "Watch This Folder…", None)?)
        .build()?;
    let window = SubmenuBuilder::new(app, "Window").minimize().maximize().separator().close_window().build()?;
    let help = SubmenuBuilder::new(app, "Help")
        .item(&item("help", "Stillroom Help", None)?)
        .item(&item("shortcuts", "Keyboard Shortcuts", Some("CmdOrCtrl+/"))?)
        .build()?;
    MenuBuilder::new(app).items(&[&app_menu, &file, &edit, &view, &run, &window, &help]).build()
}

fn builtin_stages_dir(app: &AppHandle) -> PathBuf {
    if cfg!(debug_assertions) {
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../stages");
        dev.canonicalize().unwrap_or(dev)
    } else {
        app.path().resource_dir().map(|r| r.join("stages")).unwrap_or_default()
    }
}

/// One of the catalog folders that ship with the app: `recipes` (built-in), `community`, or
/// `samples`. In dev they are the repository folders; in release they are bundled resources
/// (see `bundle.resources` in tauri.conf.json).
fn catalog_dir(app: &AppHandle, name: &str) -> PathBuf {
    if cfg!(debug_assertions) {
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(name);
        dev.canonicalize().unwrap_or(dev)
    } else {
        app.path().resource_dir().map(|r| r.join(name)).unwrap_or_default()
    }
}

fn builtin_recipes_dir(app: &AppHandle) -> PathBuf {
    catalog_dir(app, "recipes")
}

/// The catalog roots in the order the Library lists them: built-in, community, sample.
fn catalog_roots(app: &AppHandle) -> Vec<(PathBuf, RecipeSource)> {
    vec![
        (builtin_recipes_dir(app), RecipeSource::Builtin),
        (catalog_dir(app, "community"), RecipeSource::Community),
        (catalog_dir(app, "samples"), RecipeSource::Sample),
    ]
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            let menu = build_menu(app.handle())?;
            app.set_menu(menu)?;
            app.on_menu_event(|app, event| {
                let _ = app.emit("menu", event.id().0.clone());
            });
            let data_dir = app.path().app_data_dir()?;
            let user_recipes = data_dir.join("recipes");
            std::fs::create_dir_all(&user_recipes)?;
            let artifacts_dir = data_dir.join("artifacts");
            std::fs::create_dir_all(&artifacts_dir)?;
            let keep_base = data_dir.join("recipe-data");
            std::fs::create_dir_all(&keep_base)?;
            let quarantine_dir = data_dir.join("quarantine");
            std::fs::create_dir_all(&quarantine_dir)?;
            let plans_dir = data_dir.join("plans");
            std::fs::create_dir_all(&plans_dir)?;
            // An import the user never confirmed is forgotten after a day.
            stillroom_core::catalog::prune_quarantine(&quarantine_dir, std::time::Duration::from_secs(24 * 3600));
            let db_path = data_dir.join("stillroom.db");
            let store = Store::open(&db_path).map_err(|e| std::io::Error::other(e.to_string()))?;
            let mut roots = catalog_roots(app.handle());
            roots.push((user_recipes, RecipeSource::User));
            let library = RecipeLibrary::with_app_version(roots, env!("CARGO_PKG_VERSION"));
            app.manage(AppState {
                store: Arc::new(Mutex::new(store)),
                secrets: Mutex::new(BTreeMap::new()),
                current_cancel: Mutex::new(None),
                plan_cancel: Mutex::new(None),
                tools: Arc::new(ToolRegistry::builtin()),
                library,
                prompts: SystemPrompts::new(data_dir.join("prompts")),
                db_path: db_path.to_string_lossy().into_owned(),
                artifacts_dir,
                keep_base,
                stage_library: Arc::new(stillroom_core::StageLibrary::load(&builtin_stages_dir(app.handle())).unwrap_or_default()),
                run_gate: tokio::sync::Mutex::new(()),
                quarantine_dir,
                memory_bytes: stillroom_core::model::physical_memory_bytes(),
                plans_dir,
            });
            app.manage(WatchRuntime::default());
            let rt = app.state::<WatchRuntime>();
            rt.start_all(app.handle(), &app.state::<AppState>());
            app.manage(ScheduleRuntime::default());
            let schedules_rt = app.state::<ScheduleRuntime>();
            schedules_rt.start_all(app.handle(), &app.state::<AppState>());
            updates::start_daily(app.handle().clone());
            // The main window is declared in tauri.conf.json with `create: false` and built here, so
            // it can refuse navigation away from the app's own pages (there is no config key for that).
            let main_config = app
                .config()
                .app
                .windows
                .iter()
                .find(|w| w.label == "main")
                .cloned()
                .ok_or_else(|| std::io::Error::other("tauri.conf.json has no window labelled main"))?;
            let nav = app.handle().clone();
            tauri::WebviewWindowBuilder::from_config(app.handle(), &main_config)?
                .on_navigation(move |url| is_app_page(&nav, url))
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            runtime_status,
            updates::check_for_updates,
            list_models,
            hardware,
            codex_status,
            try_model,
            stop_run,
            preview_resolution,
            get_settings,
            set_settings,
            pull_model,
            open_settings,
            set_secret,
            has_secret,
            list_recipes,
            run_recipe,
            rerun_with_edit,
            list_runs,
            get_run,
            usage,
            estimate,
            probe_permissions,
            list_tools,
            read_recipe_draft,
            check_recipe_draft,
            fix_recipe_draft,
            save_recipe_draft,
            list_system_prompts,
            read_system_prompt,
            save_system_prompt,
            use_system_prompt_version,
            preview_planner,
            try_planning,
            plan_recipe,
            keep_states,
            rebuild_keep,
            keep_journal,
            keep_records,
            list_watches,
            add_watch,
            set_watch_enabled,
            remove_watch,
            duplicate_recipe,
            delete_recipe,
            set_shelf,
            grant_permissions,
            revoke_permissions,
            example_result,
            import_recipe,
            confirm_import,
            discard_import,
            export_recipe,
            reveal_recipes_folder,
            reveal_plans_folder,
            reveal_sample_folders,
            permission_status,
            open_privacy_pane,
            open_link,
            fire_watch,
            list_schedules,
            add_schedule,
            set_schedule_enabled,
            delete_schedule,
            fire_schedule,
            save_text_as,
            save_binary_as,
            save_artifact_as,
            clear_step_cache
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
