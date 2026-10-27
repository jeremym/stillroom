//! Folder watches: a saved binding of a recipe to a folder, so the recipe runs by itself
//! whenever something lands there. Local only: the app watches the folder with the
//! operating system's file notifications while it is open. Nothing listens on the network.
//!
//! A watch holds everything a run needs besides the folder (the other inputs, the model
//! pins) and the id of its last run, so each run continues the one before it: steps that
//! already handled a file are reused from that trace, side effects included.

use crate::recipe::Recipe;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use notify::{RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error("the recipe has no input called {0}")]
    NoSuchInput(String),
    #[error("input {0} is not a folder input, so it cannot be watched")]
    NotADirectoryInput(String),
    #[error("{0} is not a folder that exists")]
    NoSuchFolder(String),
    #[error("the recipe writes into {0}, so watching it would set the recipe off on its own results")]
    WouldLoop(String),
    #[error("could not watch the folder: {0}")]
    Notify(#[from] notify::Error),
}

/// A folder watch as it is saved.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Watch {
    pub id: String,
    pub recipe_id: String,
    pub recipe_name: String,
    /// The recipe's folder input that the watched folder fills.
    pub input: String,
    pub folder: String,
    /// Every input for the run, the watched one included.
    pub inputs: IndexMap<String, Value>,
    pub pins: BTreeMap<String, String>,
    pub enabled: bool,
    /// Quiet time after the last change before the recipe runs, so files being copied in
    /// are complete.
    pub settle_ms: u64,
    pub created_at: DateTime<Utc>,
    pub last_fired_at: Option<DateTime<Utc>>,
    /// The run to continue from next time.
    pub last_run_id: Option<String>,
}

pub const DEFAULT_SETTLE_MS: u64 = 2000;

impl Watch {
    /// A watch on `folder` for `recipe`, after checking that the input is a folder input,
    /// the folder exists, and the recipe does not write into it.
    pub fn new(
        recipe: &Recipe,
        input: &str,
        folder: &str,
        mut inputs: IndexMap<String, Value>,
        pins: BTreeMap<String, String>,
    ) -> Result<Self, WatchError> {
        let spec = recipe.inputs.get(input).ok_or_else(|| WatchError::NoSuchInput(input.to_string()))?;
        if spec.get("type").and_then(Value::as_str) != Some("directory") {
            return Err(WatchError::NotADirectoryInput(input.to_string()));
        }
        let path = Path::new(folder);
        if !path.is_dir() {
            return Err(WatchError::NoSuchFolder(folder.to_string()));
        }
        let folder = path.canonicalize().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| folder.to_string());
        if writes_into(recipe, input, &folder) {
            return Err(WatchError::WouldLoop(folder));
        }
        inputs.insert(input.to_string(), Value::String(folder.clone()));
        Ok(Self {
            id: uuid::Uuid::now_v7().to_string(),
            recipe_id: recipe.id.clone(),
            recipe_name: recipe.name.clone(),
            input: input.to_string(),
            folder,
            inputs,
            pins,
            enabled: true,
            settle_ms: DEFAULT_SETTLE_MS,
            created_at: Utc::now(),
            last_fired_at: None,
            last_run_id: None,
        })
    }

    pub fn settle(&self) -> Duration {
        Duration::from_millis(self.settle_ms.max(100))
    }
}

/// Whether the recipe's write permissions cover the watched input or folder.
fn writes_into(recipe: &Recipe, input: &str, folder: &str) -> bool {
    let Some(writes) = recipe.permissions.as_ref().and_then(|p| p["filesystem"]["write"].as_array()) else { return false };
    let as_ref = format!("${{inputs.{input}}}");
    writes.iter().filter_map(Value::as_str).any(|w| {
        if w == as_ref || w == "input" {
            return true;
        }
        let w = w.trim_end_matches('/');
        !w.starts_with("${") && w != "output" && Path::new(w).canonicalize().map(|p| p.to_string_lossy() == folder).unwrap_or(w == folder)
    })
}

/// Watches one folder and calls back once things have been quiet for `settle`, with the
/// files that changed. Dropping it stops the watch.
pub struct FolderWatcher {
    _watcher: notify::RecommendedWatcher,
}

impl FolderWatcher {
    pub fn start(
        folder: impl AsRef<Path>,
        settle: Duration,
        on_change: impl Fn(Vec<PathBuf>) + Send + 'static,
    ) -> Result<Self, notify::Error> {
        let folder = folder.as_ref().to_path_buf();
        let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher = notify::recommended_watcher(move |ev| {
            let _ = tx.send(ev);
        })?;
        watcher.watch(&folder, RecursiveMode::NonRecursive)?;
        std::thread::Builder::new()
            .name("stillroom-watch".into())
            .spawn(move || settle_loop(rx, &folder, settle, on_change))
            .map_err(|e| notify::Error::generic(&e.to_string()))?;
        Ok(Self { _watcher: watcher })
    }
}

/// Collects change events and fires once the folder has been quiet for `settle`. Ends when
/// the watcher is dropped.
fn settle_loop(
    rx: mpsc::Receiver<notify::Result<notify::Event>>,
    folder: &Path,
    settle: Duration,
    on_change: impl Fn(Vec<PathBuf>),
) {
    let mut pending: BTreeSet<PathBuf> = BTreeSet::new();
    let mut deadline: Option<Instant> = None;
    loop {
        let wait = match deadline {
            Some(d) => d.saturating_duration_since(Instant::now()),
            None => Duration::from_secs(3600),
        };
        match rx.recv_timeout(wait) {
            Ok(Ok(event)) => {
                for path in event.paths {
                    if is_interesting(folder, &path) {
                        pending.insert(path);
                        deadline = Some(Instant::now() + settle);
                    }
                }
            }
            Ok(Err(_)) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if deadline.is_some_and(|d| Instant::now() >= d) {
                    deadline = None;
                    if !pending.is_empty() {
                        on_change(std::mem::take(&mut pending).into_iter().collect());
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// Files worth running for: not the folder itself, not hidden files, not the half-written
/// temporaries that copies and downloads leave behind.
pub fn is_interesting(folder: &Path, path: &Path) -> bool {
    if path == folder || path.is_dir() {
        return false;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return false };
    if name.starts_with('.') || name.ends_with('~') {
        return false;
    }
    const TEMP: [&str; 6] = [".tmp", ".part", ".partial", ".crdownload", ".download", ".swp"];
    !TEMP.iter().any(|ext| name.to_ascii_lowercase().ends_with(ext))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    const YAML: &str = r#"
schemaVersion: "0"
id: w
name: W
inputs:
  inbox: { type: directory }
  archive: { type: directory }
  note: { type: text }
modelSlots: {}
permissions:
  filesystem:
    read: [input]
    write: ["${inputs.archive}"]
steps:
  - id: list
    kind: tool
    tool: fs.list
    inputs: { path: "${inputs.inbox}" }
outputs:
  files: { from: "${steps.list.output}", type: json }
"#;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stillroom-watch-{name}-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn checks_the_input_the_folder_and_write_loops() {
        let recipe = Recipe::from_yaml(YAML).unwrap();
        let dir = temp("new");
        let inputs = IndexMap::from([("note".to_string(), Value::String("hi".into()))]);
        let w = Watch::new(&recipe, "inbox", dir.to_str().unwrap(), inputs.clone(), BTreeMap::new()).unwrap();
        assert_eq!(w.inputs["inbox"], Value::String(dir.canonicalize().unwrap().to_string_lossy().into()));
        assert_eq!(w.inputs["note"], "hi");
        assert!(w.enabled);

        let e = Watch::new(&recipe, "note", dir.to_str().unwrap(), inputs.clone(), BTreeMap::new()).unwrap_err();
        assert!(matches!(e, WatchError::NotADirectoryInput(_)), "{e}");
        let e = Watch::new(&recipe, "nope", dir.to_str().unwrap(), inputs.clone(), BTreeMap::new()).unwrap_err();
        assert!(matches!(e, WatchError::NoSuchInput(_)), "{e}");
        let e = Watch::new(&recipe, "inbox", "/definitely/not/here", inputs.clone(), BTreeMap::new()).unwrap_err();
        assert!(matches!(e, WatchError::NoSuchFolder(_)), "{e}");
        let e = Watch::new(&recipe, "archive", dir.to_str().unwrap(), inputs, BTreeMap::new()).unwrap_err();
        assert!(e.to_string().contains("writes into"), "{e}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn ignores_hidden_and_temporary_files() {
        let folder = Path::new("/tmp/in");
        assert!(is_interesting(folder, Path::new("/tmp/in/mail.eml")));
        assert!(!is_interesting(folder, Path::new("/tmp/in/.DS_Store")));
        assert!(!is_interesting(folder, Path::new("/tmp/in/mail.eml.part")));
        assert!(!is_interesting(folder, Path::new("/tmp/in/download.crdownload")));
        assert!(!is_interesting(folder, Path::new("/tmp/in/notes.md~")));
        assert!(!is_interesting(folder, folder));
    }

    #[test]
    fn fires_once_after_the_folder_settles() {
        let dir = temp("fire");
        let seen: Arc<Mutex<Vec<Vec<PathBuf>>>> = Arc::default();
        let sink = seen.clone();
        let _w = FolderWatcher::start(&dir, Duration::from_millis(300), move |paths| sink.lock().unwrap().push(paths)).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(dir.join(".DS_Store"), "noise").unwrap();
        std::fs::write(dir.join("a.eml"), "From: a\r\n\r\nhi").unwrap();
        std::fs::write(dir.join("b.eml"), "From: b\r\n\r\nhi").unwrap();
        let started = Instant::now();
        while seen.lock().unwrap().is_empty() && started.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(50));
        }
        std::thread::sleep(Duration::from_millis(700));
        let fired = seen.lock().unwrap().clone();
        assert_eq!(fired.len(), 1, "one firing for a burst of writes: {fired:?}");
        let names: Vec<String> = fired[0].iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert!(names.contains(&"a.eml".to_string()) && names.contains(&"b.eml".to_string()), "{names:?}");
        assert!(!names.iter().any(|n| n.starts_with('.')), "{names:?}");
        std::fs::remove_dir_all(dir).ok();
    }
}
