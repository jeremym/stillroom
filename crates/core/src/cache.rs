//! Selective rerun: a step whose definition, resolved inputs, model, and prompt are all
//! unchanged can reuse the output it produced last time.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedStep {
    pub run_id: String,
    pub step_id: String,
    pub model: Option<String>,
    pub output: Value,
    pub created_at: DateTime<Utc>,
}

pub trait StepCache: Send + Sync {
    fn get(&self, key: &str) -> Option<CachedStep>;
    fn put(&self, key: &str, entry: &CachedStep);
}

/// Stable hex digest over the parts that determine a step's output.
pub fn fingerprint(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p.as_bytes());
    }
    format!("{:x}", h.finalize())
}

/// In-memory cache for tests and one-off runs.
#[derive(Default)]
pub struct MemoryCache(Mutex<HashMap<String, CachedStep>>);

impl StepCache for MemoryCache {
    fn get(&self, key: &str) -> Option<CachedStep> {
        self.0.lock().ok()?.get(key).cloned()
    }
    fn put(&self, key: &str, entry: &CachedStep) {
        if let Ok(mut m) = self.0.lock() {
            m.insert(key.to_string(), entry.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_and_boundary_safe() {
        assert_eq!(fingerprint(&["a", "b"]), fingerprint(&["a", "b"]));
        assert_ne!(fingerprint(&["ab", ""]), fingerprint(&["a", "b"]));
        assert_ne!(fingerprint(&["a"]), fingerprint(&["b"]));
    }
}
