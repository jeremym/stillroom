//! The person's Obsidian vault: a folder of Markdown notes chosen once in Settings › Services.
//! `vault.write` puts a note there with YAML front matter and `[[wikilinks]]` to related notes;
//! `vault.list` names the notes already there so a recipe can link to them. Choosing the vault
//! is the permission: these tools write only under it, whatever the recipe's own permissions say.

use chrono::Local;
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::tools::{Tool, ToolContext, ToolDescription, ToolError};

/// Where the vault is, read each call so a change in Settings takes at once.
pub type VaultPath = Arc<dyn Fn() -> Option<PathBuf> + Send + Sync>;

pub const NO_VAULT: &str = "No Obsidian vault is set. In Settings › Services, choose your vault's folder; until then this step cannot write notes.";

fn vault_root(p: &VaultPath) -> Result<PathBuf, ToolError> {
    let root = p().filter(|r| !r.as_os_str().is_empty()).ok_or_else(|| ToolError::Failed(NO_VAULT.into()))?;
    if !root.is_dir() {
        return Err(ToolError::Failed(format!("The Obsidian vault folder {} is not there; choose it again in Settings › Services.", root.display())));
    }
    Ok(root)
}

/// A folder inside the vault from a relative path the model wrote: no `..`, no absolute path.
fn inside(root: &Path, rel: Option<&str>) -> Result<PathBuf, ToolError> {
    let mut p = root.to_path_buf();
    if let Some(rel) = rel.map(str::trim).filter(|r| !r.is_empty()) {
        for c in Path::new(rel).components() {
            match c {
                Component::Normal(n) => p.push(n),
                Component::CurDir => {}
                _ => return Err(ToolError::InvalidInput(format!("{rel}: a note's folder is written relative to the vault, with no .. and no leading /"))),
            }
        }
    }
    Ok(p)
}

/// A note name as Obsidian allows it: no path separators, no characters it refuses, trimmed.
pub fn note_name(raw: &str) -> String {
    let cleaned: String = raw.trim().trim_end_matches(".md").chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '#' | '^' | '[' | ']') { ' ' } else { c }).collect();
    let squashed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if squashed.is_empty() { "Untitled".into() } else { squashed.chars().take(120).collect() }
}

/// YAML for the front matter: tags, then the fields given, then `created`; values are quoted.
pub fn front_matter(tags: &[String], fields: &serde_json::Map<String, Value>, created: &str) -> String {
    let mut s = String::from("---\n");
    if !tags.is_empty() {
        s.push_str("tags:\n");
        for t in tags {
            s.push_str(&format!("  - {}\n", yaml_scalar(t)));
        }
    }
    for (k, v) in fields {
        if k == "tags" || k == "created" {
            continue;
        }
        match v {
            Value::Array(a) => {
                s.push_str(&format!("{k}:\n"));
                for x in a {
                    s.push_str(&format!("  - {}\n", yaml_scalar(&scalar_text(x))));
                }
            }
            other => s.push_str(&format!("{k}: {}\n", yaml_scalar(&scalar_text(other)))),
        }
    }
    s.push_str(&format!("created: {created}\n---\n"));
    s
}

fn scalar_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn yaml_scalar(s: &str) -> String {
    if s.is_empty() || s.chars().any(|c| matches!(c, ':' | '#' | '"' | '\'' | '[' | ']' | '{' | '}' | ',' | '&' | '*' | '!' | '|' | '>' | '%' | '@' | '`')) || s.trim() != s || s.parse::<f64>().is_ok() || matches!(s, "true" | "false" | "null" | "yes" | "no") {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

/// The note's text: front matter, the body, then a Related section with a wikilink per name
/// the body does not already link to.
pub fn compose_note(body: &str, tags: &[String], fields: &serde_json::Map<String, Value>, links: &[String], created: &str) -> String {
    let mut s = front_matter(tags, fields, created);
    s.push('\n');
    s.push_str(body.trim_end());
    s.push('\n');
    let fresh: Vec<&String> = links.iter().filter(|l| !body.contains(&format!("[[{l}]]")) && !body.contains(&format!("[[{l}|"))).collect();
    if !fresh.is_empty() {
        s.push_str("\n## Related\n\n");
        for l in fresh {
            s.push_str(&format!("- [[{l}]]\n"));
        }
    }
    s
}

pub struct VaultWrite {
    pub vault: VaultPath,
}

#[async_trait::async_trait]
impl Tool for VaultWrite {
    fn id(&self) -> &str {
        "vault.write"
    }

    fn cacheable(&self) -> bool {
        false
    }

    fn has_side_effects(&self) -> bool {
        true
    }

    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "Write a note into the user's Obsidian vault (chosen once in Settings › Services; the recipe needs no folder input or permission for it). The note gets YAML front matter (tags, any fields you give, created) and a Related section with a [[wikilink]] to each name in links.".into(),
            returns: "`{ path, note, folder, links }`; `note` is the name other notes link to as [[note]].".into(),
            parameters: json!({
                "type": "object",
                "required": ["name", "text"],
                "properties": {
                    "name": { "type": "string", "description": "The note's name, which is also its file name (without .md)." },
                    "text": { "type": "string", "description": "The note's body in Markdown. Link other notes inline as [[Their name]] where it reads well." },
                    "folder": { "type": "string", "description": "A folder inside the vault, such as Reading/Briefs (made if missing). The vault's root when left out." },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags for the front matter." },
                    "frontmatter": { "type": "object", "description": "Extra front matter fields (source, author, status, …)." },
                    "links": { "type": "array", "items": { "type": "string" }, "description": "Names of related notes; each becomes a [[wikilink]] under Related unless the body already links it." },
                    "overwrite": { "type": "boolean", "description": "Replace a note of the same name (default false)." }
                }
            }),
        })
    }

    async fn call(&self, _ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let root = vault_root(&self.vault)?;
        let name = note_name(inputs.get("name").and_then(Value::as_str).ok_or_else(|| ToolError::InvalidInput("name is required".into()))?);
        let text = inputs.get("text").and_then(Value::as_str).ok_or_else(|| ToolError::InvalidInput("text (the note's Markdown) is required".into()))?;
        let folder = inside(&root, inputs.get("folder").and_then(Value::as_str))?;
        let tags: Vec<String> = inputs.get("tags").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(|t| t.trim().trim_start_matches('#').to_string()).filter(|t| !t.is_empty()).collect();
        let links: Vec<String> = inputs.get("links").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(note_name).filter(|l| l != &name).collect();
        let fields = inputs.get("frontmatter").and_then(Value::as_object).cloned().unwrap_or_default();
        let path = folder.join(format!("{name}.md"));
        if path.exists() && !inputs.get("overwrite").and_then(Value::as_bool).unwrap_or(false) {
            return Err(ToolError::Failed(format!("the vault already has a note called “{name}” in {} (set overwrite to replace it)", folder.strip_prefix(&root).unwrap_or(&folder).display())));
        }
        tokio::fs::create_dir_all(&folder).await.map_err(|e| ToolError::Failed(format!("{}: {e}", folder.display())))?;
        let created = Local::now().format("%Y-%m-%d").to_string();
        let note = compose_note(text, &tags, &fields, &links, &created);
        tokio::fs::write(&path, note).await.map_err(|e| ToolError::Failed(format!("{}: {e}", path.display())))?;
        Ok(json!({ "path": path.to_string_lossy(), "note": name, "folder": folder.strip_prefix(&root).unwrap_or(Path::new("")).to_string_lossy(), "links": links }))
    }
}

pub struct VaultList {
    pub vault: VaultPath,
}

/// At most this many notes are listed.
pub const MAX_NOTES: usize = 500;

#[async_trait::async_trait]
impl Tool for VaultList {
    fn id(&self) -> &str {
        "vault.list"
    }

    fn cacheable(&self) -> bool {
        false
    }

    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "List the notes in the user's Obsidian vault (or one folder of it), so a step can link to notes that exist. Names are what [[wikilinks]] use.".into(),
            returns: "`{ folder, count, notes: [{ name, folder, path, modified }] }`, newest first, at most 500.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "folder": { "type": "string", "description": "A folder inside the vault; the whole vault when left out." },
                    "query": { "type": "string", "description": "Keep only notes whose name contains this (case does not matter)." }
                }
            }),
        })
    }

    async fn call(&self, _ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let root = vault_root(&self.vault)?;
        let folder = inside(&root, inputs.get("folder").and_then(Value::as_str))?;
        let query = inputs.get("query").and_then(Value::as_str).map(|q| q.trim().to_lowercase()).filter(|q| !q.is_empty());
        let r = root.clone();
        let f = folder.clone();
        let notes = tokio::task::spawn_blocking(move || {
            let mut out: Vec<(std::time::SystemTime, Value)> = Vec::new();
            let mut stack = vec![f.clone()];
            while let Some(dir) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&dir) else { continue };
                for e in entries.flatten() {
                    let p = e.path();
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') {
                        continue;
                    }
                    if p.is_dir() {
                        stack.push(p);
                    } else if p.extension().is_some_and(|x| x == "md") {
                        let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                        if query.as_ref().is_some_and(|q| !stem.to_lowercase().contains(q.as_str())) {
                            continue;
                        }
                        let modified = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                        let when: chrono::DateTime<Local> = modified.into();
                        out.push((modified, json!({ "name": stem, "folder": p.parent().and_then(|d| d.strip_prefix(&r).ok()).map(|d| d.to_string_lossy().into_owned()).unwrap_or_default(), "path": p.to_string_lossy(), "modified": when.format("%Y-%m-%d").to_string() })));
                    }
                }
            }
            out.sort_by(|a, b| b.0.cmp(&a.0));
            out.into_iter().map(|(_, v)| v).take(MAX_NOTES).collect::<Vec<_>>()
        })
        .await
        .map_err(|e| ToolError::Failed(e.to_string()))?;
        Ok(json!({ "folder": folder.strip_prefix(&root).unwrap_or(Path::new("")).to_string_lossy(), "count": notes.len(), "notes": notes }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ToolContext {
        ToolContext { run_id: "t".into(), artifact_dir: std::env::temp_dir(), fs: crate::tools::FsPolicy::open(), net: crate::tools::NetPolicy::none(), keeps: Default::default(), models: None, resolutions: Default::default() }
    }

    #[tokio::test]
    async fn a_note_lands_in_the_vault_with_front_matter_and_links_and_can_be_listed() {
        let vault = std::env::temp_dir().join(format!("stillroom-vault-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&vault).unwrap();
        let v: VaultPath = { let p = vault.clone(); Arc::new(move || Some(p.clone())) };
        let w = VaultWrite { vault: v.clone() };
        let out = w.call(&ctx(), json!({ "name": "Graph theory: a brief", "folder": "Reading/Briefs", "text": "The brief.\n\nSee [[Euler]] for the start.", "tags": ["#brief", "graphs"], "frontmatter": { "source": "https://x.example/a", "status": "draft" }, "links": ["Euler", "Hamilton", "Graph theory: a brief"] })).await.unwrap();
        assert_eq!(out["note"], "Graph theory a brief");
        assert_eq!(out["folder"], "Reading/Briefs");
        assert_eq!(out["links"], json!(["Euler", "Hamilton"]));
        let text = std::fs::read_to_string(vault.join("Reading/Briefs/Graph theory a brief.md")).unwrap();
        assert!(text.starts_with("---\ntags:\n  - brief\n  - graphs\nsource: \"https://x.example/a\"\nstatus: draft\ncreated: "), "{text}");
        assert!(text.contains("\n---\n\nThe brief.\n\nSee [[Euler]] for the start.\n\n## Related\n\n- [[Hamilton]]\n"), "Euler is already linked in the body: {text}");
        // A second write refuses without overwrite; a path that climbs out is refused.
        assert!(w.call(&ctx(), json!({ "name": "Graph theory a brief", "folder": "Reading/Briefs", "text": "x" })).await.unwrap_err().to_string().contains("already has a note"));
        assert!(w.call(&ctx(), json!({ "name": "escape", "folder": "../outside", "text": "x" })).await.unwrap_err().to_string().contains("no .."));
        let l = VaultList { vault: v.clone() };
        let listed = l.call(&ctx(), json!({ "query": "graph" })).await.unwrap();
        assert_eq!(listed["count"], 1);
        assert_eq!(listed["notes"][0]["name"], "Graph theory a brief");
        assert_eq!(listed["notes"][0]["folder"], "Reading/Briefs");
        // No vault set: the words say where to set it.
        let none = VaultWrite { vault: Arc::new(|| None) };
        assert!(none.call(&ctx(), json!({ "name": "a", "text": "b" })).await.unwrap_err().to_string().contains("Settings › Services"));
        std::fs::remove_dir_all(vault).ok();
    }

    #[test]
    fn names_and_front_matter_are_what_obsidian_accepts() {
        assert_eq!(note_name("  A/B: c?.md "), "A B c");
        assert_eq!(note_name(""), "Untitled");
        let fm = front_matter(&["a".into()], &serde_json::Map::from_iter([("n".to_string(), json!(3)), ("list".to_string(), json!(["x", "y: z"]))]), "2026-11-04");
        assert_eq!(fm, "---\ntags:\n  - a\nn: \"3\"\nlist:\n  - x\n  - \"y: z\"\ncreated: 2026-11-04\n---\n");
    }
}
