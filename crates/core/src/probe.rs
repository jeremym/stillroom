//! What a run would touch, checked before it starts: each permission entry of the recipe as one
//! row, resolved against the inputs typed so far, with the folders and files actually tried.
//! Set up shows the rows and refuses to run while a folder cannot be written or read, so the
//! person learns it there and not from a failed step.

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::recipe::Recipe;
use crate::refs;
use crate::tools::normalize_path;

/// One line of the permission rows on Set up.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRow {
    /// `read`, `write`, `reach`, or `run`.
    pub kind: &'static str,
    /// Who or what, in the recipe's own words: an input's label, "the files and folders you choose", a host.
    pub label: String,
    /// What that resolves to right now: the paths chosen, the hosts in the links; `None` when nothing is chosen yet.
    pub detail: Option<String>,
    /// `ok`, `not_chosen`, `cannot_read`, `cannot_write`, `missing`.
    pub status: &'static str,
    /// A short note when the status is not plainly ok: what will happen, or what to do.
    pub note: Option<String>,
}

fn input_label(recipe: &Recipe, name: &str) -> String {
    recipe.inputs.get(name).and_then(|s| s.get("label")).and_then(Value::as_str).map(String::from).unwrap_or_else(|| name.to_string())
}

fn value_paths(v: &Value) -> Vec<PathBuf> {
    match v {
        Value::String(s) if !s.trim().is_empty() => vec![normalize_path(s)],
        Value::Array(a) => a.iter().filter_map(Value::as_str).filter(|s| !s.trim().is_empty()).map(normalize_path).collect(),
        _ => Vec::new(),
    }
}

fn shown(paths: &[PathBuf]) -> String {
    let names: Vec<String> = paths.iter().map(|p| display_short(p)).collect();
    names.join(", ")
}

/// A path with the home folder written as `~`, which is how people read their own paths.
pub fn display_short(p: &Path) -> String {
    let s = p.display().to_string();
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) if p.starts_with(&home) => format!("~/{}", p.strip_prefix(&home).map(|r| r.display().to_string()).unwrap_or_default()).trim_end_matches('/').to_string(),
        _ => s,
    }
}

/// Can every path be read? A folder is opened for listing, a file for reading.
fn readable(paths: &[PathBuf]) -> Result<(), String> {
    for p in paths {
        let r = if p.is_dir() { std::fs::read_dir(p).map(|_| ()) } else { std::fs::File::open(p).map(|_| ()) };
        if let Err(e) = r {
            return Err(match e.kind() {
                std::io::ErrorKind::NotFound => format!("{} does not exist", display_short(p)),
                std::io::ErrorKind::PermissionDenied => format!("macOS has not let Stillroom read {}; allow it in System Settings › Privacy & Security › Files and Folders", display_short(p)),
                _ => format!("{}: {e}", display_short(p)),
            });
        }
    }
    Ok(())
}

/// Can a folder be written? A probe file is created and removed. A folder that does not exist
/// yet is fine when its nearest existing parent can be written, since `fs.write` creates it.
fn writable(dir: &Path) -> Result<Option<String>, String> {
    let mut target = dir.to_path_buf();
    let mut created = false;
    while !target.exists() {
        created = true;
        match target.parent() {
            Some(p) if p != target => target = p.to_path_buf(),
            _ => return Err(format!("{} does not exist and cannot be created", display_short(dir))),
        }
    }
    if !target.is_dir() {
        return Err(format!("{} is a file, not a folder", display_short(&target)));
    }
    let probe = target.join(format!(".stillroom-write-probe-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            Ok(created.then(|| format!("{} does not exist yet; the run creates it", display_short(dir))))
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Err(format!("Stillroom cannot write in {}; choose another folder, or allow it in System Settings › Privacy & Security › Files and Folders", display_short(&target))),
        Err(e) => Err(format!("{}: {e}", display_short(&target))),
    }
}

/// The rows for a recipe with these inputs. Entries follow the recipe's `permissions` block:
/// `input` is every file, files, or directory input; `output` the run's own folder;
/// `${inputs.x}` one input; anything else a literal path or host.
pub fn probe_permissions(recipe: &Recipe, inputs: &IndexMap<String, Value>) -> Vec<PermissionRow> {
    let path_input_names: Vec<String> = recipe
        .inputs
        .iter()
        .filter(|(_, spec)| matches!(spec.get("type").and_then(Value::as_str), Some("file" | "files" | "directory")))
        .map(|(n, _)| n.clone())
        .collect();
    let fs = recipe.permissions.as_ref().and_then(|p| p.get("filesystem"));
    let list = |key: &str| -> Option<Vec<String>> { fs?.get(key)?.as_array().map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()) };
    let read = list("read").unwrap_or_else(|| vec!["input".into()]);
    let write = list("write").unwrap_or_default();
    // A folder the run writes into is not something it reads: it may not exist yet.
    let write_targets: Vec<String> = write.iter().filter_map(|e| match refs::parse_ref(e) { Some(Ok(refs::Ref::Input { name, .. })) => Some(name), _ => None }).collect();
    let read_input_names: Vec<&String> = path_input_names.iter().filter(|n| !write_targets.contains(n)).collect();
    let mut rows = Vec::new();

    for entry in &read {
        match entry.as_str() {
            "input" => {
                let chosen: Vec<PathBuf> = read_input_names.iter().filter_map(|n| inputs.get(n.as_str())).flat_map(value_paths).collect();
                if read_input_names.is_empty() {
                    continue;
                }
                rows.push(match (chosen.is_empty(), readable(&chosen)) {
                    (true, _) => PermissionRow { kind: "read", label: "the files and folders you choose".into(), detail: None, status: "not_chosen", note: Some("nothing chosen yet".into()) },
                    (false, Ok(())) => PermissionRow { kind: "read", label: "the files and folders you choose".into(), detail: Some(shown(&chosen)), status: "ok", note: None },
                    (false, Err(e)) => PermissionRow { kind: "read", label: "the files and folders you choose".into(), detail: Some(shown(&chosen)), status: "cannot_read", note: Some(e) },
                });
            }
            "output" => rows.push(PermissionRow { kind: "read", label: "the run's own folder".into(), detail: None, status: "ok", note: None }),
            e => match refs::parse_ref(e) {
                Some(Ok(refs::Ref::Input { name, .. })) => {
                    let label = input_label(recipe, &name);
                    let chosen = inputs.get(&name).map(value_paths).unwrap_or_default();
                    rows.push(match (chosen.is_empty(), readable(&chosen)) {
                        (true, _) => PermissionRow { kind: "read", label, detail: None, status: "not_chosen", note: Some("nothing chosen yet".into()) },
                        (false, Ok(())) => PermissionRow { kind: "read", label, detail: Some(shown(&chosen)), status: "ok", note: None },
                        (false, Err(err)) => PermissionRow { kind: "read", label, detail: Some(shown(&chosen)), status: "cannot_read", note: Some(err) },
                    });
                }
                Some(_) => {}
                None => {
                    let p = normalize_path(e);
                    rows.push(match readable(std::slice::from_ref(&p)) {
                        Ok(()) => PermissionRow { kind: "read", label: display_short(&p), detail: None, status: "ok", note: None },
                        Err(err) => PermissionRow { kind: "read", label: display_short(&p), detail: None, status: "cannot_read", note: Some(err) },
                    });
                }
            },
        }
    }

    for entry in &write {
        match entry.as_str() {
            "output" => rows.push(PermissionRow { kind: "write", label: "the run's own folder".into(), detail: None, status: "ok", note: None }),
            "input" => {
                let chosen: Vec<PathBuf> = path_input_names.iter().filter_map(|n| inputs.get(n)).flat_map(value_paths).collect();
                rows.push(write_row("the folders you choose".into(), &chosen));
            }
            e => match refs::parse_ref(e) {
                Some(Ok(refs::Ref::Input { name, .. })) => {
                    let chosen = inputs.get(&name).map(value_paths).unwrap_or_default();
                    rows.push(write_row(input_label(recipe, &name), &chosen));
                }
                Some(_) => {}
                None => {
                    let p = normalize_path(e);
                    rows.push(write_row(display_short(&p), std::slice::from_ref(&p)));
                }
            },
        }
    }
    if write.is_empty() {
        rows.push(PermissionRow { kind: "write", label: "only its own results".into(), detail: None, status: "ok", note: None });
    }

    let net: Vec<String> = recipe.permissions.as_ref().and_then(|p| p.get("network")).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default();
    let link_inputs: Vec<&String> = recipe.inputs.iter().filter(|(_, s)| matches!(s.get("type").and_then(Value::as_str), Some("url" | "urls"))).map(|(n, _)| n).collect();
    let hosts_of = |names: &[&String]| -> Vec<String> {
        let mut hosts: Vec<String> = names
            .iter()
            .filter_map(|n| inputs.get(n.as_str()))
            .flat_map(|v| match v {
                Value::String(s) => vec![s.clone()],
                Value::Array(a) => a.iter().filter_map(Value::as_str).map(String::from).collect(),
                _ => Vec::new(),
            })
            .filter_map(|s| url::Url::parse(s.trim()).ok().and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase())))
            .collect();
        hosts.sort();
        hosts.dedup();
        hosts
    };
    for entry in &net {
        match entry.as_str() {
            "input" => {
                let hosts = hosts_of(&link_inputs);
                rows.push(reach_row("the sites in the links you give it".into(), hosts));
            }
            "*" => rows.push(PermissionRow { kind: "reach", label: "any site".into(), detail: None, status: "ok", note: None }),
            e => match refs::parse_ref(e) {
                Some(Ok(refs::Ref::Input { name, .. })) => {
                    let hosts = hosts_of(&[&name]);
                    rows.push(reach_row(format!("the sites in “{}”", input_label(recipe, &name)), hosts));
                }
                Some(_) => {}
                None => rows.push(PermissionRow { kind: "reach", label: e.to_string(), detail: None, status: "ok", note: None }),
            },
        }
    }
    if recipe.permissions.as_ref().and_then(|p| p.get("shell")).and_then(Value::as_bool).unwrap_or(false) {
        rows.push(PermissionRow { kind: "run", label: "programs on this Mac".into(), detail: None, status: "ok", note: None });
    }
    rows
}

fn write_row(label: String, chosen: &[PathBuf]) -> PermissionRow {
    if chosen.is_empty() {
        return PermissionRow { kind: "write", label, detail: None, status: "not_chosen", note: Some("nothing chosen yet".into()) };
    }
    let mut notes = Vec::new();
    for p in chosen {
        match writable(p) {
            Ok(Some(n)) => notes.push(n),
            Ok(None) => {}
            Err(e) => return PermissionRow { kind: "write", label, detail: Some(shown(chosen)), status: "cannot_write", note: Some(e) },
        }
    }
    PermissionRow { kind: "write", label, detail: Some(shown(chosen)), status: "ok", note: (!notes.is_empty()).then(|| notes.join("; ")) }
}

fn reach_row(label: String, hosts: Vec<String>) -> PermissionRow {
    if hosts.is_empty() {
        PermissionRow { kind: "reach", label, detail: None, status: "not_chosen", note: Some("no links yet".into()) }
    } else {
        let n = hosts.len();
        PermissionRow { kind: "reach", label, detail: Some(format!("{n} site{}: {}", if n == 1 { "" } else { "s" }, hosts.join(", "))), status: "ok", note: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn recipe(permissions: Value) -> Recipe {
        serde_json::from_value(json!({
            "schemaVersion": "0", "id": "p", "name": "P",
            "inputs": {
                "folder": { "type": "directory", "label": "Your documents" },
                "out": { "type": "directory", "label": "Where to save" },
                "urls": { "type": "urls", "label": "Pages to read" }
            },
            "permissions": permissions,
            "steps": [], "outputs": {}
        }))
        .unwrap()
    }

    #[test]
    fn nothing_chosen_yet_says_so_and_blocks_nothing() {
        let r = recipe(json!({ "filesystem": { "read": ["input"], "write": ["${inputs.out}"] }, "network": ["input"] }));
        let rows = probe_permissions(&r, &IndexMap::new());
        let kinds: Vec<(&str, &str)> = rows.iter().map(|r| (r.kind, r.status)).collect();
        assert_eq!(kinds, vec![("read", "not_chosen"), ("write", "not_chosen"), ("reach", "not_chosen")]);
        assert_eq!(rows[1].label, "Where to save");
    }

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("stillroom-probe-{tag}-{}", uuid::Uuid::now_v7()));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_readable_folder_a_writable_folder_and_the_hosts_are_ok() {
        let dir = Tmp::new("ok");
        let r = recipe(json!({ "filesystem": { "read": ["input"], "write": ["${inputs.out}"] }, "network": ["input"] }));
        let mut inputs = IndexMap::new();
        inputs.insert("folder".to_string(), json!(dir.path().display().to_string()));
        inputs.insert("out".to_string(), json!(dir.path().join("new").join("deeper").display().to_string()));
        inputs.insert("urls".to_string(), json!(["https://Example.org/a", "https://example.org/b", "https://docs.rs/x"]));
        let rows = probe_permissions(&r, &inputs);
        assert_eq!(rows[0].status, "ok");
        assert_eq!(rows[1].status, "ok");
        assert!(rows[1].note.as_deref().unwrap().contains("does not exist yet; the run creates it"), "{:?}", rows[1]);
        assert!(!dir.path().join("new").exists(), "the probe must not create the folder");
        assert_eq!(rows[2].detail.as_deref(), Some("2 sites: docs.rs, example.org"));
    }

    #[test]
    fn a_missing_file_and_a_folder_that_is_a_file_are_named() {
        let dir = Tmp::new("bad");
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "x").unwrap();
        let r = recipe(json!({ "filesystem": { "read": ["input"], "write": ["${inputs.out}"] } }));
        let mut inputs = IndexMap::new();
        inputs.insert("folder".to_string(), json!(dir.path().join("gone").display().to_string()));
        inputs.insert("out".to_string(), json!(file.display().to_string()));
        let rows = probe_permissions(&r, &inputs);
        assert_eq!(rows[0].status, "cannot_read");
        assert!(rows[0].note.as_deref().unwrap().ends_with("does not exist"));
        assert_eq!(rows[1].status, "cannot_write");
        assert!(rows[1].note.as_deref().unwrap().contains("is a file, not a folder"));
        // No network block: no reach row; no write entries would say "only its own results".
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn a_recipe_with_no_permissions_block_reads_inputs_and_writes_its_own_results() {
        let r = recipe(Value::Null);
        let rows = probe_permissions(&r, &IndexMap::new());
        assert_eq!(rows.iter().map(|r| (r.kind, r.label.as_str())).collect::<Vec<_>>(), vec![("read", "the files and folders you choose"), ("write", "only its own results")]);
    }
}
