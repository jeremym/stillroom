//! Slot resolution: pinned > recommended > capability resolve. Decided before the run.

use crate::model::InstalledModel;
use crate::recipe::{Capability, ModelSlot, Preference, Recipe};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedVia {
    Pinned,
    Recommended,
    Capability,
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
        // 2. Recommended, if installed and suitable
        if let Some(rec) = &slot.recommended {
            if let Some(m) = installed.iter().find(|m| &m.id == rec || &m.name == rec) {
                if satisfies(m, slot).is_ok() && fits_context(m, slot) {
                    out.insert(name.clone(), Resolution { slot: name.clone(), model_id: m.id.clone(), via: ResolvedVia::Recommended });
                    continue;
                }
            }
        }
        // 3. Capability resolve
        let mut candidates: Vec<&InstalledModel> = installed
            .iter()
            .filter(|m| satisfies(m, slot).is_ok() && fits_context(m, slot))
            .collect();
        if candidates.is_empty() {
            return Err(ResolveError::NoCandidate { slot: name.clone(), requires: slot.requires.clone() });
        }
        let size = |m: &InstalledModel| m.parameter_billions().unwrap_or(0.0);
        match slot.preference.unwrap_or_default() {
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
