//! SQLite persistence for runs and folder watches. One file, no server.
//!
//! Runs are stored as a JSON document plus a few indexed columns for listing.
//! Proper per-step and per-artifact tables arrive with Flow Studio.

use crate::cache::{CachedStep, StepCache};
use crate::recipe::{Refresh, Stage};
use crate::trace::{Run, RunStatus, RunSummary};
use crate::watch::Watch;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

pub struct Store {
    conn: Connection,
}

/// The store's on-disk layout version, kept in SQLite's `PRAGMA user_version`. Each step of the
/// ladder in `init` brings a file up one version: one `CREATE`, `ALTER`, or backfill per step.
pub const STORE_SCHEMA_VERSION: i64 = 1;

/// A store behind a mutex can serve as the executor's step cache (`Arc<Mutex<Store>>`
/// coerces to `Arc<dyn StepCache>`).
impl StepCache for std::sync::Mutex<Store> {
    fn get(&self, key: &str) -> Option<CachedStep> {
        self.lock().ok()?.cache_get(key).ok().flatten()
    }
    fn put(&self, key: &str, entry: &CachedStep) {
        if let Ok(s) = self.lock() {
            let _ = s.cache_put(key, entry);
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub model_id: String,
    pub calls: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEstimate {
    pub based_on_runs: usize,
    pub avg_prompt_tokens: u64,
    pub avg_completion_tokens: u64,
    pub avg_duration_ms: u64,
    /// One line per stage that has actually run before: what it took when it did.
    #[serde(default)]
    pub stages: Vec<StageEstimate>,
}

/// What one stage took, averaged over the runs in which it really ran (a stage served from
/// its run keep took no time and is not counted). `once` says the stage is kept and will not
/// run again until its sources change or it is rebuilt.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageEstimate {
    pub id: String,
    pub based_on_runs: usize,
    pub avg_duration_ms: u64,
    pub avg_tokens: u64,
    pub once: bool,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        let s = Self { conn };
        s.init()?;
        Ok(s)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        let s = Self { conn: Connection::open_in_memory()? };
        s.init()?;
        Ok(s)
    }

    fn init(&self) -> Result<(), StoreError> {
        self.conn.execute_batch("PRAGMA journal_mode = WAL;")?;
        let mut v = self.schema_version()?;
        while v < STORE_SCHEMA_VERSION {
            match v {
                // Version 1: the layout as it was before versions were counted. `IF NOT EXISTS`
                // so a file written before the ladder existed (user_version 0, tables present)
                // climbs to 1 without change.
                0 => self.create_v1()?,
                _ => break,
            }
            v += 1;
            self.conn.execute_batch(&format!("PRAGMA user_version = {v};"))?;
        }
        Ok(())
    }

    /// The layout version the open file is at.
    pub fn schema_version(&self) -> Result<i64, StoreError> {
        Ok(self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    fn create_v1(&self) -> Result<(), StoreError> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS runs (
                id            TEXT PRIMARY KEY,
                recipe_id     TEXT NOT NULL,
                recipe_name   TEXT NOT NULL,
                status        TEXT NOT NULL,
                started_at    TEXT NOT NULL,
                finished_at   TEXT,
                json          TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS runs_started_at ON runs(started_at DESC);
            CREATE TABLE IF NOT EXISTS step_cache (
                key        TEXT PRIMARY KEY,
                run_id     TEXT NOT NULL,
                step_id    TEXT NOT NULL,
                model      TEXT,
                output     TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS watches (
                id         TEXT PRIMARY KEY,
                recipe_id  TEXT NOT NULL,
                created_at TEXT NOT NULL,
                json       TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS settings (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
        )?;
        Ok(())
    }

    pub fn save_run(&self, run: &Run) -> Result<(), StoreError> {
        let status = serde_json::to_value(run.status)?.as_str().unwrap_or("unknown").to_string();
        self.conn.execute(
            r#"INSERT INTO runs (id, recipe_id, recipe_name, status, started_at, finished_at, json)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
               ON CONFLICT(id) DO UPDATE SET status = excluded.status, finished_at = excluded.finished_at, json = excluded.json"#,
            params![
                run.id,
                run.recipe_id,
                run.recipe_name,
                status,
                run.started_at.to_rfc3339(),
                run.finished_at.map(|t| t.to_rfc3339()),
                serde_json::to_string(run)?,
            ],
        )?;
        Ok(())
    }

    pub fn get_run(&self, id: &str) -> Result<Option<Run>, StoreError> {
        let json: Option<String> = self
            .conn
            .query_row("SELECT json FROM runs WHERE id = ?1", params![id], |r| r.get(0))
            .optional()?;
        Ok(json.map(|j| serde_json::from_str(&j)).transpose()?)
    }

    pub fn list_runs(&self, limit: usize) -> Result<Vec<RunSummary>, StoreError> {
        let mut stmt = self.conn.prepare("SELECT json FROM runs ORDER BY started_at DESC LIMIT ?1")?;
        let rows = stmt.query_map(params![limit as i64], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            let run: Run = serde_json::from_str(&row?)?;
            out.push(run.summary());
        }
        Ok(out)
    }

    /// Every stored run, newest first. Fine while the store is small; replaced by
    /// per-step tables when it is not.
    pub fn all_runs(&self) -> Result<Vec<Run>, StoreError> {
        let mut stmt = self.conn.prepare("SELECT json FROM runs ORDER BY started_at DESC")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }

    /// Token and time totals per model, across all runs.
    pub fn usage_by_model(&self) -> Result<Vec<ModelUsage>, StoreError> {
        let mut map: std::collections::BTreeMap<String, ModelUsage> = Default::default();
        for run in self.all_runs()? {
            for s in &run.steps {
                let Some(model) = &s.model else { continue };
                let u = map.entry(model.clone()).or_insert_with(|| ModelUsage { model_id: model.clone(), ..Default::default() });
                u.calls += 1;
                u.prompt_tokens += s.prompt_tokens.unwrap_or(0);
                u.completion_tokens += s.completion_tokens.unwrap_or(0);
                u.total_ms += s.duration_ms.unwrap_or(0);
            }
        }
        Ok(map.into_values().collect())
    }

    /// Averages over previous successful runs of a recipe: a pre-run estimate, whole and by stage.
    pub fn estimate_for_recipe(&self, recipe_id: &str, stages: &[Stage]) -> Result<Option<RunEstimate>, StoreError> {
        let runs: Vec<Run> = self
            .all_runs()?
            .into_iter()
            .filter(|r| r.recipe_id == recipe_id && r.status == RunStatus::Succeeded)
            .take(10)
            .collect();
        if runs.is_empty() {
            return Ok(None);
        }
        let n = runs.len() as u64;
        let sum = |f: &dyn Fn(&Run) -> u64| runs.iter().map(f).sum::<u64>() / n;
        let stages = stages
            .iter()
            .filter_map(|st| {
                // A run counts for a stage when at least one of its steps really ran there.
                let took: Vec<(u64, u64)> = runs
                    .iter()
                    .filter_map(|r| {
                        let mine = r.steps.iter().filter(|s| s.parent.is_none() && st.steps.contains(&s.step_id) && s.reused_from.is_none());
                        let (ms, tokens, any) = mine.fold((0, 0, false), |(ms, t, _), s| {
                            (ms + s.duration_ms.unwrap_or(0), t + s.prompt_tokens.unwrap_or(0) + s.completion_tokens.unwrap_or(0), true)
                        });
                        any.then_some((ms, tokens))
                    })
                    .collect();
                if took.is_empty() {
                    return None;
                }
                let k = took.len() as u64;
                Some(StageEstimate {
                    id: st.id.clone(),
                    based_on_runs: took.len(),
                    avg_duration_ms: took.iter().map(|t| t.0).sum::<u64>() / k,
                    avg_tokens: took.iter().map(|t| t.1).sum::<u64>() / k,
                    once: st.keep.as_ref().map(|k| k.refresh != Refresh::Always).unwrap_or(false),
                })
            })
            .collect();
        Ok(Some(RunEstimate {
            based_on_runs: runs.len(),
            avg_prompt_tokens: sum(&|r| r.steps.iter().filter_map(|s| s.prompt_tokens).sum()),
            avg_completion_tokens: sum(&|r| r.steps.iter().filter_map(|s| s.completion_tokens).sum()),
            avg_duration_ms: sum(&|r| r.steps.iter().filter_map(|s| s.duration_ms).sum()),
            stages,
        }))
    }

    pub fn cache_get(&self, key: &str) -> Result<Option<CachedStep>, StoreError> {
        let row: Option<(String, String, Option<String>, String, String)> = self
            .conn
            .query_row(
                "SELECT run_id, step_id, model, output, created_at FROM step_cache WHERE key = ?1",
                params![key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        Ok(match row {
            None => None,
            Some((run_id, step_id, model, output, created_at)) => Some(CachedStep {
                run_id,
                step_id,
                model,
                output: serde_json::from_str(&output)?,
                created_at: created_at.parse().unwrap_or_else(|_| chrono::Utc::now()),
            }),
        })
    }

    pub fn cache_put(&self, key: &str, e: &CachedStep) -> Result<(), StoreError> {
        self.conn.execute(
            r#"INSERT INTO step_cache (key, run_id, step_id, model, output, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
               ON CONFLICT(key) DO UPDATE SET run_id = excluded.run_id, step_id = excluded.step_id, model = excluded.model,
                 output = excluded.output, created_at = excluded.created_at"#,
            params![key, e.run_id, e.step_id, e.model, serde_json::to_string(&e.output)?, e.created_at.to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn cache_clear(&self) -> Result<usize, StoreError> {
        Ok(self.conn.execute("DELETE FROM step_cache", [])?)
    }

    /// The newest run of a recipe that was given exactly these inputs, so a new watch can
    /// continue from work already done.
    pub fn latest_run_with_inputs(&self, recipe_id: &str, inputs: &indexmap::IndexMap<String, serde_json::Value>) -> Result<Option<Run>, StoreError> {
        Ok(self.all_runs()?.into_iter().find(|r| r.recipe_id == recipe_id && r.inputs == *inputs))
    }

    /// The run plus the records of the runs it continues, up to twenty back: what a new
    /// continuation should resume from, so nothing done before a pause is done twice.
    pub fn continuation_base(&self, run_id: &str) -> Result<Option<Run>, StoreError> {
        let Some(mut base) = self.get_run(run_id)? else { return Ok(None) };
        let mut next = base.continues.clone();
        for _ in 0..20 {
            let Some(id) = next.take() else { break };
            let Some(older) = self.get_run(&id)? else { break };
            base.absorb(&older);
            next = older.continues.clone();
        }
        Ok(Some(base))
    }

    /// Every setting, as one JSON object keyed by setting name.
    pub fn settings(&self) -> Result<serde_json::Map<String, serde_json::Value>, StoreError> {
        let mut stmt = self.conn.prepare("SELECT key, value FROM settings ORDER BY key")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = serde_json::Map::new();
        for row in rows {
            let (k, v) = row?;
            out.insert(k, serde_json::from_str(&v)?);
        }
        Ok(out)
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<serde_json::Value>, StoreError> {
        let json: Option<String> =
            self.conn.query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| r.get(0)).optional()?;
        Ok(json.map(|j| serde_json::from_str(&j)).transpose()?)
    }

    /// Set one setting; `null` removes it.
    pub fn set_setting(&self, key: &str, value: &serde_json::Value) -> Result<(), StoreError> {
        if value.is_null() {
            self.conn.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
        } else {
            self.conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, serde_json::to_string(value)?],
            )?;
        }
        Ok(())
    }

    pub fn list_watches(&self) -> Result<Vec<Watch>, StoreError> {
        let mut stmt = self.conn.prepare("SELECT json FROM watches ORDER BY created_at ASC")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }

    pub fn get_watch(&self, id: &str) -> Result<Option<Watch>, StoreError> {
        let json: Option<String> =
            self.conn.query_row("SELECT json FROM watches WHERE id = ?1", params![id], |r| r.get(0)).optional()?;
        Ok(json.map(|j| serde_json::from_str(&j)).transpose()?)
    }

    pub fn save_watch(&self, w: &Watch) -> Result<(), StoreError> {
        self.conn.execute(
            r#"INSERT INTO watches (id, recipe_id, created_at, json) VALUES (?1, ?2, ?3, ?4)
               ON CONFLICT(id) DO UPDATE SET json = excluded.json"#,
            params![w.id, w.recipe_id, w.created_at.to_rfc3339(), serde_json::to_string(w)?],
        )?;
        Ok(())
    }

    pub fn delete_watch(&self, id: &str) -> Result<bool, StoreError> {
        Ok(self.conn.execute("DELETE FROM watches WHERE id = ?1", params![id])? > 0)
    }

    /// When a run is continued (an approval, an edit), watches that would continue from the
    /// old run continue from the new one instead.
    pub fn relink_watches(&self, old_run_id: &str, new_run_id: &str) -> Result<usize, StoreError> {
        let mut n = 0;
        for mut w in self.list_watches()? {
            if w.last_run_id.as_deref() == Some(old_run_id) {
                w.last_run_id = Some(new_run_id.to_string());
                self.save_watch(&w)?;
                n += 1;
            }
        }
        Ok(n)
    }

    pub fn count_by_status(&self, status: RunStatus) -> Result<i64, StoreError> {
        let s = serde_json::to_value(status)?.as_str().unwrap_or("").to_string();
        Ok(self.conn.query_row("SELECT COUNT(*) FROM runs WHERE status = ?1", params![s], |r| r.get(0))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use indexmap::IndexMap;

    #[test]
    fn a_file_from_before_the_ladder_climbs_to_the_current_version() {
        let path = std::env::temp_dir().join(format!("stillroom-store-{}.db", uuid::Uuid::now_v7()));
        {
            // An older Stillroom: the same tables, user_version never set.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL); INSERT INTO settings VALUES ('a', '1');").unwrap();
            assert_eq!(conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), STORE_SCHEMA_VERSION);
        assert_eq!(store.get_setting("a").unwrap(), Some(serde_json::json!(1)), "what was there is kept");
        store.save_watch(&Watch { id: "w".into(), recipe_id: "r".into(), recipe_name: "R".into(), input: "x".into(), folder: "/f".into(), inputs: IndexMap::new(), pins: Default::default(), enabled: true, settle_ms: 2000, created_at: Utc::now(), last_fired_at: None, last_run_id: None }).unwrap();
        assert_eq!(store.list_watches().unwrap().len(), 1, "the tables the ladder adds are usable");
        drop(store);
        assert_eq!(Store::open(&path).unwrap().schema_version().unwrap(), STORE_SCHEMA_VERSION, "opening again is a no-op");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn round_trips_a_run() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.schema_version().unwrap(), STORE_SCHEMA_VERSION);
        let run = Run {
            id: "r1".into(),
            recipe_id: "x".into(),
            recipe_name: "X".into(),
            recipe_version: None,
            status: RunStatus::Succeeded,
            started_at: Utc::now(),
            finished_at: Some(Utc::now()),
            inputs: IndexMap::new(),
            resolutions: serde_json::Value::Null,
            steps: vec![],
            outputs: IndexMap::new(),
            error: None,
            edits: Default::default(),
            trigger: None,
            continues: None,
        };
        store.save_run(&run).unwrap();
        assert_eq!(store.get_run("r1").unwrap().unwrap().recipe_name, "X");
        assert_eq!(store.list_runs(10).unwrap().len(), 1);
        assert_eq!(store.count_by_status(RunStatus::Succeeded).unwrap(), 1);
        assert_eq!(store.latest_run_with_inputs("x", &IndexMap::new()).unwrap().map(|r| r.id), Some("r1".into()));
        assert!(store.latest_run_with_inputs("y", &IndexMap::new()).unwrap().is_none());
    }

    fn step(run_id: &str, id: &str, ms: u64, tokens: u64, reused: Option<&str>) -> crate::trace::StepRecord {
        crate::trace::StepRecord {
            run_id: run_id.into(),
            step_id: id.into(),
            kind: "tool".into(),
            name: None,
            position: 0,
            status: crate::trace::StepStatus::Succeeded,
            started_at: None,
            finished_at: None,
            duration_ms: Some(ms),
            model: None,
            prompt_path: None,
            prompt_tokens: Some(tokens),
            completion_tokens: None,
            inputs: serde_json::Value::Null,
            output: None,
            error: None,
            reused_from: reused.map(String::from),
            edited: false,
            parent: None,
            skip_reason: None,
            tool_calls: vec![],
        }
    }

    #[test]
    fn estimates_by_stage_and_skips_kept_stages() {
        let store = Store::open_in_memory().unwrap();
        let run = |id: &str, steps: Vec<crate::trace::StepRecord>| {
            store
                .save_run(&Run {
                    id: id.into(),
                    recipe_id: "x".into(),
                    recipe_name: "X".into(),
                    recipe_version: None,
                    status: RunStatus::Succeeded,
                    started_at: Utc::now(),
                    finished_at: Some(Utc::now()),
                    inputs: IndexMap::new(),
                    resolutions: serde_json::Value::Null,
                    steps,
                    outputs: IndexMap::new(),
                    error: None,
                    edits: Default::default(),
                    trigger: None,
                    continues: None,
                })
                .unwrap();
        };
        // First run builds the index (4 s) and answers (1 s); second run keeps the index and answers in 3 s.
        run("r1", vec![step("r1", "index", 4000, 100, None), step("r1", "answer", 1000, 50, None)]);
        run("r2", vec![step("r2", "index", 0, 0, Some("keep:build")), step("r2", "answer", 3000, 70, None)]);
        let stages = vec![
            Stage { id: "build".into(), name: "Build".into(), why: None, inputs: vec![], steps: vec!["index".into()], outputs: vec![], keep: Some(crate::recipe::KeepSpec { refresh: Refresh::WhenSourcesChange, id: None }), source: None },
            Stage { id: "ask".into(), name: "Ask".into(), why: None, inputs: vec![], steps: vec!["answer".into()], outputs: vec![], keep: None, source: None },
            Stage { id: "never".into(), name: "Never ran".into(), why: None, inputs: vec![], steps: vec!["nope".into()], outputs: vec![], keep: None, source: None },
        ];
        let e = store.estimate_for_recipe("x", &stages).unwrap().unwrap();
        assert_eq!(e.based_on_runs, 2);
        assert_eq!(e.avg_duration_ms, 4000);
        assert_eq!(e.stages.len(), 2, "{:?}", e.stages);
        assert_eq!((e.stages[0].id.as_str(), e.stages[0].based_on_runs, e.stages[0].avg_duration_ms, e.stages[0].once), ("build", 1, 4000, true));
        assert_eq!((e.stages[1].id.as_str(), e.stages[1].based_on_runs, e.stages[1].avg_duration_ms, e.stages[1].once), ("ask", 2, 2000, false));
        assert!(store.estimate_for_recipe("y", &stages).unwrap().is_none());
    }

    #[test]
    fn round_trips_and_relinks_watches() {
        let store = Store::open_in_memory().unwrap();
        let mut w = Watch {
            id: "w1".into(),
            recipe_id: "x".into(),
            recipe_name: "X".into(),
            input: "inbox".into(),
            folder: "/tmp/in".into(),
            inputs: IndexMap::from([("inbox".to_string(), serde_json::Value::String("/tmp/in".into()))]),
            pins: Default::default(),
            enabled: true,
            settle_ms: 2000,
            created_at: Utc::now(),
            last_fired_at: None,
            last_run_id: Some("r1".into()),
        };
        store.save_watch(&w).unwrap();
        assert_eq!(store.list_watches().unwrap(), vec![w.clone()]);
        assert_eq!(store.relink_watches("r1", "r2").unwrap(), 1);
        assert_eq!(store.get_watch("w1").unwrap().unwrap().last_run_id.as_deref(), Some("r2"));
        w.enabled = false;
        store.save_watch(&w).unwrap();
        assert!(!store.get_watch("w1").unwrap().unwrap().enabled);
        assert!(store.delete_watch("w1").unwrap());
        assert!(store.list_watches().unwrap().is_empty());
    }
}
