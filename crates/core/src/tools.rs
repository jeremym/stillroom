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
#[derive(Clone)]
pub struct ToolContext {
    pub run_id: String,
    /// Directory this run may write artifact files into. Created on demand.
    pub artifact_dir: PathBuf,
    pub fs: FsPolicy,
    pub net: NetPolicy,
    /// The run keeps this step may reach: its own stage's to write, earlier stages' to read.
    pub keeps: KeepAccess,
    /// The run's models, for tools that embed text; and which model each slot resolved to.
    pub models: Option<Arc<dyn crate::model::ModelClient>>,
    pub resolutions: HashMap<String, String>,
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext").field("run_id", &self.run_id).field("artifact_dir", &self.artifact_dir).field("resolutions", &self.resolutions).finish()
    }
}

impl ToolContext {
    /// The model behind a slot named in a tool's inputs, for tools that embed.
    fn model_for(&self, inputs: &Value) -> Result<(Arc<dyn crate::model::ModelClient>, String), ToolError> {
        let slot = inputs.get("slot").and_then(Value::as_str).ok_or_else(|| ToolError::InvalidInput("slot (the model slot to embed with) is required".into()))?;
        let id = self.resolutions.get(slot).cloned().ok_or_else(|| ToolError::Failed(format!("slot “{slot}” has no model; declare it in modelSlots with requires: [embedding]")))?;
        let models = self.models.clone().ok_or_else(|| ToolError::Failed("no models are available to this run".into()))?;
        Ok((models, id))
    }
}

/// Which run keeps a step can see. `own` is the stage the step belongs to (writable when that
/// stage has a keep); `all` holds every keep opened for the run, by stage id.
#[derive(Debug, Clone, Default)]
pub struct KeepAccess {
    pub own: Option<String>,
    pub all: HashMap<String, Arc<crate::keep::Keep>>,
}

impl KeepAccess {
    /// The keep a step reads: its own stage's by default, or an earlier stage's by name.
    fn reading(&self, stage: Option<&str>) -> Result<Arc<crate::keep::Keep>, ToolError> {
        let id = stage.map(String::from).or_else(|| self.own.clone()).ok_or_else(|| ToolError::Failed("this step's stage has no run keep; give the stage a keep, or name a stage that has one".into()))?;
        self.all.get(&id).cloned().ok_or_else(|| ToolError::Failed(format!("stage “{id}” has no run keep")))
    }
    /// The keep a step writes: only its own stage's.
    fn writing(&self) -> Result<Arc<crate::keep::Keep>, ToolError> {
        let id = self.own.clone().ok_or_else(|| ToolError::Failed("this step's stage has no run keep to write to".into()))?;
        self.all.get(&id).cloned().ok_or_else(|| ToolError::Failed(format!("stage “{id}” has no run keep")))
    }
}

fn keep_err(e: crate::keep::KeepError) -> ToolError {
    ToolError::Failed(e.to_string())
}

/// `doc.text`: the text of a document. PDF (text layer), DOCX, Markdown, or plain text.
/// Output `{ path, kind, text, chars, pages: [{ number, text }] }`; a PDF with no text layer
/// comes back with `text: ""` and a `note`, never an error.
pub struct DocText {
    pub max_bytes: u64,
}

impl Default for DocText {
    fn default() -> Self {
        Self { max_bytes: 100 * 1024 * 1024 }
    }
}

fn docx_text(bytes: &[u8]) -> Result<String, String> {
    use std::io::Read;
    let cursor = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| e.to_string())?;
    let mut file = zip.by_name("word/document.xml").map_err(|_| "not a Word document (no word/document.xml)".to_string())?;
    let mut xml = String::new();
    file.read_to_string(&mut xml).map_err(|e| e.to_string())?;
    // Paragraph and break ends become newlines; every other tag goes; entities are decoded.
    let mut out = String::with_capacity(xml.len() / 2);
    let mut rest = xml.as_str();
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('>') else { break };
        let tag = &rest[i + 1..i + j];
        if tag.starts_with("/w:p") || tag.starts_with("w:br") || tag.starts_with("w:cr") {
            out.push('\n');
        } else if tag.starts_with("w:tab") {
            out.push('\t');
        }
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    let decoded = out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'");
    Ok(decoded.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim().to_string())
}

#[async_trait::async_trait]
impl Tool for DocText {
    fn id(&self) -> &str {
        "doc.text"
    }
    fn cacheable(&self) -> bool {
        false
    }
    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "Read a document as text: a PDF (its text layer), a Word document (.docx), Markdown, or plain text. Pages are kept for PDFs.".into(),
            returns: "`{ path, kind, text, chars, pages: [{ number, text }], note }`; a scanned PDF with no text layer gives an empty `text` and a `note` saying so.".into(),
            parameters: json!({ "type": "object", "required": ["path"], "properties": { "path": { "type": "string", "description": "Absolute path of the document" } } }),
        })
    }
    async fn call(&self, ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let raw = inputs.get("path").and_then(Value::as_str).ok_or_else(|| ToolError::InvalidInput("path (string) is required".into()))?;
        let path = ctx.check_read(raw)?;
        let meta = std::fs::metadata(&path).map_err(|e| ToolError::Failed(format!("{}: {e}", path.display())))?;
        if meta.len() > self.max_bytes {
            return Err(ToolError::Failed(format!("{} is larger than {} bytes", path.display(), self.max_bytes)));
        }
        let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
        let (kind, text, pages, note): (&str, String, Vec<Value>, Option<String>) = match ext.as_str() {
            "pdf" => {
                let bytes = std::fs::read(&path).map_err(|e| ToolError::Failed(e.to_string()))?;
                match tokio::task::spawn_blocking(move || pdf_extract::extract_text_from_mem(&bytes)).await {
                    Ok(Ok(t)) => {
                        let pages: Vec<Value> = t.split('\u{c}').enumerate().filter(|(_, p)| !p.trim().is_empty()).map(|(i, p)| json!({ "number": i + 1, "text": p.trim() })).collect();
                        let text = pages.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n\n");
                        let note = text.trim().is_empty().then(|| "This PDF has no text layer (a scan, or images only); nothing to read without vision.".to_string());
                        ("pdf", text, pages, note)
                    }
                    Ok(Err(e)) => ("pdf", String::new(), Vec::new(), Some(format!("The PDF could not be read: {e}"))),
                    Err(e) => return Err(ToolError::Failed(e.to_string())),
                }
            }
            "docx" => {
                let bytes = std::fs::read(&path).map_err(|e| ToolError::Failed(e.to_string()))?;
                let text = docx_text(&bytes).map_err(ToolError::Failed)?;
                ("docx", text, Vec::new(), None)
            }
            "md" | "markdown" => ("markdown", std::fs::read_to_string(&path).map_err(|e| ToolError::Failed(e.to_string()))?, Vec::new(), None),
            _ => ("text", std::fs::read_to_string(&path).map_err(|e| ToolError::Failed(format!("{}: {e} (not a text file?)", path.display())))?, Vec::new(), None),
        };
        let pages = if pages.is_empty() && !text.is_empty() { vec![json!({ "number": 1, "text": text })] } else { pages };
        Ok(json!({ "path": path.to_string_lossy(), "kind": kind, "chars": text.chars().count(), "text": text, "pages": pages, "note": note }))
    }
}

/// `text.chunk`: split text into overlapping passages. Inputs `text` or `pages` (from
/// `doc.text`), `size` (characters, default 1500), `overlap` (default 200). Output
/// `{ chunks: [{ index, text, page }], count }`.
pub struct TextChunk;

fn split_chunks(text: &str, size: usize, overlap: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let size = size.max(100);
    let overlap = overlap.min(size / 2);
    let mut out = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let mut end = (start + size).min(chars.len());
        // Prefer to end at a paragraph or sentence boundary inside the last quarter.
        if end < chars.len() {
            let floor = start + size * 3 / 4;
            if let Some(cut) = (floor..end).rev().find(|&i| chars[i] == '\n' || chars[i] == '.' || chars[i] == '!' || chars[i] == '?') {
                end = cut + 1;
            }
        }
        let piece: String = chars[start..end].iter().collect();
        if !piece.trim().is_empty() {
            out.push(piece.trim().to_string());
        }
        if end >= chars.len() {
            break;
        }
        start = end.saturating_sub(overlap).max(start + 1);
    }
    out
}

#[async_trait::async_trait]
impl Tool for TextChunk {
    fn id(&self) -> &str {
        "text.chunk"
    }
    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "Split text into overlapping passages of about `size` characters, ending at sentence or paragraph breaks where possible. Give `pages` from doc.text to keep page numbers.".into(),
            returns: "`{ chunks: [{ index, text, page }], count }`.".into(),
            parameters: json!({ "type": "object", "properties": { "text": { "type": "string" }, "pages": { "type": "array", "description": "[{ number, text }] from doc.text; used instead of text when given" }, "size": { "type": "integer", "description": "Characters per passage, default 1500" }, "overlap": { "type": "integer", "description": "Characters shared by neighbours, default 200" } } }),
        })
    }
    async fn call(&self, _ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let size = inputs.get("size").and_then(Value::as_u64).unwrap_or(1500) as usize;
        let overlap = inputs.get("overlap").and_then(Value::as_u64).unwrap_or(200) as usize;
        let mut chunks = Vec::new();
        match inputs.get("pages").and_then(Value::as_array) {
            Some(pages) if !pages.is_empty() => {
                for p in pages {
                    let page = p.get("number").cloned().unwrap_or(Value::Null);
                    for piece in split_chunks(p.get("text").and_then(Value::as_str).unwrap_or(""), size, overlap) {
                        chunks.push(json!({ "index": chunks.len(), "text": piece, "page": page }));
                    }
                }
            }
            _ => {
                let text = inputs.get("text").and_then(Value::as_str).ok_or_else(|| ToolError::InvalidInput("text or pages is required".into()))?;
                for piece in split_chunks(text, size, overlap) {
                    chunks.push(json!({ "index": chunks.len(), "text": piece, "page": Value::Null }));
                }
            }
        }
        Ok(json!({ "count": chunks.len(), "chunks": chunks }))
    }
}

/// `keep.index`: embed passages and store them in this stage's keep so `keep.search` can find
/// them by meaning. Inputs `slot` (a model slot with the embedding capability), `chunks`
/// (`[{ text, page?, index? }]`), `source` (file path or name the passages came from), optional
/// `data` (extra fields kept on every record). A passage already stored with the same text is
/// not embedded again. Output `{ indexed, skipped, source }`.
pub struct KeepIndex;

#[async_trait::async_trait]
impl Tool for KeepIndex {
    fn id(&self) -> &str {
        "keep.index"
    }
    fn cacheable(&self) -> bool {
        false
    }
    fn has_side_effects(&self) -> bool {
        true
    }
    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "Turn passages into numbers (embeddings) and store them in this stage's run keep, so keep.search can find them by meaning later. Needs a model slot with the embedding capability. Passages already stored unchanged are skipped.".into(),
            returns: "`{ indexed, skipped, source }`.".into(),
            parameters: json!({ "type": "object", "required": ["slot", "chunks", "source"], "properties": { "slot": { "type": "string", "description": "A model slot with requires: [embedding]" }, "chunks": { "type": "array", "description": "[{ text, page, index }] from text.chunk" }, "source": { "type": "string", "description": "Where the passages came from (a file path), kept on each record" }, "data": { "type": "object", "description": "Extra fields kept on every record" } } }),
        })
    }
    async fn call(&self, ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let keep = ctx.keeps.writing()?;
        let (models, model_id) = ctx.model_for(&inputs)?;
        let source = inputs.get("source").and_then(Value::as_str).ok_or_else(|| ToolError::InvalidInput("source is required".into()))?.to_string();
        let chunks = inputs.get("chunks").and_then(Value::as_array).ok_or_else(|| ToolError::InvalidInput("chunks (array) is required".into()))?;
        let extra = inputs.get("data").and_then(Value::as_object).cloned().unwrap_or_default();
        let mut todo: Vec<(String, Value, String)> = Vec::new();
        let mut skipped = 0usize;
        for (i, c) in chunks.iter().enumerate() {
            let text = c.get("text").and_then(Value::as_str).unwrap_or("").to_string();
            if text.trim().is_empty() {
                continue;
            }
            let index = c.get("index").and_then(Value::as_u64).unwrap_or(i as u64);
            let id = format!("{source}#{index}");
            let mut data = serde_json::Map::new();
            data.insert("text".into(), Value::String(text.clone()));
            data.insert("source".into(), Value::String(source.clone()));
            data.insert("page".into(), c.get("page").cloned().unwrap_or(Value::Null));
            data.insert("index".into(), Value::from(index));
            for (k, v) in &extra {
                data.insert(k.clone(), v.clone());
            }
            let data = Value::Object(data);
            if keep.has_same(&id, &data).map_err(keep_err)? {
                skipped += 1;
                continue;
            }
            todo.push((id, data, text));
        }
        let mut indexed = 0usize;
        for batch in todo.chunks(32) {
            let texts: Vec<String> = batch.iter().map(|(_, _, t)| t.clone()).collect();
            let vectors = models.embed(&model_id, texts).await.map_err(|e| ToolError::Failed(e.to_string()))?;
            if vectors.len() != batch.len() {
                return Err(ToolError::Failed(format!("the embedding model returned {} vectors for {} passages", vectors.len(), batch.len())));
            }
            for ((id, data, _), vec) in batch.iter().zip(vectors) {
                keep.put(id, data, &ctx.run_id).map_err(keep_err)?;
                keep.put_vector(id, &model_id, &vec).map_err(keep_err)?;
                indexed += 1;
            }
        }
        Ok(json!({ "indexed": indexed, "skipped": skipped, "source": source }))
    }
}

/// `keep.search`: the passages nearest in meaning to a query, from this stage's keep or an
/// earlier kept stage named by `stage`. Output `{ passages: [{ id, text, source, page, score }], count }`.
pub struct KeepSearch;

#[async_trait::async_trait]
impl Tool for KeepSearch {
    fn id(&self) -> &str {
        "keep.search"
    }
    fn cacheable(&self) -> bool {
        false
    }
    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "Find the passages nearest in meaning to a question, from a run keep built with keep.index: this stage's, or an earlier stage's named by `stage`. Needs the same embedding slot the index was built with.".into(),
            returns: "`{ passages: [{ id, text, source, page, score }], count }`, best first.".into(),
            parameters: json!({ "type": "object", "required": ["slot", "query"], "properties": { "slot": { "type": "string" }, "query": { "type": "string" }, "limit": { "type": "integer", "description": "Default 8" }, "stage": { "type": "string", "description": "An earlier stage whose keep to search" } } }),
        })
    }
    async fn call(&self, ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let keep = ctx.keeps.reading(inputs.get("stage").and_then(Value::as_str))?;
        let (models, model_id) = ctx.model_for(&inputs)?;
        let query = inputs.get("query").and_then(Value::as_str).ok_or_else(|| ToolError::InvalidInput("query is required".into()))?.to_string();
        let limit = inputs.get("limit").and_then(Value::as_u64).unwrap_or(8) as usize;
        let mut vectors = models.embed(&model_id, vec![query]).await.map_err(|e| ToolError::Failed(e.to_string()))?;
        let q = vectors.pop().unwrap_or_default();
        let hits = keep.search(&q, limit).map_err(keep_err)?;
        let passages: Vec<Value> = hits
            .into_iter()
            .map(|(r, score)| json!({ "id": r.id, "text": r.data.get("text").cloned().unwrap_or(Value::Null), "source": r.data.get("source").cloned().unwrap_or(Value::Null), "page": r.data.get("page").cloned().unwrap_or(Value::Null), "score": score }))
            .collect();
        Ok(json!({ "count": passages.len(), "passages": passages }))
    }
}

/// `keep.put`: write or replace one record in this stage's run keep. Inputs `data` (any JSON),
/// optional `id` (default: a new id). Output `{ id, replaced }`.
pub struct KeepPut;

#[async_trait::async_trait]
impl Tool for KeepPut {
    fn id(&self) -> &str {
        "keep.put"
    }
    fn cacheable(&self) -> bool {
        false
    }
    fn has_side_effects(&self) -> bool {
        true
    }
    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "Write or replace one record in this stage's run keep (what the stage remembers between runs). Give an id to replace a record later; leave it out for a new one.".into(),
            returns: "`{ id, replaced }`; `replaced` is true when a record with that id already existed.".into(),
            parameters: json!({ "type": "object", "required": ["data"], "properties": { "id": { "type": "string" }, "data": { "description": "Any JSON: an object with the fields you will query by" } } }),
        })
    }
    async fn call(&self, ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let keep = ctx.keeps.writing()?;
        let id = inputs.get("id").and_then(Value::as_str).map(String::from).unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
        let data = inputs.get("data").cloned().unwrap_or(Value::Null);
        let replaced = keep.put(&id, &data, &ctx.run_id).map_err(keep_err)?;
        Ok(json!({ "id": id, "replaced": replaced }))
    }
}

/// `keep.get`: one record by id from this stage's keep (or an earlier stage's, by `stage`).
pub struct KeepGet;

#[async_trait::async_trait]
impl Tool for KeepGet {
    fn id(&self) -> &str {
        "keep.get"
    }
    fn cacheable(&self) -> bool {
        false
    }
    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "Read one record by id from a run keep: this stage's, or an earlier stage's named by `stage`.".into(),
            returns: "`{ found, record }`; `record` is `{ id, data, runId, writtenAt }` or null.".into(),
            parameters: json!({ "type": "object", "required": ["id"], "properties": { "id": { "type": "string" }, "stage": { "type": "string", "description": "An earlier stage whose keep to read" } } }),
        })
    }
    async fn call(&self, ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let keep = ctx.keeps.reading(inputs.get("stage").and_then(Value::as_str))?;
        let id = inputs.get("id").and_then(Value::as_str).ok_or_else(|| ToolError::Failed("keep.get needs an id".into()))?;
        let rec = keep.get(id).map_err(keep_err)?;
        Ok(json!({ "found": rec.is_some(), "record": rec }))
    }
}

/// `keep.query`: records from a keep, newest first, optionally filtered by equal top-level fields.
pub struct KeepQuery;

#[async_trait::async_trait]
impl Tool for KeepQuery {
    fn id(&self) -> &str {
        "keep.query"
    }
    fn cacheable(&self) -> bool {
        false
    }
    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "List records from a run keep, newest first: this stage's, or an earlier stage's named by `stage`. `where` keeps records whose top-level fields equal the given values.".into(),
            returns: "`{ records: [{ id, data, runId, writtenAt }], count }`.".into(),
            parameters: json!({ "type": "object", "properties": { "stage": { "type": "string" }, "where": { "type": "object", "description": "Field values to match, e.g. { \"kind\": \"news\" }" }, "limit": { "type": "integer", "description": "Default 200" }, "oldestFirst": { "type": "boolean" } } }),
        })
    }
    async fn call(&self, ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let keep = ctx.keeps.reading(inputs.get("stage").and_then(Value::as_str))?;
        let filter = inputs.get("where").and_then(Value::as_object);
        let limit = inputs.get("limit").and_then(Value::as_u64).unwrap_or(200) as usize;
        let newest = !inputs.get("oldestFirst").and_then(Value::as_bool).unwrap_or(false);
        let records = keep.query(filter, newest, limit).map_err(keep_err)?;
        Ok(json!({ "count": records.len(), "records": records }))
    }
}

/// `keep.list`: how much a keep holds and when it was built.
pub struct KeepList;

#[async_trait::async_trait]
impl Tool for KeepList {
    fn id(&self) -> &str {
        "keep.list"
    }
    fn cacheable(&self) -> bool {
        false
    }
    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "How many records a run keep holds and when it was last built: this stage's, or an earlier stage's named by `stage`.".into(),
            returns: "`{ records, builtAt, last: { runId, at, added, replaced, removed, reason } }`.".into(),
            parameters: json!({ "type": "object", "properties": { "stage": { "type": "string" } } }),
        })
    }
    async fn call(&self, ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let keep = ctx.keeps.reading(inputs.get("stage").and_then(Value::as_str))?;
        Ok(serde_json::to_value(keep.summary().map_err(keep_err)?).unwrap_or(Value::Null))
    }
}

/// `keep.remove`: delete one record from this stage's keep.
pub struct KeepRemove;

#[async_trait::async_trait]
impl Tool for KeepRemove {
    fn id(&self) -> &str {
        "keep.remove"
    }
    fn cacheable(&self) -> bool {
        false
    }
    fn has_side_effects(&self) -> bool {
        true
    }
    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "Remove one record from this stage's run keep by id.".into(),
            returns: "`{ removed }`.".into(),
            parameters: json!({ "type": "object", "required": ["id"], "properties": { "id": { "type": "string" } } }),
        })
    }
    async fn call(&self, ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let keep = ctx.keeps.writing()?;
        let id = inputs.get("id").and_then(Value::as_str).ok_or_else(|| ToolError::Failed("keep.remove needs an id".into()))?;
        Ok(json!({ "removed": keep.remove(id).map_err(keep_err)? }))
    }
}

/// Which hosts a recipe may reach over the network. Built from `permissions.network`.
#[derive(Debug, Clone, Default)]
pub struct NetPolicy {
    /// Exact hosts, or `*.example.com` for a domain and everything under it.
    pub allow: Vec<String>,
}

impl NetPolicy {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn open() -> Self {
        Self { allow: vec!["*".into()] }
    }

    pub fn allows(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.allow.iter().any(|p| {
            let p = p.to_ascii_lowercase();
            if p == "*" {
                return true;
            }
            match p.strip_prefix("*.") {
                Some(domain) => host == domain || host.ends_with(&format!(".{domain}")),
                None => host == p,
            }
        })
    }
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

    /// The parsed URL if the recipe may reach its host over http or https.
    pub fn check_url(&self, raw: &str) -> Result<url::Url, ToolError> {
        let u = url::Url::parse(raw).map_err(|e| ToolError::InvalidInput(format!("{raw}: {e}")))?;
        if !matches!(u.scheme(), "http" | "https") {
            return Err(ToolError::InvalidInput(format!("{raw}: only http and https links can be fetched")));
        }
        let host = u.host_str().ok_or_else(|| ToolError::InvalidInput(format!("{raw}: no host")))?;
        if self.net.allows(host) {
            Ok(u)
        } else {
            Err(ToolError::Denied(format!("reaching {host} is outside what this recipe may reach")))
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
    /// What the output looks like, in words, for the planner and for recipe authors.
    pub returns: String,
}

#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn id(&self) -> &str;
    /// Whether an output may be reused when the inputs are unchanged. Tools that read
    /// external state the inputs do not capture (a file's contents) say no.
    fn cacheable(&self) -> bool {
        true
    }
    /// Whether a call changes something outside the run (writes a file, saves a draft).
    /// When a run is continued, such a step is never repeated; a step that only looks at the
    /// world (lists a folder) runs again so it sees the world as it is now.
    fn has_side_effects(&self) -> bool {
        false
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
        r.register(Arc::new(HttpFetch::default()));
        r.register(Arc::new(KeepPut));
        r.register(Arc::new(KeepGet));
        r.register(Arc::new(KeepQuery));
        r.register(Arc::new(KeepList));
        r.register(Arc::new(KeepRemove));
        r.register(Arc::new(DocText::default()));
        r.register(Arc::new(TextChunk));
        r.register(Arc::new(KeepIndex));
        r.register(Arc::new(KeepSearch));
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
                    returns: d.as_ref().map(|d| d.returns.clone()).unwrap_or_default(),
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
    /// What the output looks like, in words.
    pub returns: String,
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
            returns: "`{ path, text, bytes }`; `text` is the file's contents.".into(),
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

    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "Draw a Knowledge Spine (a list of concepts with prerequisites and related concepts) as a Mermaid flowchart.".into(),
            returns: "`{ format: \"mermaid\", title, source, nodes, edges }`; `source` is the Mermaid text, which the `diagram` output type draws.".into(),
            parameters: json!({ "type": "object", "required": ["spine"], "properties": { "spine": { "type": "object", "description": "A spine: { title, concepts: [{ id, name, prerequisites: [id], related: [id] }] }" } } }),
        })
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

    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "Read text aloud into an audio file with the Mac's built-in voice.".into(),
            returns: "`{ path, format, durationSeconds, voice, chars }`; pair with the `audio` output type.".into(),
            parameters: json!({ "type": "object", "properties": { "text": { "type": "string", "description": "Plain text to read" }, "script": { "type": "object", "description": "Or a script: { chapters: [{ title, text }] }" }, "voice": { "type": "string" }, "rate": { "type": "number", "description": "Words per minute" }, "name": { "type": "string", "description": "File name without extension" } } }),
        })
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
            returns: "`{ path, entries: [{ path, name, isDir, bytes, modified }] }`.".into(),
            parameters: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string", "description": "Absolute path of the directory" },
                    "extensions": { "type": "array", "items": { "type": "string" }, "description": "Only files with these extensions, e.g. [\"md\"]; folders are then left out of the list" },
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
                if !exts.is_empty() {
                    // An extension filter means files of those kinds; folders are walked, not listed.
                    if is_dir {
                        continue;
                    }
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

    fn has_side_effects(&self) -> bool {
        true
    }

    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "Write a text file. Give either a full path, or a directory plus a file name. Parent folders are created.".into(),
            returns: "`{ path, bytes }`.".into(),
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
            returns: "`{ path, from, to, cc, subject, date, messageId, inReplyTo, text }`.".into(),
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

    fn has_side_effects(&self) -> bool {
        true
    }

    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "Write a reply as an email draft file (.eml) that a mail app can open. Nothing is sent.".into(),
            returns: "`{ path, bytes }`.".into(),
            parameters: json!({ "type": "object", "required": ["dir", "to", "subject", "body"], "properties": { "dir": { "type": "string", "description": "Folder to write into" }, "to": { "type": "string" }, "subject": { "type": "string" }, "body": { "type": "string" }, "inReplyTo": { "type": "string" }, "from": { "type": "string" } } }),
        })
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

    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "Keep only the items of a list where a field (a dot path inside each item) is present and truthy; with `absent: true`, keep the items where it is missing or false instead.".into(),
            returns: "`{ items, kept, dropped }`; `items` is the filtered list.".into(),
            parameters: json!({ "type": "object", "required": ["items", "path"], "properties": { "items": { "type": "array" }, "path": { "type": "string", "description": "Dot path inside each item, e.g. extract.keep" }, "absent": { "type": "boolean", "description": "Keep the items where the field is missing or false" } } }),
        })
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
        let absent = inputs.get("absent").and_then(Value::as_bool).unwrap_or(false);
        let kept: Vec<Value> = items.iter().filter(|it| truthy(dig(it, path)) != absent).cloned().collect();
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

    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "Turn a list of nested objects into rows with named columns, for a table output.".into(),
            returns: "`{ rows: [...], count }`; point a `table` output at `.rows`.".into(),
            parameters: json!({ "type": "object", "required": ["items", "columns"], "properties": { "items": { "type": "array" }, "columns": { "type": "object", "description": "{ \"Column name\": \"dot.path\" } in display order" } } }),
        })
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

/// `http.fetch`: GET one web page as readable text. Output `{ url, finalUrl, ok, status, contentType,
/// title, text, chars, error }`. HTTP and connection failures are reported in the output rather than
/// failing the step, so a bad link does not sink a whole research run; a host outside the recipe's
/// network permission is refused.
pub struct HttpFetch {
    pub max_bytes: u64,
    pub timeout: std::time::Duration,
}

impl Default for HttpFetch {
    fn default() -> Self {
        Self { max_bytes: 4 * 1024 * 1024, timeout: std::time::Duration::from_secs(25) }
    }
}

/// HTML to plain text: scripts and styles dropped, block ends become line breaks, tags removed,
/// common entities decoded, whitespace tidied. Also returns the `<title>`.
pub fn html_to_text(html: &str) -> (Option<String>, String) {
    let lower = html.to_ascii_lowercase();
    let title = lower.find("<title").and_then(|start| {
        let open_end = lower[start..].find('>')? + start + 1;
        let close = lower[open_end..].find("</title")? + open_end;
        Some(decode_entities(&strip_html(&html[open_end..close])))
    });
    // Drop <script>, <style>, <title>, and <noscript> bodies.
    let mut kept = String::with_capacity(html.len());
    let mut i = 0;
    while i < html.len() {
        let rest = &lower[i..];
        let next = ["<script", "<style", "<title", "<noscript"].iter().filter_map(|t| rest.find(t).map(|p| (p, *t))).min();
        match next {
            Some((p, tag)) => {
                kept.push_str(&html[i..i + p]);
                let close = format!("</{}", &tag[1..]);
                let end = lower[i + p..].find(&close).map(|e| i + p + e).and_then(|e| lower[e..].find('>').map(|g| e + g + 1)).unwrap_or(html.len());
                i = end;
            }
            None => {
                kept.push_str(&html[i..]);
                break;
            }
        }
    }
    // Block ends become line breaks so headings and paragraphs stay apart.
    let mut with_breaks = String::with_capacity(kept.len());
    let kl = kept.to_ascii_lowercase();
    let mut j = 0;
    while j < kept.len() {
        if kl[j..].starts_with('<') {
            let end = kl[j..].find('>').map(|e| j + e + 1).unwrap_or(kept.len());
            let tag = &kl[j..end];
            let block = ["</p", "<br", "</h1", "</h2", "</h3", "</h4", "</h5", "</h6", "</li", "</div", "</tr", "</blockquote", "</pre", "</section", "</article"];
            if block.iter().any(|b| tag.starts_with(b)) {
                with_breaks.push('\n');
            } else {
                with_breaks.push(' ');
            }
            j = end;
        } else {
            let ch = kept[j..].chars().next().unwrap();
            with_breaks.push(ch);
            j += ch.len_utf8();
        }
    }
    let text = decode_entities(&with_breaks);
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let l = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if l.is_empty() {
            if lines.last().map(|x| !x.is_empty()).unwrap_or(false) {
                lines.push(String::new());
            }
        } else {
            lines.push(l);
        }
    }
    while lines.last().map(|l| l.is_empty()).unwrap_or(false) {
        lines.pop();
    }
    (title.filter(|t| !t.is_empty()), lines.join("\n"))
}

fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&rsquo;", "’")
        .replace("&lsquo;", "‘")
        .replace("&rdquo;", "”")
        .replace("&ldquo;", "“")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
        .replace("&hellip;", "…")
        .replace("&amp;", "&")
}

#[async_trait::async_trait]
impl Tool for HttpFetch {
    fn id(&self) -> &str {
        "http.fetch"
    }

    fn cacheable(&self) -> bool {
        false
    }

    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "Fetch one web page (http or https) and return its readable text. Only hosts the recipe may reach.".into(),
            returns: "`{ url, finalUrl, ok, status, contentType, title, text, chars, error }`; a page that fails to load has `ok: false` and the step still succeeds.".into(),
            parameters: json!({
                "type": "object",
                "required": ["url"],
                "properties": {
                    "url": { "type": "string", "description": "The page address." },
                    "maxChars": { "type": "integer", "description": "Keep at most this much text (default 20000)." }
                }
            }),
        })
    }

    async fn call(&self, ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let raw = inputs.get("url").and_then(Value::as_str).ok_or(ToolError::InvalidInput("url is required".into()))?.trim().to_string();
        let max_chars = inputs.get("maxChars").and_then(Value::as_u64).unwrap_or(20_000) as usize;
        let mut url = ctx.check_url(&raw)?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(self.timeout)
            .user_agent("Stillroom/0.1 (local research assistant)")
            .build()
            .map_err(|e| ToolError::Failed(e.to_string()))?;
        fn soft(raw: &str, final_url: &str, status: Option<u16>, error: String) -> Value {
            json!({ "url": raw, "finalUrl": final_url, "ok": false, "status": status, "contentType": Value::Null, "title": Value::Null, "text": "", "chars": 0, "error": error })
        }
        let mut hops = 0;
        let resp = loop {
            let resp = match client.get(url.clone()).send().await {
                Ok(r) => r,
                Err(e) => return Ok(soft(&raw, url.as_str(), None, format!("could not reach {}: {e}", url.host_str().unwrap_or("?")))),
            };
            if resp.status().is_redirection() {
                hops += 1;
                if hops > 5 {
                    return Ok(soft(&raw, url.as_str(), Some(resp.status().as_u16()), "too many redirects".into()));
                }
                let Some(loc) = resp.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()) else {
                    return Ok(soft(&raw, url.as_str(), Some(resp.status().as_u16()), "redirect without a location".into()));
                };
                let next = url.join(loc).map_err(|e| ToolError::Failed(format!("bad redirect {loc}: {e}")))?;
                // Every hop must be a host the recipe may reach.
                url = ctx.check_url(next.as_str())?;
                continue;
            }
            break resp;
        };
        let status = resp.status().as_u16();
        let content_type = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(|s| s.split(';').next().unwrap_or("").trim().to_string());
        if let Some(len) = resp.content_length() {
            if len > self.max_bytes {
                return Ok(soft(&raw, url.as_str(), Some(status), format!("page is too large ({len} bytes)")));
            }
        }
        let body = match resp.bytes().await {
            Ok(b) => b,
            Err(e) => return Ok(soft(&raw, url.as_str(), Some(status), format!("could not read the page: {e}"))),
        };
        if body.len() as u64 > self.max_bytes {
            return Ok(soft(&raw, url.as_str(), Some(status), format!("page is too large ({} bytes)", body.len())));
        }
        let raw_text = String::from_utf8_lossy(&body);
        let is_html = content_type.as_deref().map(|c| c.contains("html")).unwrap_or(false) || raw_text.trim_start().starts_with('<');
        let (title, text) = if is_html {
            html_to_text(&raw_text)
        } else if content_type.as_deref().map(|c| c.starts_with("text/") || c.contains("json") || c.contains("xml")).unwrap_or(true) {
            (None, raw_text.to_string())
        } else {
            (None, String::new())
        };
        let text: String = text.chars().take(max_chars).collect();
        let ok = (200..300).contains(&status);
        Ok(json!({
            "url": raw,
            "finalUrl": url.as_str(),
            "ok": ok,
            "status": status,
            "contentType": content_type,
            "title": title,
            "text": text,
            "chars": text.chars().count(),
            "error": if ok { Value::Null } else { Value::String(format!("the server answered {status}")) },
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
            fs: FsPolicy::open(),
            net: NetPolicy::none(),
            keeps: KeepAccess::default(),
            models: None,
            resolutions: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn doc_text_reads_markdown_docx_and_pdf_and_chunks_them() {
        let dir = std::env::temp_dir().join(format!("stillroom-doc-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.md"), "# Title\n\nFirst paragraph. Second sentence!\n\nSecond paragraph.").unwrap();
        // A minimal Word document: a zip with word/document.xml.
        {
            let f = std::fs::File::create(dir.join("b.docx")).unwrap();
            let mut z = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default();
            z.start_file("word/document.xml", opts).unwrap();
            std::io::Write::write_all(&mut z, br#"<?xml version="1.0"?><w:document><w:body><w:p><w:r><w:t>Hello &amp; welcome</w:t></w:r></w:p><w:p><w:r><w:t>Second line</w:t></w:r></w:p></w:body></w:document>"#).unwrap();
            z.finish().unwrap();
        }
        // A real PDF made by the system's text-to-PDF filter, checked in as a fixture.
        std::fs::copy(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.pdf"), dir.join("c.pdf")).unwrap();
        let t = DocText::default();
        let md = t.call(&ctx(), json!({ "path": dir.join("a.md") })).await.unwrap();
        assert_eq!(md["kind"], "markdown");
        assert!(md["text"].as_str().unwrap().starts_with("# Title"));
        let docx = t.call(&ctx(), json!({ "path": dir.join("b.docx") })).await.unwrap();
        assert_eq!(docx["text"], "Hello & welcome\nSecond line");
        let pdf = t.call(&ctx(), json!({ "path": dir.join("c.pdf") })).await.unwrap();
        assert_eq!(pdf["kind"], "pdf");
        assert!(pdf["text"].as_str().unwrap().contains("Hello PDF world"), "{pdf}");
        assert_eq!(pdf["pages"][0]["number"], 1);

        let c = TextChunk.call(&ctx(), json!({ "text": "One. Two. Three. Four. Five. Six. Seven. Eight. Nine. Ten. Eleven. Twelve. Thirteen. Fourteen. Fifteen. Sixteen. Seventeen. Eighteen. Nineteen. Twenty.", "size": 100, "overlap": 10 })).await.unwrap();
        let chunks = c["chunks"].as_array().unwrap();
        assert!(chunks.len() >= 2, "{c}");
        assert!(chunks[0]["text"].as_str().unwrap().ends_with('.'), "ends at a sentence: {}", chunks[0]["text"]);
        let paged = TextChunk.call(&ctx(), json!({ "pages": [{ "number": 1, "text": "Page one text." }, { "number": 2, "text": "Page two text." }] })).await.unwrap();
        assert_eq!(paged["chunks"][1]["page"], 2);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn net_policy_matches_hosts_and_domains() {
        let p = NetPolicy { allow: vec!["example.com".into(), "*.wikipedia.org".into()] };
        assert!(p.allows("example.com"));
        assert!(!p.allows("www.example.com"));
        assert!(p.allows("en.wikipedia.org"));
        assert!(p.allows("wikipedia.org"));
        assert!(!p.allows("evil-wikipedia.org"));
        assert!(NetPolicy::open().allows("anything.test"));
        let c = ctx();
        assert!(matches!(c.check_url("https://example.com/x"), Err(ToolError::Denied(_))));
        assert!(matches!(c.check_url("ftp://example.com/x"), Err(ToolError::InvalidInput(_))));
        let c = ToolContext { net: p, ..ctx() };
        assert_eq!(c.check_url("https://en.wikipedia.org/wiki/A").unwrap().host_str(), Some("en.wikipedia.org"));
    }

    #[test]
    fn html_becomes_readable_text() {
        let html = "<html><head><title>A &amp; B</title><style>p{}</style></head><body><script>x()</script><h1>Hello</h1><p>One &lt;two&gt;</p><p>Three</p></body></html>";
        let (title, text) = html_to_text(html);
        assert_eq!(title.as_deref(), Some("A & B"));
        assert_eq!(text, "Hello\nOne <two>\nThree");
    }

    #[tokio::test]
    async fn http_fetch_refuses_hosts_outside_the_policy() {
        let err = HttpFetch::default().call(&ctx(), json!({ "url": "https://example.com/" })).await.unwrap_err();
        assert!(matches!(err, ToolError::Denied(_)), "{err}");
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
