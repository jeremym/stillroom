//! Tools: deterministic capabilities a tool step can call.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("unknown tool {0}")]
    Unknown(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("{0}")]
    Failed(String),
    #[error("not permitted: {0}")]
    Denied(String),
}

/// Where a run may read and write on disk, from the recipe's `permissions.filesystem`
/// resolved against the actual inputs. The run's artifact directory is always writable.
#[derive(Debug, Clone, Default)]
pub struct FsPolicy {
    pub read_roots: Vec<PathBuf>,
    pub write_roots: Vec<PathBuf>,
}

impl FsPolicy {
    /// Everything readable and writable: tests and ad-hoc tool calls.
    pub fn open() -> Self {
        Self { read_roots: vec![PathBuf::from("/")], write_roots: vec![PathBuf::from("/")] }
    }
}

/// What a tool may know about the run it is part of.
#[derive(Debug, Clone)]
pub struct ToolContext {
    pub run_id: String,
    /// Directory this run may write artifact files into. Created on demand.
    pub artifact_dir: PathBuf,
    pub fs: FsPolicy,
}

/// Resolve a path for permission checks: absolute, `~` expanded, `..` collapsed, and the
/// longest existing prefix canonicalized so symlinked temp dirs compare equal.
pub fn normalize_path(raw: &str) -> PathBuf {
    let expanded = if let Some(rest) = raw.strip_prefix("~/") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest)).unwrap_or_else(|| PathBuf::from(raw))
    } else if raw == "~" {
        std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(raw))
    } else {
        PathBuf::from(raw)
    };
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir().map(|d| d.join(&expanded)).unwrap_or(expanded)
    };
    let mut parts: Vec<std::ffi::OsString> = Vec::new();
    for c in absolute.components() {
        match c {
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::CurDir => {}
            other => parts.push(other.as_os_str().to_owned()),
        }
    }
    let clean: PathBuf = parts.iter().collect();
    // Canonicalize the longest prefix that exists, then re-append the rest.
    let mut existing = clean.clone();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        match existing.file_name() {
            Some(n) => {
                tail.push(n.to_owned());
                existing.pop();
            }
            None => break,
        }
    }
    let mut out = existing.canonicalize().unwrap_or(existing);
    for t in tail.iter().rev() {
        out.push(t);
    }
    out
}

impl ToolContext {
    pub fn ensure_artifact_dir(&self) -> Result<&std::path::Path, ToolError> {
        std::fs::create_dir_all(&self.artifact_dir)
            .map_err(|e| ToolError::Failed(format!("{}: {e}", self.artifact_dir.display())))?;
        Ok(&self.artifact_dir)
    }

    fn allowed(&self, roots: &[PathBuf], path: &std::path::Path) -> bool {
        roots.iter().any(|r| path.starts_with(r)) || path.starts_with(normalize_path(&self.artifact_dir.to_string_lossy()))
    }

    /// The normalized path if the recipe may read it.
    pub fn check_read(&self, raw: &str) -> Result<PathBuf, ToolError> {
        let p = normalize_path(raw);
        if self.allowed(&self.fs.read_roots, &p) {
            Ok(p)
        } else {
            Err(ToolError::Denied(format!("reading {} is outside what this recipe may read", p.display())))
        }
    }

    /// The normalized path if the recipe may write it.
    pub fn check_write(&self, raw: &str) -> Result<PathBuf, ToolError> {
        let p = normalize_path(raw);
        if self.allowed(&self.fs.write_roots, &p) {
            Ok(p)
        } else {
            Err(ToolError::Denied(format!("writing {} is outside what this recipe may write", p.display())))
        }
    }
}

/// How a tool describes itself to an agent (the model sees this).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolDescription {
    pub description: String,
    /// JSON Schema for the inputs object.
    pub parameters: Value,
}

#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn id(&self) -> &str;
    /// Whether an output may be reused when the inputs are unchanged. Tools that read
    /// external state the inputs do not capture (a file's contents) say no.
    fn cacheable(&self) -> bool {
        true
    }
    /// Description and parameter schema, for agents. Tools without one cannot be given to agents.
    fn describe(&self) -> Option<ToolDescription> {
        None
    }
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
        r.register(Arc::new(FsList));
        r.register(Arc::new(FsWrite));
        r.register(Arc::new(EmailParse::default()));
        r.register(Arc::new(EmailWriteDraft));
        r.register(Arc::new(ListFilter));
        r.register(Arc::new(ListTable));
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

    /// Every tool with its description and parameter schema (if it has one), sorted by id.
    pub fn catalog(&self) -> Vec<ToolInfo> {
        let mut out: Vec<ToolInfo> = self
            .tools
            .values()
            .map(|t| {
                let d = t.describe();
                ToolInfo {
                    id: t.id().to_string(),
                    description: d.as_ref().map(|d| d.description.clone()).unwrap_or_default(),
                    parameters: d.map(|d| d.parameters),
                    cacheable: t.cacheable(),
                }
            })
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }
}

/// What the app shows about a tool when a recipe author picks one.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolInfo {
    pub id: String,
    pub description: String,
    pub parameters: Option<Value>,
    pub cacheable: bool,
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

    fn cacheable(&self) -> bool {
        false
    }

    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "Read a UTF-8 text file. Returns its text.".into(),
            parameters: json!({ "type": "object", "required": ["path"], "properties": { "path": { "type": "string", "description": "Absolute path of the file" } } }),
        })
    }

    async fn call(&self, ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let raw = inputs
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("path (string) is required".into()))?;
        let path = ctx.check_read(raw)?;
        let path = path.to_string_lossy().into_owned();
        let path = path.as_str();
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

/// `fs.list`: list a directory. Inputs `path`, optional `extensions` (e.g. ["eml"]) and
/// `recursive` (default false). Output `{ path, entries: [{ path, name, isDir, bytes, modified }] }`.
pub struct FsList;

#[async_trait::async_trait]
impl Tool for FsList {
    fn id(&self) -> &str {
        "fs.list"
    }

    fn cacheable(&self) -> bool {
        false
    }

    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "List the files and folders inside a directory.".into(),
            parameters: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string", "description": "Absolute path of the directory" },
                    "extensions": { "type": "array", "items": { "type": "string" }, "description": "Only files with these extensions, e.g. [\"md\"]" },
                    "recursive": { "type": "boolean", "description": "Include subdirectories" }
                }
            }),
        })
    }

    async fn call(&self, ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let raw = inputs
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("path (string) is required".into()))?;
        let root = ctx.check_read(raw)?;
        let exts: Vec<String> = inputs
            .get("extensions")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(|e| e.trim_start_matches('.').to_ascii_lowercase()).collect())
            .unwrap_or_default();
        let recursive = inputs.get("recursive").and_then(Value::as_bool).unwrap_or(false);
        let mut entries = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let mut rd = tokio::fs::read_dir(&dir).await.map_err(|e| ToolError::Failed(format!("{}: {e}", dir.display())))?;
            let mut here = Vec::new();
            while let Some(e) = rd.next_entry().await.map_err(|e| ToolError::Failed(e.to_string()))? {
                let path = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                let meta = e.metadata().await.map_err(|e| ToolError::Failed(e.to_string()))?;
                let is_dir = meta.is_dir();
                if is_dir && recursive {
                    stack.push(path.clone());
                }
                if !is_dir && !exts.is_empty() {
                    let ext = path.extension().map(|x| x.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                    if !exts.contains(&ext) {
                        continue;
                    }
                }
                let modified = meta.modified().ok().map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());
                here.push(json!({
                    "path": path.to_string_lossy(),
                    "name": name,
                    "isDir": is_dir,
                    "bytes": meta.len(),
                    "modified": modified,
                }));
            }
            here.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
            entries.extend(here);
        }
        Ok(json!({ "path": root.to_string_lossy(), "entries": entries, "count": entries.len() }))
    }
}

/// `fs.write`: write a text file inside a writable root. Inputs `path` (or `dir` + `name`),
/// `text`, optional `overwrite` (default false). Creates parent folders. Output `{ path, bytes }`.
pub struct FsWrite;

fn safe_file_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ') { c } else { '_' })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').to_string();
    if trimmed.is_empty() { "untitled".into() } else { trimmed.chars().take(120).collect() }
}

#[async_trait::async_trait]
impl Tool for FsWrite {
    fn id(&self) -> &str {
        "fs.write"
    }

    fn cacheable(&self) -> bool {
        false
    }

    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "Write a text file. Give either a full path, or a directory plus a file name. Parent folders are created.".into(),
            parameters: json!({
                "type": "object",
                "required": ["text"],
                "properties": {
                    "path": { "type": "string", "description": "Absolute path of the file to write" },
                    "dir": { "type": "string", "description": "Directory to write into (with name)" },
                    "name": { "type": "string", "description": "File name (with dir)" },
                    "text": { "type": "string", "description": "File contents" },
                    "overwrite": { "type": "boolean", "description": "Replace an existing file (default false)" }
                }
            }),
        })
    }

    async fn call(&self, ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let text = inputs
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("text (string) is required".into()))?;
        let raw = match (inputs.get("path").and_then(Value::as_str), inputs.get("dir").and_then(Value::as_str)) {
            (Some(p), _) => p.to_string(),
            (None, Some(d)) => {
                let name = inputs.get("name").and_then(Value::as_str).unwrap_or("untitled.txt");
                format!("{}/{}", d.trim_end_matches('/'), safe_file_name(name))
            }
            _ => return Err(ToolError::InvalidInput("path, or dir and name, is required".into())),
        };
        let path = ctx.check_write(&raw)?;
        let overwrite = inputs.get("overwrite").and_then(Value::as_bool).unwrap_or(false);
        if path.exists() && !overwrite {
            return Err(ToolError::Failed(format!("{} already exists (set overwrite to replace it)", path.display())));
        }
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| ToolError::Failed(format!("{}: {e}", parent.display())))?;
        }
        tokio::fs::write(&path, text).await.map_err(|e| ToolError::Failed(format!("{}: {e}", path.display())))?;
        Ok(json!({ "path": path.to_string_lossy(), "bytes": text.len() }))
    }
}

/// `email.parse`: parse an RFC 822 / `.eml` file. Output
/// `{ path, from, to, cc, subject, date, messageId, inReplyTo, text }` (plain text body, HTML stripped if that is all there is).
pub struct EmailParse {
    pub max_bytes: u64,
}

impl Default for EmailParse {
    fn default() -> Self {
        Self { max_bytes: 8 * 1024 * 1024 }
    }
}

fn mail_text(part: &mailparse::ParsedMail) -> Option<String> {
    let ctype = part.ctype.mimetype.to_ascii_lowercase();
    if ctype == "text/plain" {
        return part.get_body().ok();
    }
    if ctype.starts_with("multipart/") {
        // Prefer a plain part anywhere below; fall back to HTML.
        for sub in &part.subparts {
            if let Some(t) = mail_text(sub) {
                return Some(t);
            }
        }
        for sub in &part.subparts {
            if sub.ctype.mimetype.eq_ignore_ascii_case("text/html") {
                return sub.get_body().ok().map(|h| strip_html(&h));
            }
        }
        return None;
    }
    if ctype == "text/html" {
        return part.get_body().ok().map(|h| strip_html(&h));
    }
    None
}

fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[async_trait::async_trait]
impl Tool for EmailParse {
    fn id(&self) -> &str {
        "email.parse"
    }

    fn cacheable(&self) -> bool {
        false
    }

    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "Parse a saved email (.eml) into sender, recipients, subject, date, and plain-text body.".into(),
            parameters: json!({ "type": "object", "required": ["path"], "properties": { "path": { "type": "string" } } }),
        })
    }

    async fn call(&self, ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let raw = inputs
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("path (string) is required".into()))?;
        let path = ctx.check_read(raw)?;
        let meta = tokio::fs::metadata(&path).await.map_err(|e| ToolError::Failed(format!("{}: {e}", path.display())))?;
        if meta.len() > self.max_bytes {
            return Err(ToolError::Failed(format!("{} is {} bytes; limit is {}", path.display(), meta.len(), self.max_bytes)));
        }
        let bytes = tokio::fs::read(&path).await.map_err(|e| ToolError::Failed(format!("{}: {e}", path.display())))?;
        let mail = mailparse::parse_mail(&bytes).map_err(|e| ToolError::Failed(format!("{}: {e}", path.display())))?;
        let header = |name: &str| -> Option<String> {
            use mailparse::MailHeaderMap;
            mail.headers.get_first_value(name).filter(|v| !v.trim().is_empty())
        };
        let text = mail_text(&mail).unwrap_or_default();
        Ok(json!({
            "path": path.to_string_lossy(),
            "from": header("From"),
            "to": header("To"),
            "cc": header("Cc"),
            "subject": header("Subject"),
            "date": header("Date"),
            "messageId": header("Message-ID"),
            "inReplyTo": header("In-Reply-To"),
            "text": text.trim(),
        }))
    }
}

/// `email.write_draft`: write a reply as an RFC 822 draft file that a mail client can open.
/// Inputs `dir`, `to`, `subject`, `body`, optional `inReplyTo`, `from`. Output `{ path, bytes }`.
pub struct EmailWriteDraft;

#[async_trait::async_trait]
impl Tool for EmailWriteDraft {
    fn id(&self) -> &str {
        "email.write_draft"
    }

    fn cacheable(&self) -> bool {
        false
    }

    async fn call(&self, ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let get = |k: &str| inputs.get(k).and_then(Value::as_str).map(str::to_string);
        let dir = get("dir").ok_or_else(|| ToolError::InvalidInput("dir (string) is required".into()))?;
        let to = get("to").ok_or_else(|| ToolError::InvalidInput("to (string) is required".into()))?;
        let subject = get("subject").unwrap_or_default();
        let body = get("body").ok_or_else(|| ToolError::InvalidInput("body (string) is required".into()))?;
        let mut headers = vec![
            ("To", to.clone()),
            ("Subject", subject.clone()),
            ("Date", chrono::Utc::now().to_rfc2822()),
            ("MIME-Version", "1.0".into()),
            ("Content-Type", "text/plain; charset=utf-8".into()),
            ("X-Unsent", "1".into()),
        ];
        if let Some(f) = get("from") {
            headers.insert(0, ("From", f));
        }
        if let Some(r) = get("inReplyTo") {
            headers.push(("In-Reply-To", r.clone()));
            headers.push(("References", r));
        }
        let mut eml = String::new();
        for (k, v) in headers {
            eml.push_str(&format!("{k}: {}\r\n", v.replace(['\r', '\n'], " ")));
        }
        eml.push_str("\r\n");
        eml.push_str(&body.replace("\r\n", "\n").replace('\n', "\r\n"));
        eml.push_str("\r\n");
        let name = format!("{}.eml", safe_file_name(if subject.is_empty() { "reply" } else { &subject }));
        let mut path = ctx.check_write(&format!("{}/{}", dir.trim_end_matches('/'), name))?;
        let mut n = 1;
        while path.exists() {
            n += 1;
            path.set_file_name(format!("{} ({n}).eml", safe_file_name(if subject.is_empty() { "reply" } else { &subject })));
        }
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| ToolError::Failed(format!("{}: {e}", parent.display())))?;
        }
        tokio::fs::write(&path, &eml).await.map_err(|e| ToolError::Failed(format!("{}: {e}", path.display())))?;
        Ok(json!({ "path": path.to_string_lossy(), "bytes": eml.len(), "to": to, "subject": subject }))
    }
}

/// `list.filter`: keep the items of an array where `path` (dot path inside each item) is
/// present and truthy. Output `{ items, kept, dropped }`.
pub struct ListFilter;

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::Object(_)) => true,
    }
}

/// Follow a dot path inside a value.
pub fn dig<'v>(v: &'v Value, path: &str) -> Option<&'v Value> {
    let mut cur = v;
    for p in path.split('.').filter(|p| !p.is_empty()) {
        cur = match cur {
            Value::Object(m) => m.get(p)?,
            Value::Array(a) => a.get(p.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

#[async_trait::async_trait]
impl Tool for ListFilter {
    fn id(&self) -> &str {
        "list.filter"
    }

    async fn call(&self, _ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let items = inputs
            .get("items")
            .and_then(Value::as_array)
            .ok_or_else(|| ToolError::InvalidInput("items (array) is required".into()))?;
        let path = inputs
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("path (string) is required".into()))?;
        let kept: Vec<Value> = items.iter().filter(|it| truthy(dig(it, path))).cloned().collect();
        let dropped = items.len() - kept.len();
        Ok(json!({ "items": kept, "kept": kept.len(), "dropped": dropped }))
    }
}

/// `list.table`: flatten an array of nested objects into rows. Inputs `items` and `columns`
/// (`{ "Column name": "dot.path" }`, in display order). Output `{ rows: [...], count }` where
/// each row has exactly the named columns. Pairs with the `table` output type.
pub struct ListTable;

#[async_trait::async_trait]
impl Tool for ListTable {
    fn id(&self) -> &str {
        "list.table"
    }

    async fn call(&self, _ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let items = inputs
            .get("items")
            .and_then(Value::as_array)
            .ok_or_else(|| ToolError::InvalidInput("items (array) is required".into()))?;
        let columns = inputs
            .get("columns")
            .and_then(Value::as_object)
            .ok_or_else(|| ToolError::InvalidInput("columns (object of name -> path) is required".into()))?;
        let rows: Vec<Value> = items
            .iter()
            .map(|it| {
                let mut row = serde_json::Map::new();
                for (name, path) in columns {
                    let v = path.as_str().and_then(|p| dig(it, p)).cloned().unwrap_or(Value::Null);
                    row.insert(name.clone(), v);
                }
                Value::Object(row)
            })
            .collect();
        Ok(json!({ "rows": rows, "count": rows.len() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ToolContext {
        ToolContext {
            run_id: "test".into(),
            artifact_dir: std::env::temp_dir().join(format!("stillroom-tools-{}", uuid::Uuid::now_v7())),
            fs: FsPolicy::open(),
        }
    }

    const SAMPLE_EML: &str = "From: Ana Ruiz <ana@example.com>
To: me@example.com
Subject: Invoice question
Date: Mon, 26 Oct 2026 09:00:00 +0000
Message-ID: <abc@example.com>
Content-Type: multipart/alternative; boundary=\"b1\"

--b1
Content-Type: text/plain; charset=utf-8

Hi, is invoice 42 paid?
--b1
Content-Type: text/html

<p>Hi, is invoice 42 paid?</p>
--b1--
";

    #[tokio::test]
    async fn email_parse_reads_headers_and_plain_body() {
        let c = ctx();
        let dir = c.ensure_artifact_dir().unwrap().to_path_buf();
        let p = dir.join("a.eml");
        std::fs::write(&p, SAMPLE_EML).unwrap();
        let out = EmailParse::default().call(&c, json!({ "path": p.to_string_lossy() })).await.unwrap();
        assert_eq!(out["subject"], "Invoice question");
        assert_eq!(out["from"], "Ana Ruiz <ana@example.com>");
        assert_eq!(out["messageId"], "<abc@example.com>");
        assert_eq!(out["text"], "Hi, is invoice 42 paid?");
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn fs_tools_respect_the_policy() {
        let mut c = ctx();
        let dir = std::env::temp_dir().join(format!("stillroom-fs-{}", uuid::Uuid::now_v7()));
        let inbox = dir.join("inbox");
        let elsewhere = dir.join("elsewhere");
        std::fs::create_dir_all(&inbox).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(inbox.join("one.eml"), SAMPLE_EML).unwrap();
        std::fs::write(inbox.join("notes.txt"), "x").unwrap();
        c.fs = FsPolicy { read_roots: vec![normalize_path(&inbox.to_string_lossy())], write_roots: vec![normalize_path(&inbox.to_string_lossy())] };

        let listed = FsList.call(&c, json!({ "path": inbox.to_string_lossy(), "extensions": ["eml"] })).await.unwrap();
        assert_eq!(listed["count"], 1, "{listed}");
        assert_eq!(listed["entries"][0]["name"], "one.eml");
        let denied = FsList.call(&c, json!({ "path": elsewhere.to_string_lossy() })).await.unwrap_err();
        assert!(matches!(denied, ToolError::Denied(_)), "{denied}");

        let w = FsWrite.call(&c, json!({ "dir": inbox.to_string_lossy(), "name": "Re: hi/there?.md", "text": "ok" })).await.unwrap();
        assert!(w["path"].as_str().unwrap().ends_with("Re_ hi_there_.md"), "{w}");
        let again = FsWrite.call(&c, json!({ "path": w["path"], "text": "no" })).await.unwrap_err();
        assert!(again.to_string().contains("already exists"));
        let denied = FsWrite.call(&c, json!({ "dir": elsewhere.to_string_lossy(), "name": "x.md", "text": "ok" })).await.unwrap_err();
        assert!(matches!(denied, ToolError::Denied(_)));
        // The artifact dir is always writable.
        let art = FsWrite.call(&c, json!({ "path": c.artifact_dir.join("a.txt").to_string_lossy(), "text": "ok" })).await;
        assert!(art.is_ok(), "{art:?}");
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(&c.artifact_dir).ok();
    }

    #[tokio::test]
    async fn email_draft_and_list_filter() {
        let c = ctx();
        let dir = c.ensure_artifact_dir().unwrap().to_path_buf();
        let d = EmailWriteDraft
            .call(&c, json!({ "dir": dir.to_string_lossy(), "to": "ana@example.com", "subject": "Re: Invoice question", "body": "Yes, paid.\nThanks", "inReplyTo": "<abc@example.com>" }))
            .await
            .unwrap();
        let text = std::fs::read_to_string(d["path"].as_str().unwrap()).unwrap();
        assert!(text.contains("In-Reply-To: <abc@example.com>\r\n"));
        assert!(text.ends_with("Yes, paid.\r\nThanks\r\n"));
        let d2 = EmailWriteDraft.call(&c, json!({ "dir": dir.to_string_lossy(), "to": "a@b", "subject": "Re: Invoice question", "body": "x" })).await.unwrap();
        assert!(d2["path"].as_str().unwrap().ends_with("(2).eml"));

        let f = ListFilter.call(&c, json!({ "items": [{ "draft": { "a": 1 } }, { "draft": null }, { "x": 1 }], "path": "draft" })).await.unwrap();
        assert_eq!(f["kept"], 1);
        assert_eq!(f["dropped"], 2);
        std::fs::remove_dir_all(dir).ok();
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
