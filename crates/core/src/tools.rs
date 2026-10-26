//! Tools: deterministic capabilities a tool step can call.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("unknown tool {0}")]
    Unknown(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("{0}")]
    Failed(String),
}

/// What a tool may know about the run it is part of.
#[derive(Debug, Clone)]
pub struct ToolContext {
    pub run_id: String,
    /// Directory this run may write artifact files into. Created on demand.
    pub artifact_dir: std::path::PathBuf,
}

impl ToolContext {
    pub fn ensure_artifact_dir(&self) -> Result<&std::path::Path, ToolError> {
        std::fs::create_dir_all(&self.artifact_dir)
            .map_err(|e| ToolError::Failed(format!("{}: {e}", self.artifact_dir.display())))?;
        Ok(&self.artifact_dir)
    }
}

#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn id(&self) -> &str;
    async fn call(&self, ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError>;
}

#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// The registry with every built-in tool.
    pub fn builtin() -> Self {
        let mut r = Self::new();
        r.register(Arc::new(FileRead::default()));
        r.register(Arc::new(ConceptMap));
        r.register(Arc::new(Speak));
        r
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.id().to_string(), tool);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(id).cloned()
    }

    pub fn ids(&self) -> Vec<&str> {
        self.tools.keys().map(String::as_str).collect()
    }
}

/// `file.read`: read a UTF-8 text file. Output `{ path, text, bytes }`.
pub struct FileRead {
    pub max_bytes: u64,
}

impl Default for FileRead {
    fn default() -> Self {
        Self { max_bytes: 4 * 1024 * 1024 }
    }
}

#[async_trait::async_trait]
impl Tool for FileRead {
    fn id(&self) -> &str {
        "file.read"
    }

    async fn call(&self, _ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let path = inputs
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("path (string) is required".into()))?;
        let meta = tokio::fs::metadata(path).await.map_err(|e| ToolError::Failed(format!("{path}: {e}")))?;
        if meta.len() > self.max_bytes {
            return Err(ToolError::Failed(format!(
                "{path} is {} bytes; limit is {}",
                meta.len(),
                self.max_bytes
            )));
        }
        let text = tokio::fs::read_to_string(path).await.map_err(|e| ToolError::Failed(format!("{path}: {e}")))?;
        Ok(json!({ "path": path, "text": text, "bytes": meta.len() }))
    }
}

/// `spine.concept_map`: draw a Knowledge Spine as a Mermaid flowchart. Deterministic:
/// solid arrows from prerequisites to the concepts that need them, dotted lines for
/// related concepts. Output `{ format: "mermaid", title, source, nodes, edges }`.
pub struct ConceptMap;

fn mermaid_id(raw: &str) -> String {
    let mut id = String::from("n_");
    id.extend(raw.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }));
    id
}

fn mermaid_label(raw: &str) -> String {
    raw.replace('"', "#quot;").replace('\n', " ")
}

#[async_trait::async_trait]
impl Tool for ConceptMap {
    fn id(&self) -> &str {
        "spine.concept_map"
    }

    async fn call(&self, _ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let spine = inputs.get("spine").ok_or_else(|| ToolError::InvalidInput("spine is required".into()))?;
        let concepts = spine
            .get("concepts")
            .and_then(Value::as_array)
            .ok_or_else(|| ToolError::InvalidInput("spine.concepts must be an array".into()))?;
        let direction = inputs.get("direction").and_then(Value::as_str).unwrap_or("TD");
        let include_related = inputs.get("includeRelated").and_then(Value::as_bool).unwrap_or(true);
        let title = spine.get("title").and_then(Value::as_str).unwrap_or("Concept map");

        let known: std::collections::HashSet<&str> =
            concepts.iter().filter_map(|c| c.get("id").and_then(Value::as_str)).collect();
        let mut out = format!("flowchart {direction}\n");
        let mut edges = 0usize;
        let mut seen_related: std::collections::HashSet<(String, String)> = Default::default();

        for c in concepts {
            let (Some(id), Some(name)) = (c.get("id").and_then(Value::as_str), c.get("name").and_then(Value::as_str)) else {
                continue;
            };
            out.push_str(&format!("    {}[\"{}\"]\n", mermaid_id(id), mermaid_label(name)));
        }
        for c in concepts {
            let Some(id) = c.get("id").and_then(Value::as_str) else { continue };
            for p in c.get("prerequisites").and_then(Value::as_array).into_iter().flatten() {
                if let Some(p) = p.as_str().filter(|p| known.contains(p) && *p != id) {
                    out.push_str(&format!("    {} --> {}\n", mermaid_id(p), mermaid_id(id)));
                    edges += 1;
                }
            }
            if include_related {
                for r in c.get("related").and_then(Value::as_array).into_iter().flatten() {
                    if let Some(r) = r.as_str().filter(|r| known.contains(r) && *r != id) {
                        let key = if id < r { (id.to_string(), r.to_string()) } else { (r.to_string(), id.to_string()) };
                        if seen_related.insert(key) {
                            out.push_str(&format!("    {} -.- {}\n", mermaid_id(id), mermaid_id(r)));
                            edges += 1;
                        }
                    }
                }
            }
        }
        Ok(json!({
            "format": "mermaid",
            "title": title,
            "source": out,
            "nodes": known.len(),
            "edges": edges,
        }))
    }
}

/// `speech.synthesize`: read text aloud to an audio file with the system voice.
/// Inputs: `text` (string) or `script` (`{ chapters: [{ title, text }] }`), optional
/// `voice`, `rate` (words per minute), `name` (file stem). Output
/// `{ path, format, durationSeconds, voice, chars }`.
///
/// macOS only for now (`say`). Kokoro and other engines arrive as model providers.
pub struct Speak;

impl Speak {
    /// Flatten a script into one text with pauses between chapters, using `say`'s
    /// embedded `[[slnc ms]]` command.
    pub fn script_text(inputs: &Value) -> Option<String> {
        if let Some(t) = inputs.get("text").and_then(Value::as_str) {
            return Some(t.to_string());
        }
        let chapters = inputs.get("script")?.get("chapters")?.as_array()?;
        let mut out = String::new();
        for (i, ch) in chapters.iter().enumerate() {
            if i > 0 {
                out.push_str(" [[slnc 900]] ");
            }
            if let Some(title) = ch.get("title").and_then(Value::as_str) {
                out.push_str(title.trim_end_matches('.'));
                out.push_str(". [[slnc 500]] ");
            }
            out.push_str(ch.get("text").and_then(Value::as_str).unwrap_or_default().trim());
        }
        Some(out)
    }
}

#[async_trait::async_trait]
impl Tool for Speak {
    fn id(&self) -> &str {
        "speech.synthesize"
    }

    async fn call(&self, ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        if !cfg!(target_os = "macos") {
            return Err(ToolError::Failed("speech.synthesize currently requires macOS".into()));
        }
        let text = Self::script_text(&inputs)
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| ToolError::InvalidInput("text (string) or script.chapters is required".into()))?;
        let name = inputs.get("name").and_then(Value::as_str).unwrap_or("narration");
        let name: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        let dir = ctx.ensure_artifact_dir()?;
        let txt = dir.join(format!("{name}.txt"));
        let out = dir.join(format!("{name}.m4a"));
        tokio::fs::write(&txt, &text).await.map_err(|e| ToolError::Failed(format!("{}: {e}", txt.display())))?;

        let mut cmd = tokio::process::Command::new("say");
        cmd.arg("-o").arg(&out).arg("--file-format=m4af").arg("--data-format=aac").arg("-f").arg(&txt);
        let voice = inputs.get("voice").and_then(Value::as_str).filter(|v| !v.trim().is_empty());
        if let Some(v) = voice {
            cmd.arg("-v").arg(v);
        }
        if let Some(r) = inputs.get("rate").and_then(Value::as_u64) {
            cmd.arg("-r").arg(r.to_string());
        }
        let status = cmd.output().await.map_err(|e| ToolError::Failed(format!("say: {e}")))?;
        if !status.status.success() {
            return Err(ToolError::Failed(format!("say failed: {}", String::from_utf8_lossy(&status.stderr).trim())));
        }

        let duration = tokio::process::Command::new("afinfo")
            .arg(&out)
            .output()
            .await
            .ok()
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .find_map(|l| l.trim().strip_prefix("estimated duration:"))
                    .and_then(|s| s.trim().trim_end_matches("sec").trim().parse::<f64>().ok())
            });

        Ok(json!({
            "path": out.to_string_lossy(),
            "format": "m4a",
            "durationSeconds": duration,
            "voice": voice.unwrap_or("system default"),
            "chars": text.chars().count(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ToolContext {
        ToolContext {
            run_id: "test".into(),
            artifact_dir: std::env::temp_dir().join(format!("stillroom-tools-{}", uuid::Uuid::now_v7())),
        }
    }

    #[test]
    fn script_text_joins_chapters_with_pauses() {
        let v = json!({ "script": { "chapters": [
            { "title": "Intro.", "text": "Hello there." },
            { "title": "Two", "text": "Second part." }
        ]}});
        let t = Speak::script_text(&v).unwrap();
        assert_eq!(t, "Intro. [[slnc 500]] Hello there. [[slnc 900]] Two. [[slnc 500]] Second part.");
        assert_eq!(Speak::script_text(&json!({ "text": "plain" })).unwrap(), "plain");
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn speak_writes_an_audio_file() {
        let c = ctx();
        let out = Speak.call(&c, json!({ "text": "Stillroom test.", "name": "t" })).await.unwrap();
        let path = std::path::PathBuf::from(out["path"].as_str().unwrap());
        assert!(path.exists(), "{path:?}");
        assert!(std::fs::metadata(&path).unwrap().len() > 1000);
        assert!(out["durationSeconds"].as_f64().unwrap_or(0.0) > 0.3, "{out}");
        std::fs::remove_dir_all(c.artifact_dir).ok();
    }

    #[tokio::test]
    async fn concept_map_draws_prerequisites_and_related() {
        let spine = json!({
            "title": "OAuth",
            "concepts": [
                { "id": "oauth", "name": "OAuth 2.0", "prerequisites": [], "related": ["access-token"] },
                { "id": "access-token", "name": "Access \"token\"", "prerequisites": ["oauth"], "related": ["oauth", "ghost"] },
                { "id": "end", "name": "End", "prerequisites": ["access-token"], "related": [] }
            ]
        });
        let out = ConceptMap.call(&ctx(), json!({ "spine": spine })).await.unwrap();
        let src = out["source"].as_str().unwrap();
        assert!(src.starts_with("flowchart TD\n"));
        assert!(src.contains("n_oauth --> n_access_token"), "{src}");
        assert!(src.contains("n_access_token --> n_end"), "{src}");
        assert_eq!(src.matches("-.-").count(), 1, "related edge deduped, unknown id skipped: {src}");
        assert!(src.contains("#quot;token#quot;"), "{src}");
        assert_eq!(out["nodes"], 3);
        assert_eq!(out["edges"], 3);
    }

    #[tokio::test]
    async fn file_read_reports_missing_file() {
        let err = FileRead::default().call(&ctx(), json!({ "path": "/nonexistent/x.txt" })).await.unwrap_err();
        assert!(matches!(err, ToolError::Failed(_)));
    }
}
