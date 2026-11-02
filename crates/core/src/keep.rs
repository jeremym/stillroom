//! Run keeps: what a stage holds on to between runs. Records (JSON by id), a manifest (what
//! built it, when, and the step outputs so a fresh stage can be skipped), and a journal (what
//! each run changed). One SQLite file per stage under the recipe's data folder. Mutable and
//! recorded: a stage may read and change its own keep; every change is counted and journaled.

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum KeepError {
    #[error("keep store: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("keep store: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("the keep is locked")]
    Locked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeepRecord {
    pub id: String,
    pub data: Value,
    pub run_id: String,
    pub written_at: DateTime<Utc>,
}

/// What built the keep last, so the engine can tell fresh from stale and skip a fresh stage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KeepManifest {
    pub source_fingerprint: Option<String>,
    pub built_at: Option<DateTime<Utc>>,
    pub run_id: Option<String>,
    /// Outputs of the stage's steps when it last ran, by step id.
    pub outputs: BTreeMap<String, Value>,
    /// The resolved inputs of those steps, by step id, so an edit made to a kept step can be
    /// pinned to what the step saw when it ran.
    pub inputs: BTreeMap<String, Value>,
    /// The model behind each slot the stage uses, as resolved when it was built. A different
    /// model makes the stage stale even when its sources did not change.
    pub models: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEntry {
    pub run_id: String,
    pub at: DateTime<Utc>,
    pub added: u64,
    pub replaced: u64,
    pub removed: u64,
    /// Why the stage ran, in words: "first build", "sources changed", "runs every time", "rebuilt".
    pub reason: String,
}

/// Counts of changes made during the current run, journaled when the stage finishes.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tally {
    pub added: u64,
    pub replaced: u64,
    pub removed: u64,
}

/// What the UI shows about a keep without opening its records.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeepSummary {
    pub records: u64,
    pub built_at: Option<DateTime<Utc>>,
    pub last: Option<JournalEntry>,
}

pub struct Keep {
    conn: Mutex<Connection>,
    tally: Mutex<Tally>,
    pub dir: PathBuf,
}

/// The keep's on-disk layout version (`PRAGMA user_version`); see `Keep::open` for the ladder.
pub const KEEP_SCHEMA_VERSION: i64 = 1;

impl std::fmt::Debug for Keep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keep").field("dir", &self.dir).finish()
    }
}

impl Keep {
    /// Open (creating) the keep stored in `dir`.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, KeepError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).map_err(|source| KeepError::Io { path: dir.clone(), source })?;
        let conn = Connection::open(dir.join("keep.db"))?;
        let mut v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        while v < KEEP_SCHEMA_VERSION {
            match v {
                // Version 1: the layout from before versions were counted; `IF NOT EXISTS` lets
                // an older file climb without change.
                0 => Self::create_v1(&conn)?,
                _ => break,
            }
            v += 1;
            conn.execute_batch(&format!("PRAGMA user_version = {v};"))?;
        }
        Ok(Self { conn: Mutex::new(conn), tally: Mutex::new(Tally::default()), dir })
    }

    fn create_v1(conn: &Connection) -> Result<(), KeepError> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS records (
                id         TEXT PRIMARY KEY,
                data       TEXT NOT NULL,
                run_id     TEXT NOT NULL,
                written_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS vectors (
                id    TEXT PRIMARY KEY,
                model TEXT NOT NULL,
                dim   INTEGER NOT NULL,
                vec   BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS journal (
                seq      INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id   TEXT NOT NULL,
                at       TEXT NOT NULL,
                added    INTEGER NOT NULL,
                replaced INTEGER NOT NULL,
                removed  INTEGER NOT NULL,
                reason   TEXT NOT NULL
            );
            "#,
        )?;
        Ok(())
    }

    /// The layout version the open file is at.
    pub fn schema_version(&self) -> Result<i64, KeepError> {
        Ok(self.conn()?.query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    /// Which fingerprint scheme (`format` in `meta`) the manifest was written under. None for a
    /// keep built before the scheme was marked.
    pub fn format(&self) -> Result<Option<u32>, KeepError> {
        let conn = self.conn()?;
        let v: Option<String> = conn.query_row("SELECT value FROM meta WHERE key = 'format'", [], |r| r.get(0)).optional()?;
        Ok(v.and_then(|s| s.parse().ok()))
    }

    pub fn set_format(&self, format: u32) -> Result<(), KeepError> {
        self.conn()?.execute("INSERT INTO meta (key, value) VALUES ('format', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![format.to_string()])?;
        Ok(())
    }

    fn conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, KeepError> {
        self.conn.lock().map_err(|_| KeepError::Locked)
    }

    /// Write or replace one record. Returns true when a record with that id already existed.
    pub fn put(&self, id: &str, data: &Value, run_id: &str) -> Result<bool, KeepError> {
        let conn = self.conn()?;
        let existed: bool = conn.query_row("SELECT COUNT(*) FROM records WHERE id = ?1", params![id], |r| r.get::<_, i64>(0))? > 0;
        conn.execute(
            "INSERT INTO records (id, data, run_id, written_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET data = excluded.data, run_id = excluded.run_id, written_at = excluded.written_at",
            params![id, serde_json::to_string(data)?, run_id, Utc::now().to_rfc3339()],
        )?;
        drop(conn);
        if let Ok(mut t) = self.tally.lock() {
            if existed { t.replaced += 1 } else { t.added += 1 }
        }
        Ok(existed)
    }

    pub fn get(&self, id: &str) -> Result<Option<KeepRecord>, KeepError> {
        let conn = self.conn()?;
        conn.query_row("SELECT id, data, run_id, written_at FROM records WHERE id = ?1", params![id], row)
            .optional()
            .map_err(Into::into)
    }

    /// Records, newest first by default. `filter` keeps records whose top-level fields equal the
    /// given values. Done in Rust after the fetch; fine for the sizes a keep holds.
    pub fn query(&self, filter: Option<&serde_json::Map<String, Value>>, newest_first: bool, limit: usize) -> Result<Vec<KeepRecord>, KeepError> {
        let conn = self.conn()?;
        let sql = if newest_first { "SELECT id, data, run_id, written_at FROM records ORDER BY written_at DESC, rowid DESC" } else { "SELECT id, data, run_id, written_at FROM records ORDER BY written_at ASC, rowid ASC" };
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], row)?;
        let mut out = Vec::new();
        for r in rows {
            let r = r?;
            let keep = match filter {
                Some(f) => f.iter().all(|(k, v)| r.data.get(k) == Some(v)),
                None => true,
            };
            if keep {
                out.push(r);
                if out.len() >= limit {
                    break;
                }
            }
        }
        Ok(out)
    }

    /// Delete one record and its vector. Returns true when there was one.
    pub fn remove(&self, id: &str) -> Result<bool, KeepError> {
        let n = {
            let conn = self.conn()?;
            let n = conn.execute("DELETE FROM records WHERE id = ?1", params![id])?;
            conn.execute("DELETE FROM vectors WHERE id = ?1", params![id])?;
            n
        };
        self.tally_removed(n as u64);
        Ok(n > 0)
    }

    fn tally_removed(&self, n: u64) {
        if n > 0
            && let Ok(mut t) = self.tally.lock()
        {
            t.removed += n;
        }
    }

    /// Delete every record (and its vector) that `stale` picks out; returns their ids.
    fn forget_where(&self, mut stale: impl FnMut(&KeepRecord) -> bool) -> Result<Vec<String>, KeepError> {
        let conn = self.conn()?;
        let gone: Vec<String> = {
            let mut stmt = conn.prepare("SELECT id, data, run_id, written_at FROM records")?;
            let rows = stmt.query_map([], row)?;
            rows.filter_map(Result::ok).filter(|r| stale(r)).map(|r| r.id).collect()
        };
        for id in &gone {
            conn.execute("DELETE FROM records WHERE id = ?1", params![id])?;
            conn.execute("DELETE FROM vectors WHERE id = ?1", params![id])?;
        }
        Ok(gone)
    }

    /// After indexing a file: forget its passages that this indexing did not write or confirm
    /// (the file got shorter, or a passage emptied). Counted as removed in the tally.
    pub fn forget_other_passages(&self, source: &str, current: &HashSet<String>) -> Result<u64, KeepError> {
        let gone = self.forget_where(|r| r.data.get("source").and_then(Value::as_str) == Some(source) && !current.contains(&r.id))?;
        self.tally_removed(gone.len() as u64);
        Ok(gone.len() as u64)
    }

    /// Forget every passage whose file is not in `sources` any more. Records without a `source`
    /// field are not passages and stay. Returns (passages, files). Not counted in the tally: the
    /// caller journals it in its own words.
    pub fn forget_sources_except(&self, sources: &HashSet<String>) -> Result<(u64, u64), KeepError> {
        let mut files = HashSet::new();
        let gone = self.forget_where(|r| match r.data.get("source").and_then(Value::as_str) {
            Some(s) if !sources.contains(s) => {
                files.insert(s.to_string());
                true
            }
            _ => false,
        })?;
        Ok((gone.len() as u64, files.len() as u64))
    }

    /// Whether a record with this id holds exactly this data (so indexing can skip it).
    pub fn has_same(&self, id: &str, data: &Value) -> Result<bool, KeepError> {
        Ok(self.get(id)?.is_some_and(|r| &r.data == data))
    }

    /// Whether a record with this id holds exactly this data and a vector made by `model`.
    /// Indexing skips a passage only then: a vector from another model cannot be compared with
    /// questions embedded by this one.
    pub fn has_same_embedding(&self, id: &str, data: &Value, model: &str) -> Result<bool, KeepError> {
        Ok(self.has_same(id, data)? && self.vector_model(id)?.as_deref() == Some(model))
    }

    /// The model that made a record's vector, if it has one.
    pub fn vector_model(&self, id: &str) -> Result<Option<String>, KeepError> {
        let conn = self.conn()?;
        conn.query_row("SELECT model FROM vectors WHERE id = ?1", params![id], |r| r.get(0)).optional().map_err(Into::into)
    }

    /// Store the vector for a record, replacing any earlier one.
    pub fn put_vector(&self, id: &str, model: &str, vec: &[f32]) -> Result<(), KeepError> {
        let bytes: Vec<u8> = vec.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.conn()?.execute(
            "INSERT INTO vectors (id, model, dim, vec) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(id) DO UPDATE SET model = excluded.model, dim = excluded.dim, vec = excluded.vec",
            params![id, model, vec.len() as i64, bytes],
        )?;
        Ok(())
    }

    /// The records nearest to `query` by cosine similarity, best first, with their scores.
    /// A brute-force scan: fine for the tens of thousands of passages a keep holds.
    pub fn search(&self, query: &[f32], limit: usize) -> Result<Vec<(KeepRecord, f32)>, KeepError> {
        let qn = query.iter().map(|x| x * x).sum::<f32>().sqrt();
        if qn == 0.0 {
            return Ok(Vec::new());
        }
        let mut scored: Vec<(String, f32)> = {
            let conn = self.conn()?;
            let mut stmt = conn.prepare("SELECT id, dim, vec FROM vectors")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize, r.get::<_, Vec<u8>>(2)?)))?;
            let mut out = Vec::new();
            for row in rows {
                let (id, dim, bytes) = row?;
                if dim != query.len() {
                    continue;
                }
                let mut dot = 0f32;
                let mut n = 0f32;
                for (i, chunk) in bytes.chunks_exact(4).enumerate() {
                    let x = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    dot += x * query[i];
                    n += x * x;
                }
                let n = n.sqrt();
                if n > 0.0 {
                    out.push((id, dot / (n * qn)));
                }
            }
            out
        };
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let mut out = Vec::new();
        for (id, score) in scored.into_iter().take(limit) {
            if let Some(rec) = self.get(&id)? {
                out.push((rec, score));
            }
        }
        Ok(out)
    }

    pub fn count(&self) -> Result<u64, KeepError> {
        Ok(self.conn()?.query_row("SELECT COUNT(*) FROM records", [], |r| r.get::<_, i64>(0))? as u64)
    }

    pub fn manifest(&self) -> Result<KeepManifest, KeepError> {
        let conn = self.conn()?;
        let json: Option<String> = conn.query_row("SELECT value FROM meta WHERE key = 'manifest'", [], |r| r.get(0)).optional()?;
        Ok(json.map(|j| serde_json::from_str(&j)).transpose()?.unwrap_or_default())
    }

    pub fn set_manifest(&self, m: &KeepManifest) -> Result<(), KeepError> {
        self.conn()?.execute(
            "INSERT INTO meta (key, value) VALUES ('manifest', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![serde_json::to_string(m)?],
        )?;
        Ok(())
    }

    /// The changes counted since the keep was opened or the tally last taken.
    pub fn take_tally(&self) -> Tally {
        self.tally.lock().map(|mut t| std::mem::take(&mut *t)).unwrap_or_default()
    }

    pub fn append_journal(&self, e: &JournalEntry) -> Result<(), KeepError> {
        self.conn()?.execute(
            "INSERT INTO journal (run_id, at, added, replaced, removed, reason) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![e.run_id, e.at.to_rfc3339(), e.added as i64, e.replaced as i64, e.removed as i64, e.reason],
        )?;
        Ok(())
    }

    /// Journal entries, newest first.
    pub fn journal(&self, limit: usize) -> Result<Vec<JournalEntry>, KeepError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT run_id, at, added, replaced, removed, reason FROM journal ORDER BY seq DESC LIMIT ?1")?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok(JournalEntry {
                run_id: r.get(0)?,
                at: DateTime::parse_from_rfc3339(&r.get::<_, String>(1)?).map(|d| d.with_timezone(&Utc)).unwrap_or_else(|_| Utc::now()),
                added: r.get::<_, i64>(2)? as u64,
                replaced: r.get::<_, i64>(3)? as u64,
                removed: r.get::<_, i64>(4)? as u64,
                reason: r.get(5)?,
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    pub fn summary(&self) -> Result<KeepSummary, KeepError> {
        Ok(KeepSummary { records: self.count()?, built_at: self.manifest()?.built_at, last: self.journal(1)?.into_iter().next() })
    }

    /// Empty the keep (records and manifest), journaling it. Files the stage wrote stay until
    /// the stage runs again and overwrites them.
    pub fn clear(&self, run_id: &str) -> Result<(), KeepError> {
        let removed = self.count()?;
        {
            let conn = self.conn()?;
            conn.execute_batch("DELETE FROM records; DELETE FROM meta; DELETE FROM vectors;")?;
        }
        self.append_journal(&JournalEntry { run_id: run_id.into(), at: Utc::now(), added: 0, replaced: 0, removed, reason: "rebuilt".into() })
    }
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<KeepRecord> {
    let data: String = r.get(1)?;
    let at: String = r.get(3)?;
    Ok(KeepRecord {
        id: r.get(0)?,
        data: serde_json::from_str(&data).unwrap_or(Value::Null),
        run_id: r.get(2)?,
        written_at: DateTime::parse_from_rfc3339(&at).map(|d| d.with_timezone(&Utc)).unwrap_or_else(|_| Utc::now()),
    })
}

/// Where a recipe's keeps live: `<base>/<recipe id>/<stage id>/`.
pub fn keep_dir(base: &Path, recipe_id: &str, stage_id: &str) -> PathBuf {
    base.join(recipe_id).join(stage_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn records_manifest_journal_and_clear() {
        let dir = std::env::temp_dir().join(format!("stillroom-keep-{}", uuid::Uuid::now_v7()));
        let k = Keep::open(&dir).unwrap();
        assert_eq!(k.count().unwrap(), 0);
        assert_eq!(k.schema_version().unwrap(), KEEP_SCHEMA_VERSION);
        assert_eq!(k.format().unwrap(), None, "unmarked until a stage is built");
        k.set_format(1).unwrap();
        assert_eq!(k.format().unwrap(), Some(1));
        assert!(!k.put("a", &json!({ "title": "A", "kind": "news" }), "run1").unwrap());
        assert!(!k.put("b", &json!({ "title": "B", "kind": "note" }), "run1").unwrap());
        assert!(k.put("a", &json!({ "title": "A2", "kind": "news" }), "run1").unwrap());
        assert!(k.remove("b").unwrap());
        assert!(!k.remove("zzz").unwrap());
        let t = k.take_tally();
        assert_eq!((t.added, t.replaced, t.removed), (2, 1, 1));
        assert_eq!(k.take_tally().added, 0);
        assert_eq!(k.get("a").unwrap().unwrap().data["title"], "A2");
        k.put("c", &json!({ "kind": "news" }), "run2").unwrap();
        let mut f = serde_json::Map::new();
        f.insert("kind".into(), json!("news"));
        let news = k.query(Some(&f), true, 10).unwrap();
        assert_eq!(news.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["c", "a"]);
        assert_eq!(k.query(None, false, 1).unwrap()[0].id, "a");

        assert!(k.manifest().unwrap().built_at.is_none());
        let mut m = KeepManifest { source_fingerprint: Some("fp".into()), built_at: Some(Utc::now()), run_id: Some("run2".into()), ..Default::default() };
        m.outputs.insert("index".into(), json!({ "n": 2 }));
        k.set_manifest(&m).unwrap();
        assert_eq!(k.manifest().unwrap().source_fingerprint.as_deref(), Some("fp"));
        k.append_journal(&JournalEntry { run_id: "run2".into(), at: Utc::now(), added: 1, replaced: 0, removed: 0, reason: "sources changed".into() }).unwrap();
        let s = k.summary().unwrap();
        assert_eq!(s.records, 2);
        assert_eq!(s.last.unwrap().reason, "sources changed");

        k.clear("run3").unwrap();
        assert_eq!(k.count().unwrap(), 0);
        assert!(k.manifest().unwrap().source_fingerprint.is_none());
        assert_eq!(k.format().unwrap(), None, "a rebuild clears the mark with the manifest");
        let j = k.journal(10).unwrap();
        assert_eq!(j[0].reason, "rebuilt");
        assert_eq!(j[0].removed, 2);
        assert_eq!(keep_dir(Path::new("/base"), "r", "s"), PathBuf::from("/base/r/s"));

        // Vectors: nearest by cosine, best first; a mismatched dimension is ignored.
        k.put("p1", &json!({ "text": "apples" }), "r").unwrap();
        k.put("p2", &json!({ "text": "oranges" }), "r").unwrap();
        k.put("p3", &json!({ "text": "cars" }), "r").unwrap();
        k.put_vector("p1", "m", &[1.0, 0.0, 0.0]).unwrap();
        k.put_vector("p2", "m", &[0.9, 0.1, 0.0]).unwrap();
        k.put_vector("p3", "m", &[0.0, 0.0, 1.0]).unwrap();
        k.put_vector("odd", "m", &[1.0, 1.0]).unwrap();
        let hits = k.search(&[1.0, 0.05, 0.0], 2).unwrap();
        assert_eq!(hits.iter().map(|(r, _)| r.id.as_str()).collect::<Vec<_>>(), vec!["p1", "p2"]);
        assert!(hits[0].1 > hits[1].1);
        assert!(k.has_same("p1", &json!({ "text": "apples" })).unwrap());
        assert!(!k.has_same("p1", &json!({ "text": "pears" })).unwrap());
        // Same text but a vector from another model is not the same embedding.
        assert!(k.has_same_embedding("p1", &json!({ "text": "apples" }), "m").unwrap());
        assert!(!k.has_same_embedding("p1", &json!({ "text": "apples" }), "other-model").unwrap());
        assert!(!k.has_same_embedding("nope", &json!({ "text": "apples" }), "m").unwrap());

        // Removing a record takes its vector with it.
        k.take_tally();
        assert_eq!(k.vector_model("p3").unwrap().as_deref(), Some("m"));
        assert!(k.remove("p3").unwrap());
        assert!(k.vector_model("p3").unwrap().is_none());

        // Passages of a file that got shorter, then of files no longer there, are forgotten;
        // records that are not passages (no source) stay.
        for (id, src, i) in [("f.md#0", "f.md", 0), ("f.md#1", "f.md", 1), ("f.md#2", "f.md", 2), ("g.md#0", "g.md", 0)] {
            k.put(id, &json!({ "text": id, "source": src, "index": i }), "r").unwrap();
            k.put_vector(id, "m", &[1.0, 0.0, 0.0]).unwrap();
        }
        let current: HashSet<String> = ["f.md#0".to_string()].into_iter().collect();
        assert_eq!(k.forget_other_passages("f.md", &current).unwrap(), 2);
        assert!(k.get("f.md#0").unwrap().is_some());
        assert!(k.get("f.md#1").unwrap().is_none());
        assert!(k.vector_model("f.md#2").unwrap().is_none(), "the vector went with the record");
        assert_eq!(k.take_tally().removed, 3, "one removed and two forgotten passages are counted");
        let sources: HashSet<String> = ["f.md".to_string()].into_iter().collect();
        assert_eq!(k.forget_sources_except(&sources).unwrap(), (1, 1), "g.md's one passage, one file");
        assert!(k.get("g.md#0").unwrap().is_none());
        assert!(k.get("p1").unwrap().is_some(), "not a passage, so not forgotten");
        assert_eq!(k.take_tally().removed, 0, "forgetting files is journaled by the caller, not tallied");
        let _ = std::fs::remove_dir_all(dir);
    }
}
