//! SQLite persistence for runs. One file, no server.
//!
//! Runs are stored as a JSON document plus a few indexed columns for listing.
//! Proper per-step and per-artifact tables arrive with Flow Studio.

use crate::trace::{Run, RunStatus, RunSummary};
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
        self.conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
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

    pub fn count_by_status(&self, status: RunStatus) -> Result<i64, StoreError> {
        let s = serde_json::to_value(status)?.as_str().unwrap_or("").to_string();
        Ok(self.conn.query_row("SELECT COUNT(*) FROM runs WHERE status = ?1", params![s], |r| r.get(0))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::BTreeMap;

    #[test]
    fn round_trips_a_run() {
        let store = Store::open_in_memory().unwrap();
        let run = Run {
            id: "r1".into(),
            recipe_id: "x".into(),
            recipe_name: "X".into(),
            recipe_version: None,
            status: RunStatus::Succeeded,
            started_at: Utc::now(),
            finished_at: Some(Utc::now()),
            inputs: BTreeMap::new(),
            resolutions: serde_json::Value::Null,
            steps: vec![],
            outputs: BTreeMap::new(),
            error: None,
        };
        store.save_run(&run).unwrap();
        assert_eq!(store.get_run("r1").unwrap().unwrap().recipe_name, "X");
        assert_eq!(store.list_runs(10).unwrap().len(), 1);
        assert_eq!(store.count_by_status(RunStatus::Succeeded).unwrap(), 1);
    }
}
