//! SQLite persistence for runs, folder watches, the shelf, and permission grants. One file, no server.
//!
//! Runs are stored as a JSON document plus a few indexed columns for listing.
//! Proper per-step and per-artifact tables arrive with Flow Studio.

use crate::cache::{CachedStep, StepCache};
use crate::consent::{Grant, GrantKind, Granted};
use crate::recipe::{Refresh, Stage};
use crate::trace::{Run, RunStatus, RunSummary};
use crate::watch::Watch;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::BTreeMap;
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
pub const STORE_SCHEMA_VERSION: i64 = 3;

/// The one recipe a fresh shelf starts with (the example, *Describe a document*).
pub const EXAMPLE_RECIPE_ID: &str = "describe-document";

/// Where a recipe sits for the Library. The shelf is what the Recipes zone shows; the archive is
/// what was taken off the shelf without being deleted (folded under Archive, with Restore); the
/// catalog is a built-in that is neither. A recipe of the user's own is never in the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShelfState {
    Shelf,
    Archive,
    #[default]
    Catalog,
}

impl ShelfState {
    pub fn as_str(self) -> &'static str {
        match self {
            ShelfState::Shelf => "shelf",
            ShelfState::Archive => "archive",
            ShelfState::Catalog => "catalog",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "shelf" => Some(ShelfState::Shelf),
            "archive" => Some(ShelfState::Archive),
            "catalog" => Some(ShelfState::Catalog),
            _ => None,
        }
    }
}

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
                // Version 2: the shelf. Seeded so an existing user keeps in front what they have
                // used, and a fresh install sees one recipe.
                1 => {
                    self.create_v2()?;
                    self.seed_shelf()?;
                }
                // Version 3: permission grants, one row per literal entry the user said yes to.
                2 => self.create_v3()?,
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

    fn create_v2(&self) -> Result<(), StoreError> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS shelf (
                recipe_id  TEXT PRIMARY KEY,
                state      TEXT NOT NULL,
                changed_at TEXT NOT NULL
            );
            "#,
        )?;
        Ok(())
    }

    fn create_v3(&self) -> Result<(), StoreError> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS grants (
                recipe_id  TEXT NOT NULL,
                kind       TEXT NOT NULL,
                entry      TEXT NOT NULL,
                granted_at TEXT NOT NULL,
                PRIMARY KEY (recipe_id, kind, entry)
            );
            "#,
        )?;
        Ok(())
    }

    /// Put the example recipe on the shelf, plus every recipe that has at least one run on
    /// record: whoever has run a built-in keeps it in front of them, while a fresh database has
    /// no runs and ends up with the example alone. The store does not know which ids are
    /// built-in; a row for one of the user's own recipes says what its default already says.
    fn seed_shelf(&self) -> Result<(), StoreError> {
        let now = chrono::Utc::now().to_rfc3339();
        let mut ids = vec![EXAMPLE_RECIPE_ID.to_string()];
        let mut stmt = self.conn.prepare("SELECT DISTINCT recipe_id FROM runs")?;
        for row in stmt.query_map([], |r| r.get::<_, String>(0))? {
            ids.push(row?);
        }
        for id in ids {
            self.conn.execute(
                "INSERT INTO shelf (recipe_id, state, changed_at) VALUES (?1, 'shelf', ?2) ON CONFLICT(recipe_id) DO NOTHING",
                params![id, now],
            )?;
        }
        Ok(())
    }

    /// Where the shelf table says a recipe sits; `None` when it has never been placed, which
    /// means the default for its source (shelf for the user's own, catalog for a built-in).
    pub fn shelf_state(&self, recipe_id: &str) -> Result<Option<ShelfState>, StoreError> {
        let s: Option<String> =
            self.conn.query_row("SELECT state FROM shelf WHERE recipe_id = ?1", params![recipe_id], |r| r.get(0)).optional()?;
        Ok(s.as_deref().and_then(ShelfState::parse))
    }

    pub fn set_shelf(&self, recipe_id: &str, state: ShelfState) -> Result<(), StoreError> {
        self.conn.execute(
            r#"INSERT INTO shelf (recipe_id, state, changed_at) VALUES (?1, ?2, ?3)
               ON CONFLICT(recipe_id) DO UPDATE SET state = excluded.state, changed_at = excluded.changed_at"#,
            params![recipe_id, state.as_str(), chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Forget a recipe's place (on delete). Returns whether there was a row.
    pub fn remove_shelf(&self, recipe_id: &str) -> Result<bool, StoreError> {
        Ok(self.conn.execute("DELETE FROM shelf WHERE recipe_id = ?1", params![recipe_id])? > 0)
    }

    /// Every placed recipe and where it sits, by recipe id.
    pub fn shelf_all(&self) -> Result<BTreeMap<String, ShelfState>, StoreError> {
        let mut stmt = self.conn.prepare("SELECT recipe_id, state FROM shelf")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = BTreeMap::new();
        for row in rows {
            let (id, state) = row?;
            if let Some(state) = ShelfState::parse(&state) {
                out.insert(id, state);
            }
        }
        Ok(out)
    }

    /// The permission entries the user has granted a recipe: kind, entry, and when.
    pub fn grants(&self, recipe_id: &str) -> Result<Vec<Granted>, StoreError> {
        let mut stmt = self.conn.prepare("SELECT kind, entry, granted_at FROM grants WHERE recipe_id = ?1 ORDER BY rowid")?;
        let rows = stmt.query_map(params![recipe_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (kind, entry, at) = row?;
            if let Some(kind) = GrantKind::parse(&kind) {
                out.push((kind, entry, at));
            }
        }
        Ok(out)
    }

    /// Every grant for every recipe, by recipe id.
    pub fn grants_all(&self) -> Result<BTreeMap<String, Vec<Granted>>, StoreError> {
        let mut stmt = self.conn.prepare("SELECT recipe_id, kind, entry, granted_at FROM grants ORDER BY rowid")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?;
        let mut out: BTreeMap<String, Vec<Granted>> = BTreeMap::new();
        for row in rows {
            let (id, kind, entry, at) = row?;
            if let Some(kind) = GrantKind::parse(&kind) {
                out.entry(id).or_default().push((kind, entry, at));
            }
        }
        Ok(out)
    }

    /// Record that the user granted these entries to a recipe. Granting again changes nothing.
    pub fn grant(&self, recipe_id: &str, grants: &[Grant]) -> Result<(), StoreError> {
        let now = chrono::Utc::now().to_rfc3339();
        for g in grants {
            self.conn.execute(
                "INSERT INTO grants (recipe_id, kind, entry, granted_at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(recipe_id, kind, entry) DO NOTHING",
                params![recipe_id, g.kind.as_str(), g.entry, now],
            )?;
        }
        Ok(())
    }

    /// Forget every grant for a recipe. Returns how many there were.
    pub fn revoke_grants(&self, recipe_id: &str) -> Result<usize, StoreError> {
        Ok(self.conn.execute("DELETE FROM grants WHERE recipe_id = ?1", params![recipe_id])?)
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

    fn bare_run(id: &str, recipe_id: &str) -> Run {
        Run {
            id: id.into(),
            recipe_id: recipe_id.into(),
            recipe_name: recipe_id.to_uppercase(),
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
        }
    }

    #[test]
    fn a_fresh_database_has_the_example_alone_on_the_shelf() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.schema_version().unwrap(), STORE_SCHEMA_VERSION);
        let all = store.shelf_all().unwrap();
        assert_eq!(all, BTreeMap::from([(EXAMPLE_RECIPE_ID.to_string(), ShelfState::Shelf)]));
        assert_eq!(store.shelf_state("learning-kit").unwrap(), None, "never placed: the caller's default applies");
    }

    #[test]
    fn climbing_from_version_one_shelves_what_has_been_run() {
        let path = std::env::temp_dir().join(format!("stillroom-store-shelf-{}.db", uuid::Uuid::now_v7()));
        {
            // A version-1 store (phase 1) with runs of two recipes, one of them twice.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("PRAGMA user_version = 1;").unwrap();
            let v1 = Store { conn };
            v1.create_v1().unwrap();
            v1.save_run(&bare_run("r1", "learning-kit")).unwrap();
            v1.save_run(&bare_run("r2", "learning-kit")).unwrap();
            v1.save_run(&bare_run("r3", "my-own")).unwrap();
            assert_eq!(v1.schema_version().unwrap(), 1);
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), STORE_SCHEMA_VERSION);
        let all = store.shelf_all().unwrap();
        assert_eq!(
            all,
            BTreeMap::from([
                (EXAMPLE_RECIPE_ID.to_string(), ShelfState::Shelf),
                ("learning-kit".to_string(), ShelfState::Shelf),
                ("my-own".to_string(), ShelfState::Shelf),
            ])
        );
        assert_eq!(store.list_runs(10).unwrap().len(), 3, "the runs are still there");
        drop(store);
        // Opening again is a no-op: the seed does not run twice and does not undo a change.
        let store = Store::open(&path).unwrap();
        store.set_shelf("learning-kit", ShelfState::Archive).unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.shelf_state("learning-kit").unwrap(), Some(ShelfState::Archive));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn shelf_rows_are_set_replaced_and_removed() {
        let store = Store::open_in_memory().unwrap();
        store.set_shelf("research-brief", ShelfState::Shelf).unwrap();
        assert_eq!(store.shelf_state("research-brief").unwrap(), Some(ShelfState::Shelf));
        store.set_shelf("research-brief", ShelfState::Archive).unwrap();
        assert_eq!(store.shelf_state("research-brief").unwrap(), Some(ShelfState::Archive));
        store.set_shelf("research-brief", ShelfState::Catalog).unwrap();
        assert_eq!(store.shelf_state("research-brief").unwrap(), Some(ShelfState::Catalog));
        assert!(store.remove_shelf("research-brief").unwrap());
        assert!(!store.remove_shelf("research-brief").unwrap());
        assert_eq!(store.shelf_state("research-brief").unwrap(), None);
        assert_eq!(serde_json::to_value(ShelfState::Archive).unwrap(), serde_json::json!("archive"));
        assert_eq!(serde_json::from_value::<ShelfState>(serde_json::json!("catalog")).unwrap(), ShelfState::Catalog);
    }

    #[test]
    fn grants_are_recorded_once_listed_and_revoked() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.grants("mine").unwrap().is_empty());
        store.grant("mine", &[Grant::new(GrantKind::Read, "~/Documents"), Grant::new(GrantKind::Network, "example.com")]).unwrap();
        store.grant("mine", &[Grant::new(GrantKind::Read, "~/Documents")]).unwrap();
        store.grant("other", &[Grant::new(GrantKind::Shell, "true")]).unwrap();
        let mine = store.grants("mine").unwrap();
        assert_eq!(mine.iter().map(|(k, e, _)| (*k, e.as_str())).collect::<Vec<_>>(), vec![(GrantKind::Read, "~/Documents"), (GrantKind::Network, "example.com")]);
        assert!(chrono::DateTime::parse_from_rfc3339(&mine[0].2).is_ok(), "granted_at is a timestamp: {}", mine[0].2);
        let all = store.grants_all().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!((all["other"][0].0, all["other"][0].1.as_str()), (GrantKind::Shell, "true"));
        assert_eq!(store.revoke_grants("mine").unwrap(), 2);
        assert!(store.grants("mine").unwrap().is_empty());
        assert_eq!(store.grants("other").unwrap().len(), 1, "another recipe's grants stay");
        assert_eq!(store.revoke_grants("mine").unwrap(), 0);
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
