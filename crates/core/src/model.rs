//! Provider-neutral model types and the trait the executor calls.

use crate::recipe::Capability;
use serde::{Deserialize, Serialize};

/// A model a provider has available right now, with the capabilities it can satisfy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModel {
    /// Provider-qualified id, e.g. `ollama:qwen3:32b`.
    pub id: String,
    pub provider: String,
    /// Bare model name as the provider knows it.
    pub name: String,
    pub capabilities: Vec<Capability>,
    pub context_window: Option<u64>,
    /// Provider-reported size such as "8.2B". Used as a rough quality/speed proxy.
    pub parameter_size: Option<String>,
    /// The weights on disk, in bytes, when the provider says (Ollama does). What the model needs
    /// in memory to run is about this much; see [`fits_in_memory`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    /// How the weights were shrunk ("Q4_K_M"), when the provider says. Smaller numbers are
    /// smaller and faster, and a little less sharp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantization: Option<String>,
    /// Whether the model is in memory right now, when the provider can say (Ollama can).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loaded: Option<bool>,
    /// True when the model runs on this Mac. Auto never picks a model that is not local;
    /// a cloud model is used only through a pin or a default you set.
    #[serde(default = "yes")]
    pub local: bool,
}

/// How a model's weights sit in this Mac's memory. Below [`FIT_ROOM`] of memory there is room
/// for the context and the rest of the Mac; up to [`MEMORY_CEILING`] it fits but tightly; past
/// that it swaps and crawls. Unknown when either size is unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    Room,
    Tight,
    TooLarge,
    Unknown,
}

/// The share of memory under which a model has room to spare.
pub const FIT_ROOM: f64 = 0.40;

pub fn fit_of(size_bytes: Option<u64>, memory_bytes: Option<u64>) -> Fit {
    match (size_bytes, memory_bytes) {
        (Some(size), Some(memory)) => {
            let share = size as f64 / memory as f64;
            if share <= FIT_ROOM {
                Fit::Room
            } else if share <= MEMORY_CEILING {
                Fit::Tight
            } else {
                Fit::TooLarge
            }
        }
        _ => Fit::Unknown,
    }
}

impl Fit {
    /// The verdict in words, for a reason line or a table cell.
    pub fn word(self) -> &'static str {
        match self {
            Fit::Room => "fits with room",
            Fit::Tight => "fits, but tightly",
            Fit::TooLarge => "larger than this Mac can hold comfortably",
            Fit::Unknown => "size not known",
        }
    }
}

/// Bytes as people read them: "17 GB", "850 MB".
pub fn size_word(bytes: u64) -> String {
    if bytes >= 1 << 30 {
        let gb = bytes as f64 / (1u64 << 30) as f64;
        if gb >= 10.0 { format!("{} GB", gb.round() as u64) } else { format!("{gb:.1} GB") }
    } else {
        format!("{} MB", bytes >> 20)
    }
}

/// The amber line for a model that does not fit, or `None`.
pub fn fit_notice(m: &InstalledModel, memory_bytes: Option<u64>) -> Option<String> {
    if !m.local || fit_of(m.size_bytes, memory_bytes) != Fit::TooLarge {
        return None;
    }
    let size = size_word(m.size_bytes?);
    let room = size_word(model_ceiling_bytes(memory_bytes)?);
    Some(format!("{} is {size}; this Mac holds about {room} comfortably. It will run, but slowly. Pin a smaller model, or let Auto choose.", m.name))
}

/// The amber line for a Mac without a GPU Ollama can use, or `None`.
pub fn gpu_notice(h: &Hardware) -> Option<String> {
    (h.gpu == Some(false)).then(|| "This Mac has no GPU Ollama can use, so every model runs on the CPU alone. Expect minutes where a Mac with Apple silicon takes seconds.".to_string())
}

fn yes() -> bool {
    true
}

/// The share of physical RAM a model's weights may take and still run comfortably. Above this,
/// the system swaps the weights in and out and generation falls off a cliff; below it, the model,
/// its context, the app, and the rest of the Mac fit together. 60 % leaves a 36 GB Mac about
/// 21 GB for weights, a 64 GB Mac about 38 GB.
pub const MEMORY_CEILING: f64 = 0.60;

/// Whether weights of `size_bytes` fit comfortably in `memory_bytes` of physical RAM (at most
/// [`MEMORY_CEILING`] of it). Either unknown means "treat as fits": a model without a reported
/// size (a cloud model) or a Mac whose RAM could not be read is never marked too large.
pub fn fits_in_memory(size_bytes: Option<u64>, memory_bytes: Option<u64>) -> bool {
    match (size_bytes, memory_bytes) {
        (Some(size), Some(memory)) => (size as f64) <= (memory as f64) * MEMORY_CEILING,
        _ => true,
    }
}

/// Physical RAM of this machine, read once by the shell at startup. On a Mac it is
/// `sysctl -n hw.memsize`; elsewhere, or if the read fails, `None` (unknown: every model fits).
pub fn physical_memory_bytes() -> Option<u64> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let out = std::process::Command::new("/usr/sbin/sysctl").args(["-n", "hw.memsize"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

impl InstalledModel {
    /// Parameter count in billions: from the provider's `parameter_size`, or as a
    /// fallback from a `27b` / `0.6b` token in the model name.
    pub fn parameter_billions(&self) -> Option<f64> {
        if let Some(n) = self.parameter_size.as_deref().and_then(parse_size) {
            return Some(n);
        }
        self.name
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '.')
            .find_map(parse_size)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    /// Tool calls the assistant made in this message (echoed back in the transcript).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// For `role: tool` messages, the tool that produced `content`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

impl ChatMessage {
    pub fn new(role: &str, content: impl Into<String>) -> Self {
        Self { role: role.into(), content: content.into(), tool_calls: Vec::new(), tool_name: None }
    }
}

/// A tool the model may call, described to the provider in JSON Schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    /// Tools the model may call. Empty means a plain completion.
    #[serde(default)]
    pub tools: Vec<ToolSpec>,
    /// If set, the provider is asked to constrain output to this JSON Schema.
    pub json_schema: Option<serde_json::Value>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    /// Requested context window. Providers that support it honor it.
    pub context_length: Option<u32>,
    /// Thinking control for reasoning models: `false` disables, `true` enables, or a
    /// provider-specific level such as "low". Passed through as-is.
    pub think: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatResponse {
    pub content: String,
    /// Tool calls the model wants made before it can continue.
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    /// How fast the model wrote its answer, in tokens per second, when the provider says how long
    /// generation took (Ollama does). What this Mac manages with this model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens_per_second: Option<f64>,
    /// How long loading the model into memory took, in milliseconds, when the provider says
    /// (Ollama does; zero when it was already loaded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_ms: Option<u64>,
    /// How long reading the prompt took (prefill), in milliseconds, when the provider says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_ms: Option<u64>,
}

/// Where a model call is: loading the model into memory, reading the prompt (prefill), or
/// writing the answer. The dead time before the first word is the first two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallPhase {
    Loading,
    Reading,
    Writing,
}

/// What a streaming call reports as it goes: a phase change, or a piece of text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StreamEvent<'a> {
    Phase(CallPhase),
    Text(&'a str),
}

/// Tokens per second from a count and a duration in nanoseconds; `None` when either is missing
/// or the duration is zero.
pub fn tokens_per_second(tokens: Option<u64>, nanos: Option<u64>) -> Option<f64> {
    match (tokens, nanos) {
        (Some(t), Some(ns)) if ns > 0 => Some(t as f64 / (ns as f64 / 1e9)),
        _ => None,
    }
}

/// What this Mac runs models on: the chip, its memory (shared by the CPU and GPU on Apple
/// silicon), its cores, and the room [`fits_in_memory`] allows a model. Anything that could not
/// be read is `None`; nothing here fails.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hardware {
    /// `sysctl -n machdep.cpu.brand_string`, e.g. "Apple M3 Max".
    pub chip: Option<String>,
    /// Physical RAM, `hw.memsize`.
    pub memory_bytes: Option<u64>,
    /// GPU cores, from `ioreg` (the AGXAccelerator's `gpu-core-count`) or `system_profiler`.
    pub gpu_cores: Option<u32>,
    /// CPU cores, `hw.ncpu`.
    pub cpu_cores: Option<u32>,
    /// A GPU Ollama can use: Apple silicon yes, an Intel Mac no; `None` when it could not be told.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<bool>,
    /// The most a model's weights may take and still run comfortably: [`MEMORY_CEILING`] of memory.
    pub model_ceiling_bytes: Option<u64>,
}

/// Read this Mac's hardware facts. Slow (system_profiler may run); call once and keep the answer.
/// Off a Mac every field is `None`.
pub fn detect_hardware() -> Hardware {
    if !cfg!(target_os = "macos") {
        return Hardware::default();
    }
    let memory_bytes = physical_memory_bytes();
    let gpu_cores = command_output("/usr/sbin/ioreg", &["-rc", "AGXAccelerator", "-d", "1"])
        .and_then(|s| parse_ioreg_gpu_cores(&s))
        .or_else(|| command_output("/usr/sbin/system_profiler", &["SPDisplaysDataType", "-json"]).and_then(|s| parse_profiler_gpu_cores(&s)));
    let chip = sysctl("machdep.cpu.brand_string").map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    Hardware {
        gpu: gpu_from(chip.as_deref(), gpu_cores),
        chip,
        memory_bytes,
        gpu_cores,
        cpu_cores: sysctl("hw.ncpu").and_then(|s| s.trim().parse().ok()),
        model_ceiling_bytes: model_ceiling_bytes(memory_bytes),
    }
}

/// Apple silicon has a GPU Ollama uses through Metal; an Intel Mac does not. Failing a chip
/// name, a core count says.
pub fn gpu_from(chip: Option<&str>, gpu_cores: Option<u32>) -> Option<bool> {
    match chip {
        Some(c) if c.contains("Apple") => Some(true),
        Some(c) if c.contains("Intel") => Some(false),
        _ => gpu_cores.map(|n| n > 0),
    }
}

/// The room [`fits_in_memory`] allows: [`MEMORY_CEILING`] of `memory_bytes`.
pub fn model_ceiling_bytes(memory_bytes: Option<u64>) -> Option<u64> {
    memory_bytes.map(|m| (m as f64 * MEMORY_CEILING) as u64)
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn sysctl(key: &str) -> Option<String> {
    command_output("/usr/sbin/sysctl", &["-n", key])
}

/// The GPU core count from `ioreg -rc AGXAccelerator -d 1`: the line `"gpu-core-count" = 40`.
pub fn parse_ioreg_gpu_cores(text: &str) -> Option<u32> {
    text.lines()
        .find(|l| l.contains("\"gpu-core-count\""))
        .and_then(|l| l.rsplit('=').next())
        .and_then(|v| v.trim().parse().ok())
}

/// The GPU core count from `system_profiler SPDisplaysDataType -json`: the first display
/// adapter's `sppci_cores`, a string such as "40".
pub fn parse_profiler_gpu_cores(json: &str) -> Option<u32> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    v.get("SPDisplaysDataType")?
        .as_array()?
        .iter()
        .find_map(|gpu| gpu.get("sppci_cores").and_then(|c| c.as_str()).and_then(|c| c.trim().parse().ok()))
}

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("no provider for model id {0}")]
    UnknownProvider(String),
    #[error("provider unavailable: {0}")]
    Unavailable(String),
    #[error("provider error: {0}")]
    Remote(String),
}

/// Anything that can answer a chat request for a provider-qualified model id.
#[async_trait::async_trait]
pub trait ModelClient: Send + Sync {
    async fn chat(&self, model_id: &str, req: ChatRequest) -> Result<ChatResponse, ModelError>;
    /// Like `chat`, but `on_delta` sees each piece of text as the model writes it (streaming).
    /// Clients that cannot stream call it once with the whole content.
    async fn chat_stream(&self, model_id: &str, req: ChatRequest, on_delta: &(dyn for<'a> Fn(&'a str) + Send + Sync)) -> Result<ChatResponse, ModelError> {
        let resp = self.chat(model_id, req).await?;
        if !resp.content.is_empty() {
            on_delta(&resp.content);
        }
        Ok(resp)
    }
    /// Like `chat_stream`, but `on` also hears where the call is (loading, reading, writing)
    /// before any text arrives. Clients that cannot tell say `Reading`, then `Writing` with the
    /// first text.
    async fn chat_events(&self, model_id: &str, req: ChatRequest, on: &(dyn for<'a> Fn(StreamEvent<'a>) + Send + Sync)) -> Result<ChatResponse, ModelError> {
        on(StreamEvent::Phase(CallPhase::Reading));
        let writing = std::sync::atomic::AtomicBool::new(false);
        let on_delta = |text: &str| {
            if !writing.swap(true, std::sync::atomic::Ordering::SeqCst) {
                on(StreamEvent::Phase(CallPhase::Writing));
            }
            on(StreamEvent::Text(text));
        };
        self.chat_stream(model_id, req, &on_delta).await
    }
    /// One vector per text, from an embedding model. Providers that cannot embed say so.
    async fn embed(&self, model_id: &str, _texts: Vec<String>) -> Result<Vec<Vec<f32>>, ModelError> {
        Err(ModelError::Remote(format!("{model_id} cannot make embeddings")))
    }
}

fn parse_size(s: &str) -> Option<f64> {
    let s = s.trim();
    let (num, unit) = s.split_at(s.len().checked_sub(1)?);
    let n: f64 = num.parse().ok()?;
    match unit {
        "B" | "b" => Some(n),
        "M" | "m" => Some(n / 1000.0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(name: &str, size: Option<&str>) -> InstalledModel {
        InstalledModel {
            id: format!("ollama:{name}"),
            provider: "ollama".into(),
            name: name.into(),
            capabilities: vec![],
            context_window: None,
            parameter_size: size.map(String::from),
            size_bytes: None,
            quantization: None,
            loaded: None,
            local: true,
        }
    }

    #[test]
    fn fits_in_memory_uses_the_ceiling_and_treats_unknown_as_fits() {
        let gb = 1u64 << 30;
        let mac_36 = Some(36 * gb);
        assert!(fits_in_memory(Some(20 * gb), mac_36), "20 GB on a 36 GB Mac is under 60 %");
        assert!(!fits_in_memory(Some(30 * gb), mac_36), "30 GB on a 36 GB Mac is over 60 %");
        assert!(fits_in_memory(Some(60 * gb), Some(100 * gb)), "exactly at the ceiling fits");
        assert!(fits_in_memory(None, mac_36), "a model without a size is not called too large");
        assert!(fits_in_memory(Some(30 * gb), None), "RAM unknown: treat as fits");
    }

    #[test]
    fn physical_memory_reads_on_a_mac() {
        if cfg!(target_os = "macos") {
            assert!(physical_memory_bytes().unwrap_or(0) >= 4 << 30, "a Mac has at least 4 GB");
        }
    }

    #[test]
    fn tokens_per_second_from_a_count_and_nanoseconds() {
        assert_eq!(tokens_per_second(Some(300), Some(5_000_000_000)), Some(60.0));
        assert_eq!(tokens_per_second(Some(300), Some(0)), None, "a zero duration is not a speed");
        assert_eq!(tokens_per_second(None, Some(1)), None);
        assert_eq!(tokens_per_second(Some(1), None), None);
    }

    #[test]
    fn gpu_cores_parse_from_ioreg_and_system_profiler() {
        let ioreg = concat!(
            "+-o AGXAcceleratorG15X  <class AGXAcceleratorG15X, id 0x1000002f3, registered, matched, active, busy 0 (12 ms), retain 150>\n",
            "    {\n",
            "      \"IOClass\" = \"AGXAcceleratorG15X\"\n",
            "      \"gpu-core-count\" = 40\n",
            "      \"IOMatchCategory\" = \"IODefault\"\n",
            "    }\n"
        );
        assert_eq!(parse_ioreg_gpu_cores(ioreg), Some(40));
        assert_eq!(parse_ioreg_gpu_cores("nothing here"), None);
        assert_eq!(parse_ioreg_gpu_cores("\"gpu-core-count\" = lots"), None);

        let profiler = r#"{"SPDisplaysDataType":[{"_name":"Apple M3 Ultra","spdisplays_vendor":"sppci_vendor_Apple","sppci_cores":"60","sppci_device_type":"spdisplays_gpu","sppci_model":"Apple M3 Ultra"}]}"#;
        assert_eq!(parse_profiler_gpu_cores(profiler), Some(60));
        assert_eq!(parse_profiler_gpu_cores(r#"{"SPDisplaysDataType":[{"_name":"x"}]}"#), None);
        assert_eq!(parse_profiler_gpu_cores("not json"), None);
    }

    #[test]
    fn the_model_ceiling_is_the_memory_ceiling_share() {
        assert_eq!(model_ceiling_bytes(Some(100 << 30)), Some(60 << 30));
        assert_eq!(model_ceiling_bytes(None), None);
    }

    #[test]
    fn hardware_reads_on_a_mac_and_never_fails() {
        let h = detect_hardware();
        if cfg!(target_os = "macos") {
            assert!(h.chip.as_deref().is_some_and(|c| c.contains("Apple") || c.contains("Intel")), "{h:?}");
            assert!(h.memory_bytes.unwrap_or(0) >= 4 << 30);
            assert!(h.cpu_cores.unwrap_or(0) >= 1);
            assert_eq!(h.model_ceiling_bytes, model_ceiling_bytes(h.memory_bytes));
            eprintln!("hardware on this Mac: {}", serde_json::to_string(&h).unwrap());
        } else {
            assert_eq!(h, Hardware::default());
        }
    }

    #[test]
    fn parameter_size_from_provider_or_name() {
        assert_eq!(m("x", Some("29.9B")).parameter_billions(), Some(29.9));
        assert!((m("x", Some("566.70M")).parameter_billions().unwrap() - 0.5667).abs() < 1e-9);
        assert_eq!(m("qwen3.8:27b-mlx", None).parameter_billions(), Some(27.0));
        assert_eq!(m("hf.co/unsloth/Qwen3-Coder-Next-GGUF:UD-Q4_K_XL", None).parameter_billions(), None);
    }
}
