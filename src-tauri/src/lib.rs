//! Stillroom desktop shell: wires the recipe library, executor, model router, and store to the webview.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use stillroom_core::store::{ModelUsage, RunEstimate};
use stillroom_core::tools::ToolRegistry;
use stillroom_core::trace::RunSummary;
use stillroom_core::{
    edits_for_rerun, pins_from_run, resolve_slots, validate_edit, Executor, RecipeInfo, RecipeLibrary, RecipeSource, Run, StepRecord,
    Store,
};
use stillroom_models::ollama::Ollama;
use stillroom_models::{InstalledModel, ModelRouter, ProviderStatus};
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    store: Arc<Mutex<Store>>,
    router: Arc<ModelRouter>,
    tools: Arc<ToolRegistry>,
    library: RecipeLibrary,
    db_path: String,
    artifacts_dir: PathBuf,
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
    let router = state.router.clone();
    Ok(ModelsResponse { providers: router.statuses().await, models: router.list_all().await })
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
    let bundle = state
        .library
        .find(&args.recipe_id)
        .ok_or_else(|| format!("recipe {} not found", args.recipe_id))?;
    let router = state.router.clone();
    let installed = router.list_all().await;
    let resolutions = resolve_slots(&bundle.recipe, &installed, &args.pins).map_err(err)?;

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
    let run = executor.run(request, Some(observer)).await;

    state.store.lock().map_err(err)?.save_run(&run).map_err(err)?;
    let _ = app.emit("run:finished", run.summary());
    Ok(run)
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
    let prior = state
        .store
        .lock()
        .map_err(err)?
        .get_run(&args.run_id)
        .map_err(err)?
        .ok_or_else(|| format!("run {} not found", args.run_id))?;
    let bundle = state
        .library
        .find(&prior.recipe_id)
        .ok_or_else(|| format!("recipe {} not found", prior.recipe_id))?;
    validate_edit(&bundle.recipe, &args.step_id, &args.output).map_err(err)?;
    let edits = edits_for_rerun(&prior, &args.step_id, args.output).map_err(err)?;

    let router = state.router.clone();
    let installed = router.list_all().await;
    let resolutions = resolve_slots(&bundle.recipe, &installed, &pins_from_run(&prior)).map_err(err)?;

    let observer = {
        let app = app.clone();
        Arc::new(move |rec: &StepRecord| {
            let _ = app.emit("run:step", rec.clone());
        })
    };
    let executor = Executor::new(state.tools.clone(), router).with_cache(state.store.clone());
    let mut request = bundle.request(prior.inputs.clone(), resolutions);
    request.artifact_base = Some(state.artifacts_dir.clone());
    request.reuse = args.reuse;
    request.edits = edits;
    let run = executor.run(request, Some(observer)).await;

    state.store.lock().map_err(err)?.save_run(&run).map_err(err)?;
    let _ = app.emit("run:finished", run.summary());
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
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let user_recipes = data_dir.join("recipes");
            std::fs::create_dir_all(&user_recipes)?;
            let artifacts_dir = data_dir.join("artifacts");
            std::fs::create_dir_all(&artifacts_dir)?;
            let db_path = data_dir.join("stillroom.db");
            let store = Store::open(&db_path).map_err(|e| std::io::Error::other(e.to_string()))?;
            let router = Arc::new(ModelRouter::new(vec![Arc::new(Ollama::default_local())]));
            let library = RecipeLibrary::new(vec![
                (builtin_recipes_dir(app.handle()), RecipeSource::Builtin),
                (user_recipes, RecipeSource::User),
            ]);
            app.manage(AppState {
                store: Arc::new(Mutex::new(store)),
                router,
                tools: Arc::new(ToolRegistry::builtin()),
                library,
                db_path: db_path.to_string_lossy().into_owned(),
                artifacts_dir,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            runtime_status,
            list_models,
            list_recipes,
            run_recipe,
            rerun_with_edit,
            list_runs,
            get_run,
            usage,
            estimate,
            write_text_file,
            write_binary_file,
            copy_file,
            clear_step_cache
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
