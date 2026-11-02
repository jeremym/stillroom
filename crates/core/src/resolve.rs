//! Slot resolution: pinned > your default for the capability > recommended > capability resolve.
//! Decided before the run, never by a model. The planner's own slot has its own Auto, the
//! planning rank, since the largest model is rarely the best or the quickest drafter.

use crate::model::{fits_in_memory, InstalledModel};
use crate::recipe::{Capability, ModelSlot, Preference, Recipe};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedVia {
    Pinned,
    /// The user's default model for the slot's capability, from Settings.
    Default,
    Recommended,
    Capability,
}

/// What the user set in Settings › Models: a preference when a slot states none, and a default
/// model per capability. A default is used only if it is installed and satisfies the whole slot.
/// `planning` is not a setting: the shell fills it with what Try planning measured and this
/// Mac's RAM, for the planner slot's Auto.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ResolveDefaults {
    pub preference: Option<Preference>,
    pub by_capability: BTreeMap<Capability, String>,
    #[serde(skip)]
    pub planning: PlanningFacts,
}

/// What Try planning recorded for one model: how long the strip call took and whether its
/// answer checked clean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Measured {
    pub ms: u64,
    pub clean: bool,
}

/// Facts the planning rank uses that are not in Settings.
#[derive(Debug, Clone, Default)]
pub struct PlanningFacts {
    /// Model id → the last Try planning result.
    pub measured: BTreeMap<String, Measured>,
    /// Physical RAM of this Mac; `None` when unknown (every model then counts as fitting).
    pub memory_bytes: Option<u64>,
}

/// A measured clean plan under this many milliseconds puts a model ahead of every unmeasured one.
pub const MEASURED_FAST_MS: u64 = 120_000;

/// How well a model family drafts recipes in JSON, from Jeremy's runs: lower is better. Not a
/// quality judgement of the family in general; drafting a recipe is a particular job.
///
/// | rank | family |
/// |---|---|
/// | 0 | qwen3 at 14 B and up (not the coder variants) |
/// | 1 | glm-4.x flash |
/// | 2 | gpt-oss |
/// | 3 | gemma3 at 12 B and up |
/// | 4 | qwen3-coder |
/// | 5 | everything else |
pub fn family_rank(m: &InstalledModel) -> u8 {
    let name = m.name.to_ascii_lowercase();
    let billions = m.parameter_billions().unwrap_or(0.0);
    let has = |s: &str| name.contains(s);
    if has("qwen3") && has("coder") {
        4
    } else if has("qwen3") && billions >= 14.0 {
        0
    } else if has("glm") && has("flash") {
        1
    } else if has("gpt-oss") || has("gpt_oss") {
        2
    } else if (has("gemma3") || has("gemma-3")) && billions >= 12.0 {
        3
    } else {
        5
    }
}

/// Milliseconds as people read them: "48 s", "2 m 10 s".
pub fn seconds_words(ms: u64) -> String {
    let secs = (ms + 500) / 1000;
    if secs < 60 {
        format!("{secs} s")
    } else {
        format!("{} m {} s", secs / 60, secs % 60)
    }
}

/// Why a model stands where it does in the planning rank, from the facts: whether it fits in
/// memory, that it answers in a fixed shape, and what Try planning measured (or that nothing has).
pub fn planning_reason(fits: bool, measured: Option<&Measured>) -> String {
    let fit = if fits { "fits in memory" } else { "larger than this Mac can hold comfortably" };
    let shape = "answers in a fixed shape (JSON)";
    let timing = match measured {
        Some(m) if m.clean => format!("planned clean in {}", seconds_words(m.ms)),
        Some(m) => format!("the last trial plan had problems, after {}", seconds_words(m.ms)),
        None => "not yet measured".to_string(),
    };
    format!("{fit}, {shape}, {timing}")
}

/// Whether a model may be an automatic pick for the planner: local, and it can do what the
/// planner slot requires.
fn can_plan(m: &InstalledModel) -> bool {
    m.local && m.capabilities.contains(&Capability::Text) && m.capabilities.contains(&Capability::StructuredOutput)
}

/// The planner's Auto: every installed local model that could plan, best first, each with its
/// rank (1 is first) and the reason in words. The order is
/// (a) a measured clean plan under [`MEASURED_FAST_MS`] first, fastest first;
/// (b) then what fits comfortably in memory ([`fits_in_memory`]);
/// (c) then the [`family_rank`];
/// (d) then size, largest first.
pub fn rank_for_planning(models: &[InstalledModel], measured: &BTreeMap<String, Measured>, memory_bytes: Option<u64>) -> Vec<(InstalledModel, u32, String)> {
    let mut candidates: Vec<&InstalledModel> = models.iter().filter(|m| can_plan(m)).collect();
    // Billions of parameters, or, when the provider gave none, a gigabyte of weights as a stand-in.
    let size = |m: &InstalledModel| m.parameter_billions().or_else(|| m.size_bytes.map(|b| b as f64 / 1e9)).unwrap_or(0.0);
    let fast = |m: &InstalledModel| measured.get(&m.id).filter(|x| x.clean && x.ms < MEASURED_FAST_MS).map(|x| x.ms);
    candidates.sort_by(|a, b| {
        let key = |m: &InstalledModel| (fast(m).is_none(), fast(m).unwrap_or(0), !fits_in_memory(m.size_bytes, memory_bytes), family_rank(m));
        key(a).cmp(&key(b)).then_with(|| size(b).partial_cmp(&size(a)).unwrap_or(std::cmp::Ordering::Equal))
    });
    candidates
        .into_iter()
        .enumerate()
        .map(|(i, m)| {
            let reason = planning_reason(fits_in_memory(m.size_bytes, memory_bytes), measured.get(&m.id));
            (m.clone(), i as u32 + 1, reason)
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolution {
    pub slot: String,
    pub model_id: String,
    pub via: ResolvedVia,
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("slot {slot}: pinned model {model} is not installed")]
    PinnedNotInstalled { slot: String, model: String },
    #[error("slot {slot}: the pinned model {model} cannot do {}", capability_word(*.capability))]
    PinnedLacksCapability { slot: String, model: String, capability: Capability },
    #[error("slot {slot}: no installed model can do {}", capability_words(.requires))]
    NoCandidate { slot: String, requires: Vec<Capability> },
}

/// The friendly word for a capability, for messages people read.
pub fn capability_word(c: Capability) -> &'static str {
    match c {
        Capability::Text => "text",
        Capability::StructuredOutput => "structured output",
        Capability::ToolCalling => "tool calling",
        Capability::Thinking => "thinking",
        Capability::Vision => "vision",
        Capability::Embedding => "embeddings",
        Capability::TextToSpeech => "text to speech",
        Capability::SpeechToText => "speech to text",
        Capability::ImageGeneration => "image generation",
    }
}

fn capability_words(cs: &[Capability]) -> String {
    cs.iter().map(|c| capability_word(*c)).collect::<Vec<_>>().join(" and ")
}

fn satisfies(m: &InstalledModel, slot: &ModelSlot) -> Result<(), Capability> {
    for c in &slot.requires {
        if !m.capabilities.contains(c) {
            return Err(*c);
        }
    }
    Ok(())
}

fn fits_context(m: &InstalledModel, slot: &ModelSlot) -> bool {
    match (slot.min_context, m.context_window) {
        (Some(min), Some(have)) => have >= min,
        _ => true,
    }
}

/// Resolve every slot in `recipe` against the installed models and the user's pins.
pub fn resolve_slots(
    recipe: &Recipe,
    installed: &[InstalledModel],
    pins: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, Resolution>, ResolveError> {
    resolve_slots_with(recipe, installed, pins, &ResolveDefaults::default())
}

/// Resolve with the user's defaults from Settings as well as their pins.
pub fn resolve_slots_with(
    recipe: &Recipe,
    installed: &[InstalledModel],
    pins: &BTreeMap<String, String>,
    defaults: &ResolveDefaults,
) -> Result<BTreeMap<String, Resolution>, ResolveError> {
    let mut out = BTreeMap::new();
    for (name, slot) in &recipe.model_slots {
        // 1. Pinned
        if let Some(pin) = pins.get(name) {
            let m = installed
                .iter()
                .find(|m| &m.id == pin)
                .ok_or_else(|| ResolveError::PinnedNotInstalled { slot: name.clone(), model: pin.clone() })?;
            if let Err(capability) = satisfies(m, slot) {
                return Err(ResolveError::PinnedLacksCapability { slot: name.clone(), model: pin.clone(), capability });
            }
            out.insert(name.clone(), Resolution { slot: name.clone(), model_id: m.id.clone(), via: ResolvedVia::Pinned });
            continue;
        }
        // 2. The user's default for one of the slot's capabilities, if installed and suitable
        if let Some(m) = slot.requires.iter().find_map(|c| defaults.by_capability.get(c)).and_then(|id| {
            installed.iter().find(|m| &m.id == id || &m.name == id).filter(|m| satisfies(m, slot).is_ok() && fits_context(m, slot))
        }) {
            out.insert(name.clone(), Resolution { slot: name.clone(), model_id: m.id.clone(), via: ResolvedVia::Default });
            continue;
        }
        // 3. Recommended, if installed and suitable
        if let Some(rec) = &slot.recommended {
            if let Some(m) = installed.iter().find(|m| &m.id == rec || &m.name == rec) {
                if satisfies(m, slot).is_ok() && fits_context(m, slot) {
                    out.insert(name.clone(), Resolution { slot: name.clone(), model_id: m.id.clone(), via: ResolvedVia::Recommended });
                    continue;
                }
            }
        }
        // 4. Capability resolve, among local models only: a cloud model is never an automatic pick
        let mut candidates: Vec<&InstalledModel> = installed
            .iter()
            .filter(|m| m.local && satisfies(m, slot).is_ok() && fits_context(m, slot))
            .collect();
        if candidates.is_empty() {
            return Err(ResolveError::NoCandidate { slot: name.clone(), requires: slot.requires.clone() });
        }
        // The planner's own slot: the planning rank, not a size preference.
        if recipe.id == crate::planner::PLANNER_RECIPE_ID && name == crate::planner::PLANNER_SLOT {
            let pool: Vec<InstalledModel> = candidates.iter().map(|m| (*m).clone()).collect();
            if let Some((best, _, _)) = rank_for_planning(&pool, &defaults.planning.measured, defaults.planning.memory_bytes).into_iter().next() {
                out.insert(name.clone(), Resolution { slot: name.clone(), model_id: best.id, via: ResolvedVia::Capability });
                continue;
            }
        }
        let size = |m: &InstalledModel| m.parameter_billions().unwrap_or(0.0);
        match slot.preference.or(defaults.preference).unwrap_or_default() {
            Preference::Accuracy | Preference::Quality => {
                candidates.sort_by(|a, b| size(b).partial_cmp(&size(a)).unwrap_or(std::cmp::Ordering::Equal))
            }
            Preference::Speed | Preference::Cost => {
                candidates.sort_by(|a, b| size(a).partial_cmp(&size(b)).unwrap_or(std::cmp::Ordering::Equal))
            }
            Preference::Local => {} // every provider is local for now; keep provider order
        }
        out.insert(name.clone(), Resolution { slot: name.clone(), model_id: candidates[0].id.clone(), via: ResolvedVia::Capability });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_default_beats_recommended_but_not_a_pin() {
        let installed = vec![
            model("big", "27B", &[Capability::Text]),
            model("small", "4B", &[Capability::Text]),
            model("rec", "8B", &[Capability::Text]),
        ];
        let r = recipe("accuracy", Some("rec"));
        let mut defaults = ResolveDefaults::default();
        defaults.by_capability.insert(Capability::Text, "ollama:small".into());
        let res = resolve_slots_with(&r, &installed, &BTreeMap::new(), &defaults).unwrap();
        let slot = res.values().next().unwrap();
        assert_eq!(slot.model_id, "ollama:small");
        assert_eq!(slot.via, ResolvedVia::Default);
        let mut pins = BTreeMap::new();
        pins.insert(slot.slot.clone(), "ollama:big".to_string());
        let res = resolve_slots_with(&r, &installed, &pins, &defaults).unwrap();
        assert_eq!(res.values().next().unwrap().via, ResolvedVia::Pinned);
        // A default that is not installed falls through to the recommendation.
        defaults.by_capability.insert(Capability::Text, "ollama:gone".into());
        let res = resolve_slots_with(&r, &installed, &BTreeMap::new(), &defaults).unwrap();
        assert_eq!(res.values().next().unwrap().via, ResolvedVia::Recommended);
    }

    fn model(name: &str, size: &str, caps: &[Capability]) -> InstalledModel {
        InstalledModel {
            id: format!("ollama:{name}"),
            provider: "ollama".into(),
            name: name.into(),
            capabilities: caps.to_vec(),
            context_window: Some(32_000),
            parameter_size: Some(size.into()),
            size_bytes: None,
            local: true,
        }
    }

    fn recipe(pref: &str, recommended: Option<&str>) -> Recipe {
        let rec = recommended.map(|r| format!("    recommended: {r}\n")).unwrap_or_default();
        Recipe::from_yaml(&format!(
            r#"
schemaVersion: "0"
id: t
name: T
modelSlots:
  writer:
    requires: [text]
    preference: {pref}
{rec}steps:
  - id: s
    kind: model
    slot: writer
    prompt: p.md
outputs:
  o:
    from: ${{steps.s.output}}
    type: text
"#
        ))
        .unwrap()
    }

    #[test]
    fn pinned_wins_over_everything() {
        let installed = [model("small", "1B", &[Capability::Text]), model("big", "30B", &[Capability::Text])];
        let pins = BTreeMap::from([("writer".to_string(), "ollama:small".to_string())]);
        let r = resolve_slots(&recipe("accuracy", Some("ollama:big")), &installed, &pins).unwrap();
        assert_eq!(r["writer"].model_id, "ollama:small");
        assert_eq!(r["writer"].via, ResolvedVia::Pinned);
    }

    #[test]
    fn recommended_used_when_installed() {
        let installed = [model("small", "1B", &[Capability::Text]), model("big", "30B", &[Capability::Text])];
        let r = resolve_slots(&recipe("speed", Some("big")), &installed, &BTreeMap::new()).unwrap();
        assert_eq!(r["writer"].via, ResolvedVia::Recommended);
        assert_eq!(r["writer"].model_id, "ollama:big");
    }

    #[test]
    fn capability_resolve_honors_preference() {
        let installed = [model("small", "1B", &[Capability::Text]), model("big", "30B", &[Capability::Text])];
        let fast = resolve_slots(&recipe("speed", None), &installed, &BTreeMap::new()).unwrap();
        assert_eq!(fast["writer"].model_id, "ollama:small");
        let good = resolve_slots(&recipe("accuracy", None), &installed, &BTreeMap::new()).unwrap();
        assert_eq!(good["writer"].model_id, "ollama:big");
        assert_eq!(good["writer"].via, ResolvedVia::Capability);
    }

    #[test]
    fn auto_never_picks_a_cloud_model_but_a_pin_or_default_may() {
        let mut cloud = model("remote", "", &[Capability::Text]);
        cloud.id = "codex:remote".into();
        cloud.provider = "codex".into();
        cloud.local = false;
        let installed = [cloud];
        let err = resolve_slots(&recipe("accuracy", None), &installed, &BTreeMap::new()).unwrap_err();
        assert!(matches!(err, ResolveError::NoCandidate { .. }), "{err}");
        let pins = BTreeMap::from([("writer".to_string(), "codex:remote".to_string())]);
        assert_eq!(resolve_slots(&recipe("accuracy", None), &installed, &pins).unwrap()["writer"].model_id, "codex:remote");
        let mut defaults = ResolveDefaults::default();
        defaults.by_capability.insert(Capability::Text, "codex:remote".into());
        let r = resolve_slots_with(&recipe("accuracy", None), &installed, &BTreeMap::new(), &defaults).unwrap();
        assert_eq!(r["writer"].via, ResolvedVia::Default);
    }

    fn planner_model(name: &str, size: &str, gb: u64) -> InstalledModel {
        let mut m = model(name, size, &[Capability::Text, Capability::StructuredOutput]);
        m.size_bytes = Some(gb << 30);
        m
    }

    fn ids(ranked: &[(InstalledModel, u32, String)]) -> Vec<&str> {
        ranked.iter().map(|(m, _, _)| m.name.as_str()).collect()
    }

    #[test]
    fn family_rank_reads_names_and_sizes() {
        assert_eq!(family_rank(&planner_model("qwen3:32b", "32.8B", 20)), 0);
        assert_eq!(family_rank(&planner_model("qwen3:8b", "8.2B", 5)), 5, "a small qwen3 is not a top drafter");
        assert_eq!(family_rank(&planner_model("glm-4.7-flash:q8", "29.9B", 31)), 1);
        assert_eq!(family_rank(&planner_model("gpt-oss:20b", "20.9B", 13)), 2);
        assert_eq!(family_rank(&planner_model("gemma3:27b", "27.4B", 17)), 3);
        assert_eq!(family_rank(&planner_model("gemma3:4b", "4.3B", 3)), 5);
        assert_eq!(family_rank(&planner_model("hf.co/unsloth/Qwen3-Coder-Next-GGUF:UD-Q4_K_XL", "80B", 49)), 4);
        assert_eq!(family_rank(&planner_model("qwen3-coder:30b", "30.5B", 18)), 4);
        assert_eq!(family_rank(&planner_model("llama3.1:70b", "70.6B", 40)), 5);
    }

    #[test]
    fn planning_rank_orders_by_family_then_size_when_everything_fits_and_nothing_is_measured() {
        let models = vec![
            planner_model("llama3.1:8b", "8B", 5),
            planner_model("qwen3-coder:30b", "30.5B", 18),
            planner_model("gemma3:27b", "27.4B", 17),
            planner_model("mistral-small:24b", "24B", 14),
            planner_model("gpt-oss:20b", "20.9B", 13),
            planner_model("glm-4.7-flash", "29.9B", 31),
            planner_model("qwen3:32b", "32.8B", 20),
        ];
        let ranked = rank_for_planning(&models, &BTreeMap::new(), Some(100 << 30));
        assert_eq!(ids(&ranked), vec!["qwen3:32b", "glm-4.7-flash", "gpt-oss:20b", "gemma3:27b", "qwen3-coder:30b", "mistral-small:24b", "llama3.1:8b"]);
        assert_eq!(ranked.iter().map(|(_, r, _)| *r).collect::<Vec<_>>(), (1..=7).collect::<Vec<u32>>());
        assert_eq!(ranked[0].2, "fits in memory, answers in a fixed shape (JSON), not yet measured");
    }

    #[test]
    fn planning_rank_puts_what_fits_in_memory_before_a_better_family() {
        let models = vec![planner_model("qwen3:235b", "235B", 140), planner_model("glm-4.7-flash", "29.9B", 20), planner_model("llama3.1:8b", "8B", 5)];
        let ranked = rank_for_planning(&models, &BTreeMap::new(), Some(36 << 30));
        assert_eq!(ids(&ranked), vec!["glm-4.7-flash", "llama3.1:8b", "qwen3:235b"]);
        assert!(ranked[2].2.starts_with("larger than this Mac can hold comfortably"), "{}", ranked[2].2);
        // RAM unknown: everything fits, so the family decides.
        let ranked = rank_for_planning(&models, &BTreeMap::new(), None);
        assert_eq!(ids(&ranked)[0], "qwen3:235b");
    }

    #[test]
    fn a_measured_clean_plan_under_two_minutes_beats_everything_fastest_first() {
        let models = vec![planner_model("qwen3:32b", "32.8B", 20), planner_model("glm-4.7-flash", "29.9B", 20), planner_model("llama3.1:8b", "8B", 5), planner_model("gemma3:27b", "27B", 17)];
        let measured = BTreeMap::from([
            ("ollama:llama3.1:8b".to_string(), Measured { ms: 48_000, clean: true }),
            ("ollama:gemma3:27b".to_string(), Measured { ms: 30_000, clean: true }),
            ("ollama:qwen3:32b".to_string(), Measured { ms: 130_000, clean: true }), // clean but slow: no head start
            ("ollama:glm-4.7-flash".to_string(), Measured { ms: 20_000, clean: false }), // fast but not clean
        ]);
        let ranked = rank_for_planning(&models, &measured, Some(100 << 30));
        assert_eq!(ids(&ranked), vec!["gemma3:27b", "llama3.1:8b", "qwen3:32b", "glm-4.7-flash"]);
        assert_eq!(ranked[1].2, "fits in memory, answers in a fixed shape (JSON), planned clean in 48 s");
        assert_eq!(ranked[2].2, "fits in memory, answers in a fixed shape (JSON), planned clean in 2 m 10 s");
        assert_eq!(ranked[3].2, "fits in memory, answers in a fixed shape (JSON), the last trial plan had problems, after 20 s");
    }

    #[test]
    fn planning_rank_skips_cloud_models_and_models_that_cannot_plan() {
        let mut cloud = planner_model("gpt-5", "", 0);
        cloud.local = false;
        cloud.size_bytes = None;
        let text_only = model("plain", "70B", &[Capability::Text]);
        let ranked = rank_for_planning(&[cloud, text_only, planner_model("qwen3:14b", "14.8B", 9)], &BTreeMap::new(), None);
        assert_eq!(ids(&ranked), vec!["qwen3:14b"]);
    }

    #[test]
    fn the_planner_slot_resolves_by_the_planning_rank_but_a_pin_or_default_still_wins() {
        let installed = vec![planner_model("qwen3-coder:80b", "80B", 49), planner_model("glm-4.7-flash", "29.9B", 20)];
        let recipe = crate::planner::planner_recipe();
        let mut defaults = ResolveDefaults::default();
        defaults.planning.memory_bytes = Some(36 << 30);
        let slot = crate::planner::PLANNER_SLOT;
        let r = resolve_slots_with(&recipe, &installed, &BTreeMap::new(), &defaults).unwrap();
        assert_eq!(r[slot].model_id, "ollama:glm-4.7-flash", "not the largest: the one that fits and drafts well");
        assert_eq!(r[slot].via, ResolvedVia::Capability);
        defaults.planning.measured.insert("ollama:qwen3-coder:80b".into(), Measured { ms: 90_000, clean: true });
        let r = resolve_slots_with(&recipe, &installed, &BTreeMap::new(), &defaults).unwrap();
        assert_eq!(r[slot].model_id, "ollama:qwen3-coder:80b", "a measured clean plan under two minutes wins");
        defaults.by_capability.insert(Capability::Text, "ollama:glm-4.7-flash".into());
        let r = resolve_slots_with(&recipe, &installed, &BTreeMap::new(), &defaults).unwrap();
        assert_eq!(r[slot].via, ResolvedVia::Default);
        let pins = BTreeMap::from([(slot.to_string(), "ollama:qwen3-coder:80b".to_string())]);
        let r = resolve_slots_with(&recipe, &installed, &pins, &defaults).unwrap();
        assert_eq!(r[slot].via, ResolvedVia::Pinned);
        // An ordinary recipe's slot named "planner" keeps the size preference.
        let ordinary = {
            let mut rec = self::recipe("accuracy", None);
            let s = rec.model_slots.shift_remove("writer").unwrap();
            rec.model_slots.insert("planner".into(), s);
            rec
        };
        let r = resolve_slots_with(&ordinary, &installed, &BTreeMap::new(), &ResolveDefaults::default()).unwrap();
        assert_eq!(r["planner"].model_id, "ollama:qwen3-coder:80b");
    }

    #[test]
    fn pinned_model_must_have_capability() {
        let installed = [model("embed", "0.3B", &[Capability::Embedding])];
        let pins = BTreeMap::from([("writer".to_string(), "ollama:embed".to_string())]);
        let err = resolve_slots(&recipe("accuracy", None), &installed, &pins).unwrap_err();
        assert!(matches!(err, ResolveError::PinnedLacksCapability { .. }), "{err}");
    }
}
