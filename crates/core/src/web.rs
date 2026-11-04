//! `web.search`: find pages through a search service the person set up once in Settings (a
//! SearXNG instance, theirs or one they trust, speaking its JSON API). Stillroom ships no search
//! of its own and holds no key: without a service the step says so in words. The service's host
//! is reached because the person chose it; the pages found are fetched by `http.fetch` under the
//! recipe's own network permission.

use serde_json::{json, Value};
use std::sync::Arc;

use crate::tools::{Tool, ToolContext, ToolDescription, ToolError};

/// Where the search service answers, read each call so a change in Settings takes at once.
pub type SearchUrl = Arc<dyn Fn() -> Option<String> + Send + Sync>;

pub struct WebSearch {
    pub base_url: SearchUrl,
    pub timeout: std::time::Duration,
}

impl WebSearch {
    pub fn new(base_url: SearchUrl) -> Self {
        Self { base_url, timeout: std::time::Duration::from_secs(20) }
    }
}

/// The words when no service is set.
pub const NO_SERVICE: &str = "No web search service is set. In Settings › Services, give the address of a SearXNG instance (one you run, or one you trust); until then this step cannot search.";

/// The request a SearXNG instance understands: `<base>/search?q=…&format=json`.
pub fn search_url(base: &str, query: &str, count: usize, language: Option<&str>) -> Result<url::Url, String> {
    let mut u = url::Url::parse(&format!("{}/search", base.trim_end_matches('/'))).map_err(|e| format!("{base}: {e}"))?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err(format!("{base}: a search service is an http or https address"));
    }
    {
        let mut q = u.query_pairs_mut();
        q.append_pair("q", query);
        q.append_pair("format", "json");
        if let Some(l) = language {
            q.append_pair("language", l);
        }
        q.append_pair("pageno", "1");
    }
    let _ = count;
    Ok(u)
}

/// What the tool hands back from a SearXNG answer: the first `count` results, each trimmed to
/// title, link, snippet, and which engine found it.
pub fn results_from(answer: &Value, count: usize) -> Vec<Value> {
    answer
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|r| r.get("url").and_then(Value::as_str).is_some())
        .take(count)
        .map(|r| {
            json!({
                "title": r.get("title").and_then(Value::as_str).unwrap_or("").trim(),
                "url": r.get("url").and_then(Value::as_str).unwrap_or(""),
                "snippet": r.get("content").and_then(Value::as_str).unwrap_or("").trim(),
                "engine": r.get("engine").and_then(Value::as_str).unwrap_or(""),
            })
        })
        .collect()
}

#[async_trait::async_trait]
impl Tool for WebSearch {
    fn id(&self) -> &str {
        "web.search"
    }

    fn cacheable(&self) -> bool {
        false
    }

    fn describe(&self) -> Option<ToolDescription> {
        Some(ToolDescription {
            description: "Search the web for pages about a query, through the search service the user set in Settings (a SearXNG instance). Without one the step fails in words. Fetch the pages found with http.fetch; a recipe that does so declares network [\"*\"].".into(),
            returns: "`{ query, count, results: [{ title, url, snippet, engine }] }`.".into(),
            parameters: json!({
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": { "type": "string", "description": "What to search for, as a person would type it." },
                    "count": { "type": "integer", "description": "How many results to keep (default 8, at most 20)." },
                    "language": { "type": "string", "description": "A language code such as en (optional)." }
                }
            }),
        })
    }

    async fn call(&self, _ctx: &ToolContext, inputs: Value) -> Result<Value, ToolError> {
        let query = inputs.get("query").and_then(Value::as_str).map(str::trim).filter(|q| !q.is_empty()).ok_or_else(|| ToolError::InvalidInput("query is required".into()))?;
        let count = inputs.get("count").and_then(Value::as_u64).unwrap_or(8).clamp(1, 20) as usize;
        let language = inputs.get("language").and_then(Value::as_str);
        let base = (self.base_url)().filter(|b| !b.trim().is_empty()).ok_or_else(|| ToolError::Failed(NO_SERVICE.into()))?;
        let u = search_url(&base, query, count, language).map_err(ToolError::InvalidInput)?;
        let client = reqwest::Client::builder().timeout(self.timeout).user_agent("Stillroom/0.5 (local research assistant)").build().map_err(|e| ToolError::Failed(e.to_string()))?;
        let resp = client.get(u.clone()).header("Accept", "application/json").send().await.map_err(|e| ToolError::Failed(format!("the search service at {base} did not answer: {e}")))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| ToolError::Failed(e.to_string()))?;
        if !status.is_success() {
            return Err(ToolError::Failed(format!("the search service at {base} answered {status}: {}", text.chars().take(200).collect::<String>())));
        }
        let answer: Value = serde_json::from_str(&text).map_err(|_| ToolError::Failed(format!("the search service at {base} did not answer with JSON; is it a SearXNG instance with the JSON format enabled?")))?;
        let results = results_from(&answer, count);
        Ok(json!({ "query": query, "count": results.len(), "results": results }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_is_searxngs_json_search() {
        let u = search_url("https://search.example.org/", "local-first ai tools", 5, Some("en")).unwrap();
        assert_eq!(u.as_str(), "https://search.example.org/search?q=local-first+ai+tools&format=json&language=en&pageno=1");
        assert!(search_url("ftp://x", "q", 5, None).is_err());
    }

    #[test]
    fn results_are_trimmed_to_what_a_recipe_needs() {
        let answer = json!({ "results": [
            { "title": " A ", "url": "https://a.example/", "content": "about a ", "engine": "duckduckgo", "score": 9 },
            { "title": "no link", "content": "dropped" },
            { "title": "B", "url": "https://b.example/", "content": "", "engine": "brave" },
            { "title": "C", "url": "https://c.example/" }
        ] });
        let r = results_from(&answer, 2);
        assert_eq!(r, vec![json!({ "title": "A", "url": "https://a.example/", "snippet": "about a", "engine": "duckduckgo" }), json!({ "title": "B", "url": "https://b.example/", "snippet": "", "engine": "brave" })]);
    }

    #[tokio::test]
    async fn without_a_service_the_step_says_what_to_do() {
        let t = WebSearch::new(Arc::new(|| None));
        let ctx = ToolContext { run_id: "t".into(), artifact_dir: std::env::temp_dir(), fs: crate::tools::FsPolicy::open(), net: crate::tools::NetPolicy::none(), keeps: Default::default(), models: None, resolutions: Default::default() };
        let err = t.call(&ctx, json!({ "query": "x" })).await.unwrap_err();
        assert!(err.to_string().contains(NO_SERVICE));
    }
}
