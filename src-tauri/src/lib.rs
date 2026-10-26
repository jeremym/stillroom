//! Stillroom desktop shell: wires the core executor, model router, and store to the webview.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use stillroom_core::tools::ToolRegistry;
use stillroom_core::trace::RunSummary;
use stillroom_core::{resolve_slots, Executor, Recipe, Run, RunRequest, StepRecord, Store};
use stillroom_models::ollama::Ollama;
use stillroom_models::{InstalledModel, ModelRouter, ProviderStatus};
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    store: Mutex<Store>,
    router: Arc<ModelRouter>,
    tools: Arc<ToolRegistry>,
    db_path: String,
}

/// Recipes compiled into the app. Later: a recipe library on disk.
struct BuiltinRecipe {
    yaml: &'static str,
    prompts: &'static [(&'static str, &'static str)],
}

const SUMMARIZE_FILE: BuiltinRecipe = BuiltinRecipe {
    yaml: include_str!("../../recipes/summarize-file/recipe.yaml"),
    prompts: &[("prompts/summarize.md", include_str!("../../recipes/summarize-file/prompts/summarize.md"))],
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeStatus {
    schema_title: String,
    db_path: String,
    tools: Vec<String>,
    run_count: i64,
}

#[tauri::command]
fn runtime_status(state: State<'_, AppState>) -> Result<RuntimeStatus, String> {
    let schema: Value = serde_json::from_str(stillroom_core::RECIPE_SCHEMA).map_err(|e| e.to_string())?;
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let run_count = store.list_runs(10_000).map_err(|e| e.to_string())?.len() as i64;
    let mut tools: Vec<String> = state.tools.ids().into_iter().map(String::from).collect();
    tools.sort();
    Ok(RuntimeStatus {
        schema_title: schema["title"].as_str().unwrap_or("?").to_string(),
        db_path: state.db_path.clone(),
        tools,
        run_count,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelsResponse {
    providers: Vec<ProviderStatus>,
    models: Vec<InstalledModel>,
}

#[tauri::command]
async fn list_models(state: State<'_, AppState>) -> Result<ModelsResponse, String> {
    let router = state.router.clone();
    Ok(ModelsResponse { providers: router.statuses().await, models: router.list_all().await })
}

#[tauri::command]
fn builtin_recipe() -> Result<Recipe, String> {
    Recipe::from_yaml(SUMMARIZE_FILE.yaml).map_err(|e| e.to_string())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunArgs {
    inputs: BTreeMap<String, Value>,
    #[serde(default)]
    pins: BTreeMap<String, String>,
}

#[tauri::command]
async fn run_builtin(app: AppHandle, state: State<'_, AppState>, args: RunArgs) -> Result<Run, String> {
    let recipe = Recipe::from_yaml(SUMMARIZE_FILE.yaml).map_err(|e| e.to_string())?;
    let router = state.router.clone();
    let installed = router.list_all().await;
    let resolutions = resolve_slots(&recipe, &installed, &args.pins).map_err(|e| e.to_string())?;
    let prompts: HashMap<String, String> =
        SUMMARIZE_FILE.prompts.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();

    let observer = {
        let app = app.clone();
        Arc::new(move |rec: &StepRecord| {
            let _ = app.emit("run:step", rec.clone());
        })
    };
    let executor = Executor::new(state.tools.clone(), router);
    let run = executor
        .run(RunRequest { recipe, prompts, inputs: args.inputs, resolutions }, Some(observer))
        .await;

    state.store.lock().map_err(|e| e.to_string())?.save_run(&run).map_err(|e| e.to_string())?;
    let _ = app.emit("run:finished", run.summary());
    Ok(run)
}

#[tauri::command]
fn list_runs(state: State<'_, AppState>, limit: Option<usize>) -> Result<Vec<RunSummary>, String> {
    state.store.lock().map_err(|e| e.to_string())?.list_runs(limit.unwrap_or(50)).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_run(state: State<'_, AppState>, id: String) -> Result<Option<Run>, String> {
    state.store.lock().map_err(|e| e.to_string())?.get_run(&id).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let db_path = dir.join("stillroom.db");
            let store = Store::open(&db_path).map_err(|e| std::io::Error::other(e.to_string()))?;
            let router = Arc::new(ModelRouter::new(vec![Arc::new(Ollama::default_local())]));
            app.manage(AppState {
                store: Mutex::new(store),
                router,
                tools: Arc::new(ToolRegistry::builtin()),
                db_path: db_path.to_string_lossy().into_owned(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            runtime_status,
            list_models,
            builtin_recipe,
            run_builtin,
            list_runs,
            get_run
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
