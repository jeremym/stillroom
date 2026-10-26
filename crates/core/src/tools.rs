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
