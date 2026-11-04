//! A first guess at how long a run will take, before the recipe has ever run: from the model's
//! size (loading), the words it will read (prefill), the words it will write (generation), and
//! what this Mac has managed before. Rough on purpose; the first real run replaces it.

use indexmap::IndexMap;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

use crate::recipe::{Recipe, Step};
use crate::refs;
use crate::store::{RunEstimate, StageEstimate};

/// What is known about one model on this Mac.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelFacts {
    pub size_bytes: Option<u64>,
    /// Already in memory: no load to wait for.
    pub loaded: bool,
    /// Measured on this Mac, when it has answered before.
    pub tokens_per_second: Option<f64>,
    pub prefill_tokens_per_second: Option<f64>,
    /// How long the last cold load took, in seconds.
    pub load_seconds: Option<f64>,
}

/// What this Mac is like, for the defaults when nothing is measured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Machine {
    /// A GPU Ollama can use (Apple silicon). Without one, everything is many times slower.
    pub gpu: bool,
}

/// Defaults used until a model has been measured on this Mac.
pub const SPEED_GPU: f64 = 20.0;
pub const SPEED_NO_GPU: f64 = 3.0;
pub const PREFILL_GPU: f64 = 400.0;
pub const PREFILL_NO_GPU: f64 = 50.0;
/// Weights read from an SSD into memory, bytes per second.
pub const LOAD_BYTES_PER_SECOND: f64 = 1.0e9;
/// Tokens per word, about.
pub const TOKENS_PER_WORD: f64 = 1.3;
/// Output of a model step when nothing is known: a schema answer is short, prose is longer.
pub const OUTPUT_TOKENS_SCHEMA: u64 = 300;
pub const OUTPUT_TOKENS_TEXT: u64 = 600;
/// Prompt tokens of a step whose inputs cannot be sized yet.
pub const PROMPT_TOKENS_UNKNOWN: u64 = 400;
/// Items in a loop over something not known yet.
pub const LOOP_ITEMS_UNKNOWN: u64 = 3;
/// A program step, and the slow one (speech).
pub const TOOL_MS: u64 = 1_000;
pub const SPEECH_MS: u64 = 5_000;

pub fn tokens_of_words(words: u64) -> u64 {
    (words as f64 * TOKENS_PER_WORD).round() as u64
}

/// Milliseconds one model call takes: load (if not loaded) + read + write.
pub fn call_ms(facts: &ModelFacts, machine: Machine, prompt_tokens: u64, output_tokens: u64) -> u64 {
    let load = if facts.loaded {
        0.0
    } else {
        facts.load_seconds.unwrap_or_else(|| facts.size_bytes.map(|b| b as f64 / LOAD_BYTES_PER_SECOND).unwrap_or(0.0))
    };
    let prefill = facts.prefill_tokens_per_second.filter(|x| *x > 0.0).unwrap_or(if machine.gpu { PREFILL_GPU } else { PREFILL_NO_GPU });
    let speed = facts.tokens_per_second.filter(|x| *x > 0.0).unwrap_or(if machine.gpu { SPEED_GPU } else { SPEED_NO_GPU });
    ((load + prompt_tokens as f64 / prefill + output_tokens as f64 / speed) * 1000.0).round() as u64
}

/// Everything the guess needs besides the recipe.
pub struct GuessInput<'a> {
    pub recipe: &'a Recipe,
    /// Prompt files by path, for their word counts.
    pub prompts: &'a HashMap<String, String>,
    /// The inputs typed so far, for the words they add and the lists they loop over.
    pub inputs: &'a IndexMap<String, Value>,
    /// Slot → model id, as a run would resolve them.
    pub resolutions: &'a BTreeMap<String, String>,
    /// Model id → what is known about it.
    pub models: &'a BTreeMap<String, ModelFacts>,
    pub machine: Machine,
    /// Step id → average completion tokens from earlier runs of any outcome, when there were any.
    pub history: &'a HashMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepGuess {
    pub id: String,
    pub ms: u64,
    pub tokens: u64,
}

fn words(text: &str) -> u64 {
    text.split_whitespace().count() as u64
}

/// Words a value would add to a prompt when written in.
fn value_words(v: &Value) -> u64 {
    match v {
        Value::String(s) => words(s),
        Value::Array(a) => a.iter().map(value_words).sum(),
        Value::Object(o) => o.values().map(value_words).sum(),
        Value::Null => 0,
        other => words(&other.to_string()),
    }
}

/// How many items a loop runs over, from the inputs when it loops over one, else a stand-in.
fn loop_items(over: &str, inputs: &IndexMap<String, Value>) -> u64 {
    match refs::parse_ref(over) {
        Some(Ok(refs::Ref::Input { name, .. })) => match inputs.get(&name) {
            Some(Value::Array(a)) => a.len() as u64,
            Some(Value::String(s)) if !s.trim().is_empty() => LOOP_ITEMS_UNKNOWN,
            _ => LOOP_ITEMS_UNKNOWN,
        },
        _ => LOOP_ITEMS_UNKNOWN,
    }
}

/// Prompt tokens of a model step: its prompt file plus whatever inputs it names that are typed.
fn prompt_tokens(step_inputs: &IndexMap<String, Value>, prompt: &str, g: &GuessInput) -> u64 {
    let file = g.prompts.get(prompt).map(|t| words(t)).unwrap_or(0);
    let mut typed = 0u64;
    let mut unknown = false;
    for v in step_inputs.values() {
        match v.as_str().and_then(refs::parse_ref) {
            Some(Ok(refs::Ref::Input { name, .. })) => match g.inputs.get(&name) {
                Some(val) if !val.is_null() => typed += value_words(val),
                _ => unknown = true,
            },
            Some(_) => unknown = true,
            None => typed += value_words(v),
        }
    }
    tokens_of_words(file + typed) + if unknown { PROMPT_TOKENS_UNKNOWN } else { 0 }
}

fn guess_step(step: &Step, times: u64, g: &GuessInput, loaded: &mut BTreeMap<String, bool>) -> Vec<StepGuess> {
    match step {
        Step::Model(m) => {
            let model_id = g.resolutions.get(&m.slot).cloned().unwrap_or_default();
            let mut facts = g.models.get(&model_id).cloned().unwrap_or_default();
            facts.loaded = *loaded.get(&model_id).unwrap_or(&facts.loaded);
            let output = g.history.get(&m.id).copied().unwrap_or(if m.output_schema.is_some() { OUTPUT_TOKENS_SCHEMA } else { OUTPUT_TOKENS_TEXT });
            let prompt = prompt_tokens(&m.inputs, &m.prompt, g);
            // The model loads once; every later call finds it in memory.
            let first = call_ms(&facts, g.machine, prompt, output);
            loaded.insert(model_id.clone(), true);
            facts.loaded = true;
            let rest = call_ms(&facts, g.machine, prompt, output) * times.saturating_sub(1);
            vec![StepGuess { id: m.id.clone(), ms: first + rest, tokens: (prompt + output) * times }]
        }
        Step::Agent(a) => {
            let model_id = g.resolutions.get(&a.slot).cloned().unwrap_or_default();
            let mut facts = g.models.get(&model_id).cloned().unwrap_or_default();
            facts.loaded = *loaded.get(&model_id).unwrap_or(&facts.loaded);
            let output = g.history.get(&a.id).copied().unwrap_or(OUTPUT_TOKENS_TEXT);
            let prompt = prompt_tokens(&a.inputs, &a.prompt, g);
            // Two turns on average: a tool call and an answer.
            let first = call_ms(&facts, g.machine, prompt, output);
            loaded.insert(model_id.clone(), true);
            facts.loaded = true;
            let more = call_ms(&facts, g.machine, prompt, output) * (2 * times).saturating_sub(1);
            vec![StepGuess { id: a.id.clone(), ms: first + more, tokens: (prompt + output) * 2 * times }]
        }
        Step::Map(m) => {
            let items = loop_items(&m.over, g.inputs) * times;
            let inner: Vec<StepGuess> = m.steps.iter().flat_map(|s| guess_step(s, items, g, loaded)).collect();
            vec![StepGuess { id: m.id.clone(), ms: inner.iter().map(|s| s.ms).sum(), tokens: inner.iter().map(|s| s.tokens).sum() }]
        }
        Step::Tool(t) => vec![StepGuess { id: t.id.clone(), ms: if t.tool == "speech.synthesize" { SPEECH_MS } else { TOOL_MS } * times, tokens: 0 }],
        Step::Review(r) => vec![StepGuess { id: r.id.clone(), ms: 0, tokens: 0 }],
        Step::Recipe(o) | Step::Branch(o) => vec![StepGuess { id: o.id.clone(), ms: TOOL_MS * times, tokens: 0 }],
    }
}

/// The guess for a whole run, whole and by stage, marked as a guess.
pub fn guess_run(g: &GuessInput) -> RunEstimate {
    let mut loaded = BTreeMap::new();
    let steps: Vec<StepGuess> = g.recipe.steps.iter().flat_map(|s| guess_step(s, 1, g, &mut loaded)).collect();
    let by_id: HashMap<&str, &StepGuess> = steps.iter().map(|s| (s.id.as_str(), s)).collect();
    let stages = g
        .recipe
        .stages
        .iter()
        .filter(|st| !st.steps.is_empty())
        .map(|st| {
            let mine: Vec<&StepGuess> = st.steps.iter().filter_map(|id| by_id.get(id.as_str()).copied()).collect();
            StageEstimate {
                id: st.id.clone(),
                based_on_runs: 0,
                avg_duration_ms: mine.iter().map(|s| s.ms).sum(),
                avg_tokens: mine.iter().map(|s| s.tokens).sum(),
                once: st.keep.as_ref().map(|k| k.refresh != crate::recipe::Refresh::Always).unwrap_or(false),
            }
        })
        .collect();
    let tokens: u64 = steps.iter().map(|s| s.tokens).sum();
    RunEstimate {
        based_on_runs: 0,
        avg_prompt_tokens: tokens * 2 / 3,
        avg_completion_tokens: tokens / 3,
        avg_duration_ms: steps.iter().map(|s| s.ms).sum(),
        stages,
        guessed: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(size_gb: f64, loaded: bool) -> ModelFacts {
        ModelFacts { size_bytes: Some((size_gb * 1e9) as u64), loaded, tokens_per_second: None, prefill_tokens_per_second: None, load_seconds: None }
    }

    #[test]
    fn a_call_is_load_plus_read_plus_write_with_the_defaults_for_the_machine() {
        let gpu = Machine { gpu: true };
        // 4 GB to load (4 s), 400 tokens to read (1 s), 600 to write (30 s).
        assert_eq!(call_ms(&facts(4.0, false), gpu, 400, 600), 35_000);
        // Already loaded: no load.
        assert_eq!(call_ms(&facts(4.0, true), gpu, 400, 600), 31_000);
        // Measured speeds win over the defaults.
        let measured = ModelFacts { tokens_per_second: Some(60.0), prefill_tokens_per_second: Some(800.0), load_seconds: Some(1.5), ..facts(4.0, false) };
        assert_eq!(call_ms(&measured, gpu, 400, 600), 12_000);
        // No GPU: many times slower.
        assert_eq!(call_ms(&facts(4.0, true), Machine { gpu: false }, 400, 600), 208_000);
    }

    #[test]
    fn a_recipe_guess_loads_each_model_once_and_sizes_loops_from_the_inputs() {
        let recipe = Recipe::from_yaml(
            r#"
schemaVersion: "0"
id: g
name: G
inputs:
  urls: { type: urls }
  focus: { type: text }
modelSlots:
  reader: { requires: [text] }
steps:
  - id: pages
    kind: map
    over: "${inputs.urls}"
    steps:
      - id: fetch
        kind: tool
        tool: http.fetch
        inputs: { url: "${item}" }
      - id: read
        kind: model
        slot: reader
        prompt: prompts/read.md
        inputs: { focus: "${inputs.focus}", text: "${steps.fetch.output.text}" }
        outputSchema: { type: object }
  - id: brief
    kind: model
    slot: reader
    prompt: prompts/brief.md
    inputs: { pages: "${steps.pages.output}" }
outputs:
  brief: { from: "${steps.brief.output}", type: markdown }
stages:
  - { id: read, name: Read, steps: [pages] }
  - { id: write, name: Write, steps: [brief] }
"#,
        )
        .unwrap();
        let prompts = HashMap::from([("prompts/read.md".to_string(), "one two three four five".to_string()), ("prompts/brief.md".to_string(), "write it".to_string())]);
        let inputs: IndexMap<String, Value> = IndexMap::from([("urls".to_string(), serde_json::json!(["https://a", "https://b"])), ("focus".to_string(), serde_json::json!("graph theory basics"))]);
        let resolutions = BTreeMap::from([("reader".to_string(), "ollama:q".to_string())]);
        let models = BTreeMap::from([("ollama:q".to_string(), ModelFacts { tokens_per_second: Some(100.0), prefill_tokens_per_second: Some(1000.0), load_seconds: Some(2.0), ..facts(8.0, false) })]);
        let history = HashMap::from([("brief".to_string(), 1000u64)]);
        let g = GuessInput { recipe: &recipe, prompts: &prompts, inputs: &inputs, resolutions: &resolutions, models: &models, machine: Machine { gpu: true }, history: &history };
        let e = guess_run(&g);
        assert!(e.guessed);
        assert_eq!(e.stages.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["read", "write"]);
        // read: 2 items. Per item: fetch 1 s; the model reads (5 + 3 words) * 1.3 = 10 tokens + 400 for the
        // unsized step output, 410 tokens at 1000/s = 0.41 s, writes 300 at 100/s = 3 s. First call loads 2 s.
        let read = &e.stages[0];
        assert_eq!(read.avg_duration_ms, 2 * 1000 + (2000 + 410 + 3000) + (410 + 3000));
        // write: the model is loaded now; 2 words -> 3 tokens + 400 unsized, 1000 tokens from history at 100/s.
        let write = &e.stages[1];
        assert_eq!(write.avg_duration_ms, 403 + 10_000);
        assert_eq!(e.avg_duration_ms, read.avg_duration_ms + write.avg_duration_ms);
    }
}
