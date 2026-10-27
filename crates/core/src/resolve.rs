//! Slot resolution: pinned > your default for the capability > recommended > capability resolve.
//! Decided before the run, never by a model.

use crate::model::InstalledModel;
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ResolveDefaults {
    pub preference: Option<Preference>,
    pub by_capability: BTreeMap<Capability, String>,
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
    #[error("slot {slot}: pinned model {model} lacks required capability {capability:?}")]
    PinnedLacksCapability { slot: String, model: String, capability: Capability },
    #[error("slot {slot}: no installed model satisfies {requires:?}")]
    NoCandidate { slot: String, requires: Vec<Capability> },
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
        // 4. Capability resolve
        let mut candidates: Vec<&InstalledModel> = installed
            .iter()
            .filter(|m| satisfies(m, slot).is_ok() && fits_context(m, slot))
            .collect();
        if candidates.is_empty() {
            return Err(ResolveError::NoCandidate { slot: name.clone(), requires: slot.requires.clone() });
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
    fn pinned_model_must_have_capability() {
        let installed = [model("embed", "0.3B", &[Capability::Embedding])];
        let pins = BTreeMap::from([("writer".to_string(), "ollama:embed".to_string())]);
        let err = resolve_slots(&recipe("accuracy", None), &installed, &pins).unwrap_err();
        assert!(matches!(err, ResolveError::PinnedLacksCapability { .. }), "{err}");
    }
}
