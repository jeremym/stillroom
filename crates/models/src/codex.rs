//! Codex CLI as a model provider: `codex exec` run headless, one process per call.
//!
//! The call is billed to the ChatGPT plan the CLI is signed in with, not per token.
//! Each call carries Codex's own instructions (about 13k input tokens), so this suits
//! recipe steps, not chatter.
//!
//! What the sandbox does and does not do: `--sandbox read-only` applies to the commands Codex
//! itself decides to run while answering, and keeps them from writing. It does not stop those
//! commands from reading: Codex may run commands that read files on this Mac, and what they
//! read can end up in the request to OpenAI. The working folder is a scratch folder, so nothing
//! of the user's is in front of it by default, but a recipe step's prompt goes to a cloud
//! service either way; that is why this provider is off until the user turns it on, and why
//! Auto never picks it.
//!
//! What maps and what does not: messages become one prompt; `json_schema` becomes
//! `--output-schema`; `think` becomes the reasoning effort; recipe tools cannot be
//! given to Codex (it has its own), so the adapter reports no tool-calling capability
//! and refuses a request that carries tools. Temperature, max tokens, and context
//! length have no flag and are ignored.

use crate::provider::{Provider, ProviderError};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use stillroom_core::model::{ChatRequest, ChatResponse, InstalledModel};
use stillroom_core::recipe::Capability;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

pub const PROVIDER_ID: &str = "codex";

pub struct Codex {
    binary: PathBuf,
    /// Model names offered, the CLI's configured default first.
    models: Vec<String>,
}

/// What Settings shows about the CLI, whether or not the provider is turned on.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexStatus {
    pub installed: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub signed_in: bool,
    pub default_model: Option<String>,
}

impl Codex {
    /// `binary` overrides discovery; `extra_models` are names the user added by hand.
    pub fn new(binary: Option<PathBuf>, extra_models: Vec<String>) -> Self {
        let binary = binary.or_else(Self::find_binary).unwrap_or_else(|| PathBuf::from("codex"));
        let mut models: Vec<String> = Self::default_model().into_iter().collect();
        for m in extra_models {
            let m = m.trim().to_string();
            if !m.is_empty() && !models.contains(&m) {
                models.push(m);
            }
        }
        Self { binary, models }
    }

    pub fn binary(&self) -> &Path {
        &self.binary
    }

    fn home() -> Option<PathBuf> {
        std::env::var_os("HOME").map(PathBuf::from)
    }

    /// The CLI in the usual places, then on PATH. App bundles do not inherit a shell PATH.
    pub fn find_binary() -> Option<PathBuf> {
        let mut candidates = Vec::new();
        if let Some(h) = Self::home() {
            candidates.push(h.join(".local/bin/codex"));
        }
        candidates.push(PathBuf::from("/opt/homebrew/bin/codex"));
        candidates.push(PathBuf::from("/usr/local/bin/codex"));
        if let Some(path) = std::env::var_os("PATH") {
            candidates.extend(std::env::split_paths(&path).map(|p| p.join("codex")));
        }
        candidates.into_iter().find(|p| p.is_file())
    }

    /// The `model = "..."` line of `~/.codex/config.toml`, the model Codex uses when none is named.
    pub fn default_model() -> Option<String> {
        let text = std::fs::read_to_string(Self::home()?.join(".codex/config.toml")).ok()?;
        parse_default_model(&text)
    }

    pub async fn status(&self) -> CodexStatus {
        let installed = self.binary.is_file();
        let version = if installed { run_capture(&self.binary, &["--version"]).await.map(|s| s.trim().to_string()) } else { None };
        CodexStatus {
            installed,
            path: installed.then(|| self.binary.to_string_lossy().into_owned()),
            version,
            signed_in: installed && self.signed_in().await,
            default_model: Self::default_model(),
        }
    }

    async fn signed_in(&self) -> bool {
        Command::new(&self.binary)
            .args(["login", "status"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

/// Reads the top-level `model = "name"` entry, ignoring the same key inside a table.
pub fn parse_default_model(toml: &str) -> Option<String> {
    for line in toml.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            break;
        }
        if let Some(rest) = t.strip_prefix("model") {
            let rest = rest.trim_start();
            if let Some(v) = rest.strip_prefix('=') {
                return Some(v.trim().trim_matches('"').trim_matches('\'').to_string()).filter(|s| !s.is_empty());
            }
        }
    }
    None
}

/// One prompt from the transcript: system text first, then each turn labelled unless it is a single user message.
pub fn flatten_prompt(req: &ChatRequest) -> String {
    let mut system = Vec::new();
    let mut turns = Vec::new();
    for m in &req.messages {
        if m.role == "system" {
            system.push(m.content.as_str());
        } else {
            turns.push(m);
        }
    }
    let mut out = system.join("\n\n");
    if turns.len() == 1 && turns[0].role == "user" {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&turns[0].content);
    } else {
        for m in turns {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(&format!("[{}]\n{}", m.role, m.content));
        }
    }
    out
}

/// The reasoning effort for `think`: off is low, on is high, and a string is passed on only
/// when it is one of the CLI's own levels (`low`, `medium`, `high`), so a recipe cannot smuggle
/// anything else into the command line. Anything else leaves the CLI's default in place.
pub fn reasoning_effort(think: &Option<serde_json::Value>) -> Option<String> {
    match think {
        Some(serde_json::Value::Bool(false)) => Some("low".into()),
        Some(serde_json::Value::Bool(true)) => Some("high".into()),
        Some(serde_json::Value::String(s)) if matches!(s.as_str(), "low" | "medium" | "high") => Some(s.clone()),
        _ => None,
    }
}

#[derive(Default)]
struct Parsed {
    last_message: Option<String>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    error: Option<String>,
}

/// What the `--json` event stream says: the final message, token usage, any error event.
pub(crate) fn parse_events(stdout: &str) -> (Option<String>, Option<u64>, Option<u64>, Option<String>) {
    let mut p = Parsed::default();
    for line in stdout.lines() {
        let Ok(e) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        match e.get("type").and_then(|t| t.as_str()) {
            Some("item.completed") => {
                if let Some(item) = e.get("item") {
                    if item.get("type").and_then(|t| t.as_str()) == Some("agent_message") {
                        p.last_message = item.get("text").and_then(|t| t.as_str()).map(String::from);
                    }
                }
            }
            Some("turn.completed") => {
                if let Some(u) = e.get("usage") {
                    p.input_tokens = u.get("input_tokens").and_then(|v| v.as_u64());
                    let out = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    let reasoning = u.get("reasoning_output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    p.output_tokens = Some(out + reasoning);
                }
            }
            Some("error") | Some("turn.failed") => {
                p.error = e
                    .get("message")
                    .or_else(|| e.get("error").and_then(|x| x.get("message")))
                    .and_then(|m| m.as_str())
                    .map(String::from)
                    .or(Some("Codex reported an error".into()));
            }
            _ => {}
        }
    }
    (p.last_message, p.input_tokens, p.output_tokens, p.error)
}

async fn run_capture(binary: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new(binary).args(args).stdin(Stdio::null()).kill_on_drop(true).output().await.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn scratch_dir() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    std::env::temp_dir().join(format!("stillroom-codex-{}-{}-{}", std::process::id(), stamp, N.fetch_add(1, Ordering::Relaxed)))
}

#[async_trait::async_trait]
impl Provider for Codex {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn label(&self) -> &str {
        "Codex"
    }

    fn base_url(&self) -> &str {
        self.binary.to_str().unwrap_or("codex")
    }

    fn is_local(&self) -> bool {
        false
    }

    async fn is_available(&self) -> bool {
        self.binary.is_file() && self.signed_in().await
    }

    async fn list_models(&self) -> Result<Vec<InstalledModel>, ProviderError> {
        Ok(self
            .models
            .iter()
            .map(|name| InstalledModel {
                id: format!("{PROVIDER_ID}:{name}"),
                provider: PROVIDER_ID.into(),
                name: name.clone(),
                capabilities: vec![Capability::Text, Capability::StructuredOutput, Capability::Thinking],
                context_window: None,
                parameter_size: None,
                size_bytes: None,
                local: false,
            })
            .collect())
    }

    async fn chat(&self, model: &str, req: ChatRequest) -> Result<ChatResponse, ProviderError> {
        if !req.tools.is_empty() {
            return Err(ProviderError::Process("Codex steps cannot call recipe tools; use a local model for agent steps".into()));
        }
        let dir = scratch_dir();
        std::fs::create_dir_all(&dir).map_err(|e| ProviderError::Process(format!("could not make a scratch folder: {e}")))?;
        let result = self.chat_in(&dir, model, &req).await;
        let _ = std::fs::remove_dir_all(&dir);
        result
    }
}

impl Codex {
    async fn chat_in(&self, dir: &Path, model: &str, req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let last = dir.join("last.txt");
        let mut cmd = Command::new(&self.binary);
        cmd.arg("exec")
            .args(["--ephemeral", "--skip-git-repo-check", "--color", "never", "--json"])
            .args(["-s", "read-only"])
            .arg("-C")
            .arg(dir)
            .args(["-m", model])
            .arg("-o")
            .arg(&last);
        if let Some(effort) = reasoning_effort(&req.think) {
            cmd.arg("-c").arg(format!("model_reasoning_effort=\"{effort}\""));
        }
        if let Some(schema) = &req.json_schema {
            let path = dir.join("schema.json");
            std::fs::write(&path, serde_json::to_vec(schema)?).map_err(|e| ProviderError::Process(format!("could not write the schema: {e}")))?;
            cmd.arg("--output-schema").arg(&path);
        }
        cmd.arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(dir)
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| ProviderError::Process(format!("could not start {}: {e}", self.binary.display())))?;
        let prompt = flatten_prompt(req);
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(prompt.as_bytes()).await.map_err(|e| ProviderError::Process(format!("could not send the prompt to Codex: {e}")))?;
            drop(stdin);
        }
        let out = child.wait_with_output().await.map_err(|e| ProviderError::Process(format!("Codex did not finish: {e}")))?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let (last_message, prompt_tokens, completion_tokens, event_error) = parse_events(&stdout);
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let tail: String = stderr.lines().rev().take(3).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" ");
            return Err(ProviderError::Process(format!(
                "Codex exited with {}: {}",
                out.status.code().map(|c| c.to_string()).unwrap_or_else(|| "a signal".into()),
                event_error.unwrap_or(tail)
            )));
        }
        if let Some(err) = event_error {
            return Err(ProviderError::Process(format!("Codex: {err}")));
        }
        let content = std::fs::read_to_string(&last).ok().filter(|s| !s.trim().is_empty()).or(last_message).unwrap_or_default();
        if content.trim().is_empty() {
            return Err(ProviderError::Process("Codex finished without an answer".into()));
        }
        Ok(ChatResponse { content, tool_calls: Vec::new(), prompt_tokens, completion_tokens })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stillroom_core::model::ChatMessage;

    #[test]
    fn reads_the_top_level_model_only() {
        let toml = "model = \"gpt-x\"\nmodel_reasoning_effort = \"medium\"\n[projects.\"/a\"]\nmodel = \"other\"\n";
        assert_eq!(parse_default_model(toml).as_deref(), Some("gpt-x"));
        assert_eq!(parse_default_model("[a]\nmodel = \"x\"\n"), None);
    }

    #[test]
    fn flattens_system_and_single_user_without_labels() {
        let req = ChatRequest {
            messages: vec![ChatMessage::new("system", "Be brief."), ChatMessage::new("user", "Say hi.")],
            ..Default::default()
        };
        assert_eq!(flatten_prompt(&req), "Be brief.\n\nSay hi.");
        let multi = ChatRequest {
            messages: vec![ChatMessage::new("user", "a"), ChatMessage::new("assistant", "b"), ChatMessage::new("user", "c")],
            ..Default::default()
        };
        assert_eq!(flatten_prompt(&multi), "[user]\na\n\n[assistant]\nb\n\n[user]\nc");
    }

    #[test]
    fn maps_think_to_effort() {
        assert_eq!(reasoning_effort(&Some(serde_json::json!(false))).as_deref(), Some("low"));
        assert_eq!(reasoning_effort(&Some(serde_json::json!(true))).as_deref(), Some("high"));
        assert_eq!(reasoning_effort(&Some(serde_json::json!("medium"))).as_deref(), Some("medium"));
        assert_eq!(reasoning_effort(&Some(serde_json::json!("low"))).as_deref(), Some("low"));
        assert_eq!(reasoning_effort(&None), None);
        for not_a_level in ["", "max", "high; rm -rf ~", "xhigh", "Medium"] {
            assert_eq!(reasoning_effort(&Some(serde_json::json!(not_a_level))), None, "{not_a_level:?} is not passed on");
        }
        assert_eq!(reasoning_effort(&Some(serde_json::json!(3))), None);
    }

    #[test]
    fn parses_the_event_stream() {
        let s = concat!(
            "{\"type\":\"thread.started\",\"thread_id\":\"t\"}\n",
            "{\"type\":\"item.completed\",\"item\":{\"id\":\"i\",\"type\":\"agent_message\",\"text\":\"{\\\"word\\\":\\\"pong\\\"}\"}}\n",
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":13382,\"output_tokens\":15,\"reasoning_output_tokens\":4}}\n"
        );
        let (msg, inp, out, err) = parse_events(s);
        assert_eq!(msg.as_deref(), Some("{\"word\":\"pong\"}"));
        assert_eq!((inp, out), (Some(13382), Some(19)));
        assert!(err.is_none());
    }
}
