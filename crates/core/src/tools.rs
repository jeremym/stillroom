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

#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn id(&self) -> &str;
    async fn call(&self, inputs: Value) -> Result<Value, ToolError>;
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

    async fn call(&self, inputs: Value) -> Result<Value, ToolError> {
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

    async fn call(&self, inputs: Value) -> Result<Value, ToolError> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let out = ConceptMap.call(json!({ "spine": spine })).await.unwrap();
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
        let err = FileRead::default().call(json!({ "path": "/nonexistent/x.txt" })).await.unwrap_err();
        assert!(matches!(err, ToolError::Failed(_)));
    }
}
