//! Folder watches in the running app: starts a watcher per enabled watch, runs the recipe
//! when the folder settles, one run at a time, and tells the webview what happened.

use crate::{execute, AppState, ExecArgs};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use stillroom_core::{FolderWatcher, RunTrigger, Watch};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Waiting for something to land in the folder.
    Watching,
    Running,
    /// Something landed while a run was going; it runs again when that finishes.
    Queued,
    Paused,
    /// The watcher could not start (folder gone, recipe gone).
    Failed,
}

#[derive(Debug, Clone)]
struct Status {
    phase: Phase,
    note: Option<String>,
    /// Files that changed while a run was going, for the queued run.
    pending: Vec<String>,
}

struct Live {
    _watcher: Option<FolderWatcher>,
    status: Arc<Mutex<Status>>,
}

/// Watchers for the enabled watches, by watch id.
#[derive(Default)]
pub struct WatchRuntime {
    live: Mutex<HashMap<String, Live>>,
}

/// A watch as the UI sees it: what is saved plus what it is doing right now.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchView {
    #[serde(flatten)]
    pub watch: Watch,
    pub phase: Phase,
    pub note: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct WatchRunEvent {
    watch_id: String,
    recipe_id: String,
    phase: &'static str,
    run_id: Option<String>,
    status: Option<String>,
    error: Option<String>,
}

impl WatchRuntime {
    pub fn view(&self, watch: Watch) -> WatchView {
        let (phase, note) = self
            .live
            .lock()
            .ok()
            .and_then(|l| l.get(&watch.id).map(|x| x.status.lock().map(|s| (s.phase, s.note.clone())).unwrap_or((Phase::Failed, None))))
            .unwrap_or((Phase::Paused, None));
        WatchView { watch, phase, note }
    }

    /// Start watching for an enabled watch (replacing any watcher it already has).
    pub fn start(&self, app: &AppHandle, watch: &Watch) {
        let status = Arc::new(Mutex::new(Status { phase: Phase::Watching, note: None, pending: Vec::new() }));
        let watcher = {
            let app = app.clone();
            let id = watch.id.clone();
            FolderWatcher::start(&watch.folder, watch.settle(), move |paths| fire(&app, &id, names(&paths)))
        };
        let live = match watcher {
            Ok(w) => Live { _watcher: Some(w), status },
            Err(e) => {
                if let Ok(mut s) = status.lock() {
                    s.phase = Phase::Failed;
                    s.note = Some(format!("could not watch {}: {e}", watch.folder));
                }
                Live { _watcher: None, status }
            }
        };
        if let Ok(mut map) = self.live.lock() {
            map.insert(watch.id.clone(), live);
        }
    }

    pub fn stop(&self, id: &str) {
        if let Ok(mut map) = self.live.lock() {
            map.remove(id);
        }
    }

    /// Start every enabled watch saved in the store. Called once at launch.
    pub fn start_all(&self, app: &AppHandle, state: &AppState) {
        let watches = state.store.lock().ok().and_then(|s| s.list_watches().ok()).unwrap_or_default();
        for w in watches.into_iter().filter(|w| w.enabled) {
            self.start(app, &w);
        }
    }
}

fn names(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect()
}

/// Something landed (or the user pressed Run now): run the recipe, or queue a run if one
/// is already going.
pub fn fire(app: &AppHandle, id: &str, changed: Vec<String>) {
    let rt = app.state::<WatchRuntime>();
    {
        let Ok(map) = rt.live.lock() else { return };
        let Some(live) = map.get(id) else { return };
        let Ok(mut st) = live.status.lock() else { return };
        match st.phase {
            Phase::Running | Phase::Queued => {
                st.phase = Phase::Queued;
                st.pending.extend(changed);
                return;
            }
            Phase::Paused | Phase::Failed => return,
            Phase::Watching => st.phase = Phase::Running,
        }
    }
    let app = app.clone();
    let id = id.to_string();
    tauri::async_runtime::spawn(async move {
        let mut changed = changed;
        loop {
            run_once(&app, &id, changed).await;
            // Anything that landed during the run means one more run; otherwise back to watching.
            let again = {
                let rt = app.state::<WatchRuntime>();
                let Ok(map) = rt.live.lock() else { break };
                let Some(live) = map.get(&id) else { break };
                let Ok(mut st) = live.status.lock() else { break };
                if st.phase == Phase::Queued {
                    st.phase = Phase::Running;
                    Some(std::mem::take(&mut st.pending))
                } else {
                    st.phase = Phase::Watching;
                    None
                }
            };
            match again {
                Some(next) => changed = next,
                None => break,
            }
        }
    });
}

fn set_note(app: &AppHandle, id: &str, note: Option<String>) {
    let rt = app.state::<WatchRuntime>();
    if let Ok(map) = rt.live.lock() {
        if let Some(live) = map.get(id) {
            if let Ok(mut st) = live.status.lock() {
                st.note = note;
            }
        }
    }
}

async fn run_once(app: &AppHandle, id: &str, changed: Vec<String>) {
    let state = app.state::<AppState>();
    let (watch, prior) = {
        let Ok(store) = state.store.lock() else { return };
        let Some(watch) = store.get_watch(id).ok().flatten() else { return };
        if !watch.enabled {
            return;
        }
        let prior = watch.last_run_id.as_deref().and_then(|rid| store.continuation_base(rid).ok().flatten());
        (watch, prior)
    };
    let event = |phase: &'static str, run_id: Option<String>, status: Option<String>, error: Option<String>| WatchRunEvent {
        watch_id: watch.id.clone(),
        recipe_id: watch.recipe_id.clone(),
        phase,
        run_id,
        status,
        error,
    };
    let _ = app.emit("watch:run", event("started", None, None, None));
    let trigger = RunTrigger { kind: "watch".into(), watch_id: watch.id.clone(), folder: watch.folder.clone(), changed };
    let result = execute(
        app,
        &state,
        ExecArgs {
            recipe_id: watch.recipe_id.clone(),
            inputs: watch.inputs.clone(),
            pins: watch.pins.clone(),
            reuse: true,
            edits: Default::default(),
            resume_from: prior,
            trigger: Some(trigger),
        },
    )
    .await;
    match result {
        Ok(run) => {
            let status = serde_json::to_value(run.status).ok().and_then(|v| v.as_str().map(String::from));
            if let Ok(store) = state.store.lock() {
                if let Ok(Some(mut w)) = store.get_watch(id) {
                    w.last_run_id = Some(run.id.clone());
                    w.last_fired_at = Some(chrono::Utc::now());
                    let _ = store.save_watch(&w);
                }
            }
            set_note(app, id, run.error.clone());
            let _ = app.emit("watch:run", event("finished", Some(run.id), status, run.error));
        }
        Err(e) => {
            set_note(app, id, Some(e.clone()));
            let _ = app.emit("watch:run", event("failed", None, None, Some(e)));
        }
    }
    let _ = app.emit("watch:changed", ());
}
