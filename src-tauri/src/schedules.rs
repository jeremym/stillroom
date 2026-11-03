//! Schedules in the running app: one ticker checks every half minute whether an enabled
//! schedule's time has come, runs the recipe through the same door watches use (the run gate,
//! the consent check), and tells the webview what happened. Runs only while the app is open.
//!
//! The rule is the same at every tick and at launch: a schedule fires when a scheduled moment
//! lies between its last fire (or its creation) and now. That is what `Schedule::missed`
//! answers, so there is no timer state to lose; the clock, the store, and the rule decide.
//! At launch the first pass waits ten seconds so the window is up first, and the runs it
//! starts are marked caught up (the app was closed when the time came).

use crate::{execute, AppState, CmdResult, ExecArgs};
use chrono::{DateTime, Local, Utc};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use stillroom_core::{Run, RunTrigger, Schedule};
use tauri::{AppHandle, Emitter, Manager};

/// How often the clock is checked.
pub const TICK: Duration = Duration::from_secs(30);
/// How long after launch the catch-up pass runs.
pub const CATCH_UP_DELAY: Duration = Duration::from_secs(10);

struct Live {
    running: bool,
    /// When it was counted in during this session (added or resumed), so moments that went by
    /// while it was paused are not owed. None for one counted in at launch: the stored last
    /// fire decides, which is what lets a morning the app was closed for be caught up.
    since: Option<DateTime<Utc>>,
}

/// The enabled schedules, by id, and the ticker that checks them.
#[derive(Default)]
pub struct ScheduleRuntime {
    live: Mutex<HashMap<String, Live>>,
    ticker: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

/// A schedule as the UI sees it: what is saved plus when it next fires.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleView {
    #[serde(flatten)]
    pub schedule: Schedule,
    /// The next moment the clock fires it (UTC instant of a local time of day). Computed for
    /// a paused schedule too, so the UI can say when it would run.
    pub next_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ScheduleRunEvent {
    schedule_id: String,
    recipe_id: String,
    phase: &'static str,
    run_id: Option<String>,
    status: Option<String>,
    error: Option<String>,
}

impl ScheduleRuntime {
    pub fn view(&self, schedule: Schedule) -> ScheduleView {
        let next_at = schedule.next_at(Local::now()).map(|t| t.with_timezone(&Utc));
        ScheduleView { schedule, next_at }
    }

    /// Count an enabled schedule in from now (added or resumed). Idempotent: one already counted
    /// in, running or not, is left as it is.
    pub fn start(&self, schedule: &Schedule) {
        self.insert(&schedule.id, Some(Utc::now()));
    }

    fn insert(&self, id: &str, since: Option<DateTime<Utc>>) {
        if let Ok(mut map) = self.live.lock() {
            map.entry(id.to_string()).or_insert(Live { running: false, since });
        }
    }

    /// Stop checking a schedule (paused or deleted). A run already going finishes.
    pub fn stop(&self, id: &str) {
        if let Ok(mut map) = self.live.lock() {
            map.remove(id);
        }
    }

    /// Count every enabled schedule in and start the ticker. Called once at launch; calling it
    /// again replaces the ticker.
    pub fn start_all(&self, app: &AppHandle, state: &AppState) {
        let schedules = state.store.lock().ok().and_then(|s| s.list_schedules().ok()).unwrap_or_default();
        for s in schedules.iter().filter(|s| s.enabled) {
            self.insert(&s.id, None);
        }
        let handle = tauri::async_runtime::spawn(tick_loop(app.clone()));
        if let Ok(mut t) = self.ticker.lock() {
            if let Some(old) = t.replace(handle) {
                old.abort();
            }
        }
    }

    /// Take the running flag, or say why not: gone, paused, or already running.
    fn claim(&self, id: &str) -> CmdResult<()> {
        let mut map = self.live.lock().map_err(|e| e.to_string())?;
        let Some(live) = map.get_mut(id) else { return Err("that schedule is paused or gone".into()) };
        if live.running {
            return Err("that schedule is running now".into());
        }
        live.running = true;
        Ok(())
    }

    fn release(&self, id: &str) {
        if let Ok(mut map) = self.live.lock() {
            if let Some(live) = map.get_mut(id) {
                live.running = false;
            }
        }
    }
}

/// Wait for the app to be up, catch up on what was missed while it was closed, then check the
/// clock every `TICK`.
async fn tick_loop(app: AppHandle) {
    tokio::time::sleep(CATCH_UP_DELAY).await;
    for id in due_now(&app) {
        // One after another: the first morning's run finishes before the next starts.
        let _ = fire(&app, &id, true).await;
    }
    loop {
        tokio::time::sleep(TICK).await;
        for id in due_now(&app) {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = fire(&app, &id, false).await;
            });
        }
    }
}

/// Enabled schedules that are counted in, not running, and have a missed moment behind them.
fn due_now(app: &AppHandle) -> Vec<String> {
    let state = app.state::<AppState>();
    let rt = app.state::<ScheduleRuntime>();
    let Some(schedules) = state.store.lock().ok().and_then(|s| s.list_schedules().ok()) else { return Vec::new() };
    let now = Local::now();
    let Ok(live) = rt.live.lock() else { return Vec::new() };
    schedules
        .into_iter()
        .filter(|s| {
            s.enabled
                && live.get(&s.id).is_some_and(|l| {
                    !l.running
                        && match l.since {
                            Some(since) => s.missed_since(since, now),
                            None => s.missed(now),
                        }
                })
        })
        .map(|s| s.id)
        .collect()
}

/// Run a schedule's recipe now (the clock, the launch catch-up, or the user's Run now). Refused
/// while the same schedule is already running; the run gate keeps it to one run at a time
/// overall, so a schedule that comes due during another run waits its turn.
pub async fn fire(app: &AppHandle, id: &str, caught_up: bool) -> CmdResult<Run> {
    let rt = app.state::<ScheduleRuntime>();
    rt.claim(id)?;
    let result = run_once(app, id, caught_up).await;
    rt.release(id);
    result
}

async fn run_once(app: &AppHandle, id: &str, caught_up: bool) -> CmdResult<Run> {
    let state = app.state::<AppState>();
    let schedule = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let schedule = store.get_schedule(id).map_err(|e| e.to_string())?.ok_or("that schedule is gone")?;
        if !schedule.enabled {
            return Err("the schedule is paused; resume it first".into());
        }
        schedule
    };
    let event = |phase: &'static str, run_id: Option<String>, status: Option<String>, error: Option<String>| ScheduleRunEvent {
        schedule_id: schedule.id.clone(),
        recipe_id: schedule.recipe_id.clone(),
        phase,
        run_id,
        status,
        error,
    };
    // "started" is said only once the run has its turn at the gate and an id, as for watches.
    let on_started: Box<dyn Fn(&str) + Send + Sync> = {
        let app = app.clone();
        let started = event("started", None, None, None);
        Box::new(move |run_id: &str| {
            let _ = app.emit("schedule:run", ScheduleRunEvent { run_id: Some(run_id.to_string()), ..started.clone() });
        })
    };
    let trigger = RunTrigger::Schedule { schedule_id: schedule.id.clone(), time: schedule.time.clone(), caught_up };
    let result = execute(
        app,
        &state,
        ExecArgs {
            recipe_id: schedule.recipe_id.clone(),
            inputs: schedule.inputs.clone(),
            pins: schedule.pins.clone(),
            reuse: true,
            edits: Default::default(),
            resume_from: None,
            trigger: Some(trigger),
            rebuild: Vec::new(),
            on_started: Some(on_started),
        },
    )
    .await;
    // The attempt is the fire, whether or not it ran: a refusal (consent, a missing recipe) is
    // reported once per scheduled moment, not every half minute.
    if let Ok(store) = state.store.lock() {
        if let Ok(Some(mut s)) = store.get_schedule(id) {
            s.last_fired_at = Some(Utc::now());
            if let Ok(run) = &result {
                s.last_run_id = Some(run.id.clone());
            }
            let _ = store.save_schedule(&s);
        }
    }
    match &result {
        Ok(run) => {
            let status = serde_json::to_value(run.status).ok().and_then(|v| v.as_str().map(String::from));
            let _ = app.emit("schedule:run", event("finished", Some(run.id.clone()), status, run.error.clone()));
        }
        Err(e) => {
            let _ = app.emit("schedule:run", event("failed", None, None, Some(e.clone())));
        }
    }
    let _ = app.emit("schedule:changed", ());
    result
}
