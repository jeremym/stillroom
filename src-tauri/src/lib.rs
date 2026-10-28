//! Stillroom desktop shell: wires the recipe library, executor, model router, and store to the webview.

mod watches;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use stillroom_core::store::{ModelUsage, RunEstimate};
use stillroom_core::tools::{ToolInfo, ToolRegistry};
use stillroom_core::trace::RunSummary;
use stillroom_core::{
    edits_for_rerun, pins_from_run, resolve_slots_with, validate_edit, Cancel, Draft, Executor, ModelDefaults, RecipeInfo, RecipeLibrary, RecipeSource,
    ResolveDefaults, Run,
    RunTrigger, StepEdit, StepRecord, Store, Watch,
};
use stillroom_models::ollama::Ollama;
use stillroom_models::{InstalledModel, ModelRouter, ProviderStatus};
use tauri::{AppHandle, Emitter, Manager, State};
use watches::{WatchRuntime, WatchView};

pub(crate) struct AppState {
    store: Arc<Mutex<Store>>,
    /// Secrets read from the keychain once per process, so a run does not knock on it every time.
    secrets: Mutex<BTreeMap<String, Option<String>>>,
    /// The stop handle of the run in flight, if any. One run at a time.
    current_cancel: Mutex<Option<Cancel>>,
    tools: Arc<ToolRegistry>,
    library: RecipeLibrary,
    db_path: String,
    artifacts_dir: PathBuf,
    /// One run at a time: a watch firing while the user runs something waits its turn.
    run_gate: tokio::sync::Mutex<()>,
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

    /// The providers as configured right now. Cheap to build; a run keeps the one it started with.
    fn router(&self) -> Arc<ModelRouter> {
        Arc::new(ModelRouter::new(vec![Arc::new(self.ollama())]))
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

    /// The user's model defaults from Settings › Models.
    fn resolve_defaults(&self) -> ResolveDefaults {
        self.settings()
            .ok()
            .and_then(|s| s.get("models").cloned())
            .and_then(|m| serde_json::from_value(m).ok())
            .unwrap_or_default()
    }
}

const KEYCHAIN_SERVICE: &str = "com.jeremym.stillroom";
/// The keychain item for a remote Ollama's access key (sent as a bearer token).
const OLLAMA_KEY: &str = "ollama.apiKey";
const KNOWN_SECRETS: &[&str] = &[OLLAMA_KEY];

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
}

/// Resolve models, run the recipe with live step events, save the run, announce it.
pub(crate) async fn execute(app: &AppHandle, state: &AppState, args: ExecArgs) -> CmdResult<Run> {
    let bundle = state
        .library
        .find(&args.recipe_id)
        .ok_or_else(|| format!("recipe {} not found", args.recipe_id))?;
    let router = state.router();
    let installed = router.list_all().await;
    let resolutions = resolve_slots_with(&bundle.recipe, &installed, &args.pins, &state.resolve_defaults()).map_err(err)?;

    let _turn = state.run_gate.lock().await;
    let observer = {
        let app = app.clone();
        Arc::new(move |rec: &StepRecord| {
            let _ = app.emit("run:step", rec.clone());
        })
    };
    let executor = Executor::new(state.tools.clone(), router).with_cache(state.store.clone());
    let mut request = bundle.request(args.inputs, resolutions);
    request.artifact_base = Some(state.artifacts_dir.clone());
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
    let _ = app.emit("run:finished", run.summary());
    Ok(run)
}

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeStatus {
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
}

#[tauri::command]
async fn list_models(state: State<'_, AppState>) -> CmdResult<ModelsResponse> {
    let router = state.router();
    Ok(ModelsResponse { providers: router.statuses().await, models: router.list_all().await })
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

/// Stop the run in flight. Returns whether there was one.
#[tauri::command]
fn stop_run(state: State<'_, AppState>) -> CmdResult<bool> {
    let guard = state.current_cancel.lock().map_err(err)?;
    match guard.as_ref() {
        Some(c) => {
            c.cancel();
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Every setting as one JSON object. Keys: look, ollama, models, pins, firstRun.
#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> CmdResult<Value> {
    Ok(Value::Object(state.settings()?))
}

/// Merge a patch of top-level settings (a key set to null is removed), then tell every window.
#[tauri::command]
fn set_settings(app: AppHandle, state: State<'_, AppState>, patch: Value) -> CmdResult<Value> {
    let patch = patch.as_object().ok_or("settings patch must be an object")?;
    {
        let store = state.store.lock().map_err(err)?;
        for (k, v) in patch {
            store.set_setting(k, v).map_err(err)?;
        }
    }
    let all = Value::Object(state.settings()?);
    let _ = app.emit("settings:changed", all.clone());
    Ok(all)
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

/// Open the Settings window, or bring it forward if it is already open.
#[tauri::command]
fn open_settings(app: AppHandle) -> CmdResult<()> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("index.html?window=settings".into()))
        .title("Stillroom Settings")
        .inner_size(960.0, 760.0)
        .min_inner_size(760.0, 560.0)
        .build()
        .map_err(err)?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecipesResponse {
    recipes: Vec<RecipeInfo>,
    errors: Vec<RecipeLoadError>,
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
    Ok(RecipesResponse {
        recipes: scan.bundles.iter().map(|b| b.info()).collect(),
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
            recipe_id: prior.recipe_id.clone(),
            inputs: prior.inputs.clone(),
            pins: pins_from_run(&prior),
            reuse: args.reuse,
            edits,
            resume_from: Some(base),
            trigger: None,
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

#[tauri::command]
fn estimate(state: State<'_, AppState>, recipe_id: String) -> CmdResult<Option<RunEstimate>> {
    state.store.lock().map_err(err)?.estimate_for_recipe(&recipe_id).map_err(err)
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

/// Problems with a draft, in words. Empty means it would load and run.
#[tauri::command]
fn check_recipe_draft(state: State<'_, AppState>, draft: Draft) -> CmdResult<Vec<String>> {
    Ok(draft.check(&known_tools(&state)))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveResponse {
    recipe: RecipeInfo,
    path: String,
    /// True when a built-in recipe with the same id is now hidden behind this one.
    shadows_builtin: bool,
}

/// Write the draft into the user's recipes folder as `<id>/`. A user recipe with that id is
/// replaced; a built-in with that id stays on disk but the saved copy is the one listed.
#[tauri::command]
fn save_recipe_draft(state: State<'_, AppState>, draft: Draft) -> CmdResult<SaveResponse> {
    let id = draft.id().ok_or("the recipe needs an id")?.to_string();
    let (user_root, _) = state
        .library
        .roots()
        .iter()
        .find(|(_, s)| *s == RecipeSource::User)
        .ok_or("no folder for your recipes")?;
    let dir = user_root.join(&id);
    let shadows_builtin = state.library.find(&id).map(|b| b.source == RecipeSource::Builtin).unwrap_or(false);
    draft.write(&dir, &known_tools(&state)).map_err(err)?;
    let bundle = state.library.find(&id).ok_or("saved, but the recipe did not load back")?;
    Ok(SaveResponse { recipe: bundle.info(), path: dir.to_string_lossy().into_owned(), shadows_builtin })
}

/// Delete one of the user's recipes (its folder) and any watches bound to it. Built-ins cannot be deleted.
#[tauri::command]
fn delete_recipe(app: AppHandle, state: State<'_, AppState>, rt: State<'_, WatchRuntime>, recipe_id: String) -> CmdResult<()> {
    let bundle = state.library.find(&recipe_id).ok_or_else(|| format!("recipe {recipe_id} not found"))?;
    if bundle.source != RecipeSource::User {
        return Err("built-in recipes cannot be deleted; make a copy and change that".into());
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
    }
    std::fs::remove_dir_all(&dir).map_err(err)?;
    let _ = app.emit("watch:changed", ());
    Ok(())
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
fn write_text_file(path: String, content: String) -> CmdResult<()> {
    std::fs::write(&path, content).map_err(|e| format!("{path}: {e}"))
}

/// Write binary artifact data (base64-encoded over IPC) to a path the user chose in a save dialog.
#[tauri::command]
fn write_binary_file(path: String, data_base64: String) -> CmdResult<()> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data_base64).map_err(err)?;
    std::fs::write(&path, bytes).map_err(|e| format!("{path}: {e}"))
}

/// Forget every cached step output, so the next run recomputes everything.
#[tauri::command]
fn clear_step_cache(state: State<'_, AppState>) -> CmdResult<usize> {
    state.store.lock().map_err(err)?.cache_clear().map_err(err)
}

/// Copy an artifact file (e.g. audio) to a path the user chose in a save dialog.
#[tauri::command]
fn copy_file(src: String, dst: String) -> CmdResult<()> {
    std::fs::copy(&src, &dst).map(|_| ()).map_err(|e| format!("{src} -> {dst}: {e}"))
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
        .item(&item("reveal-recipes", "Show Recipes Folder", None)?)
        .separator()
        .item(&PredefinedMenuItem::close_window(app, Some("Close Window"))?)
        .build()?;
    let edit = SubmenuBuilder::new(app, "Edit").undo().redo().separator().cut().copy().paste().select_all().build()?;
    let view = SubmenuBuilder::new(app, "View")
        .item(&item("place-library", "Library", Some("CmdOrCtrl+0"))?)
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

fn builtin_recipes_dir(app: &AppHandle) -> PathBuf {
    if cfg!(debug_assertions) {
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../recipes");
        dev.canonicalize().unwrap_or(dev)
    } else {
        app.path().resource_dir().map(|r| r.join("recipes")).unwrap_or_default()
    }
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
            let db_path = data_dir.join("stillroom.db");
            let store = Store::open(&db_path).map_err(|e| std::io::Error::other(e.to_string()))?;
            let library = RecipeLibrary::new(vec![
                (builtin_recipes_dir(app.handle()), RecipeSource::Builtin),
                (user_recipes, RecipeSource::User),
            ]);
            app.manage(AppState {
                store: Arc::new(Mutex::new(store)),
                secrets: Mutex::new(BTreeMap::new()),
                current_cancel: Mutex::new(None),
                tools: Arc::new(ToolRegistry::builtin()),
                library,
                db_path: db_path.to_string_lossy().into_owned(),
                artifacts_dir,
                run_gate: tokio::sync::Mutex::new(()),
            });
            app.manage(WatchRuntime::default());
            let rt = app.state::<WatchRuntime>();
            rt.start_all(app.handle(), &app.state::<AppState>());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            runtime_status,
            list_models,
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
            list_tools,
            read_recipe_draft,
            check_recipe_draft,
            save_recipe_draft,
            list_watches,
            add_watch,
            set_watch_enabled,
            remove_watch,
            delete_recipe,
            reveal_recipes_folder,
            permission_status,
            open_privacy_pane,
            fire_watch,
            write_text_file,
            write_binary_file,
            copy_file,
            clear_step_cache
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
