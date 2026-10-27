//! Change propagation: an artifact edited by the user is fed back in as that step's output,
//! and only the steps downstream of it are recomputed. Everything else is reused from the cache.
//!
//! An edit is pinned to the inputs the step had when the user made it. If a later run changes
//! what feeds that step, the edit is set aside and the step is regenerated, because the user
//! edited an output of inputs that no longer exist.

use crate::cache::fingerprint;
use crate::recipe::{Recipe, Step};
use crate::refs;
use crate::trace::Run;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// A user-supplied output for one step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepEdit {
    pub output: Value,
    /// Fingerprint of the step's resolved inputs at the time of the edit. The edit applies only
    /// while the step's inputs still match.
    pub inputs_fingerprint: String,
}

/// Fingerprint of a step's resolved inputs, as used to pin edits.
pub fn inputs_fingerprint(inputs: &Value) -> String {
    fingerprint(&[&serde_json::to_string(inputs).unwrap_or_default()])
}

/// Step ids this step reads from: data references in its inputs, `when`, `over`, `items`,
/// and (for a map) its inner steps, plus explicit `dependsOn`. A map's inner steps are not
/// dependencies of the map.
pub fn dependencies(step: &Step) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    refs::step_refs(&Value::Object(step.inputs().clone().into_iter().collect()), &mut names);
    if let Some(w) = step.when() {
        refs::step_refs(&Value::String(w.to_string()), &mut names);
    }
    match step {
        Step::Map(m) => {
            refs::step_refs(&Value::String(m.over.clone()), &mut names);
            let inner: BTreeSet<&str> = m.steps.iter().map(|s| s.id()).collect();
            for sub in &m.steps {
                for d in dependencies(sub) {
                    if !inner.contains(d.as_str()) {
                        names.insert(d);
                    }
                }
            }
        }
        Step::Review(r) => refs::step_refs(&Value::String(r.items.clone()), &mut names),
        _ => {}
    }
    names.extend(step.depends_on().iter().cloned());
    names
}

/// Recipe inputs a step reads, directly or (for a map) through its inner steps.
pub fn input_dependencies(step: &Step) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    refs::input_refs(&Value::Object(step.inputs().clone().into_iter().collect()), &mut names);
    if let Some(w) = step.when() {
        refs::input_refs(&Value::String(w.to_string()), &mut names);
    }
    match step {
        Step::Map(m) => {
            refs::input_refs(&Value::String(m.over.clone()), &mut names);
            for sub in &m.steps {
                names.extend(input_dependencies(sub));
            }
        }
        Step::Review(r) => refs::input_refs(&Value::String(r.items.clone()), &mut names),
        _ => {}
    }
    names
}

/// Every step downstream of `step_id`, transitively, in recipe order.
pub fn dependents(recipe: &Recipe, step_id: &str) -> Vec<String> {
    let mut affected: BTreeSet<String> = BTreeSet::from([step_id.to_string()]);
    let mut out = Vec::new();
    // Steps are declared in an order where dependencies usually come first, but a single pass
    // is not guaranteed to be enough, so iterate to a fixed point.
    loop {
        let before = affected.len();
        for step in &recipe.steps {
            if affected.contains(step.id()) {
                continue;
            }
            if dependencies(step).iter().any(|d| affected.contains(d)) {
                affected.insert(step.id().to_string());
            }
        }
        if affected.len() == before {
            break;
        }
    }
    for step in &recipe.steps {
        if step.id() != step_id && affected.contains(step.id()) {
            out.push(step.id().to_string());
        }
    }
    out
}

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("run has no step {0}")]
    NoSuchStep(String),
    #[error("step {step}: edited output does not match its schema: {message}")]
    Schema { step: String, message: String },
    #[error("step {0} has not produced an output to edit")]
    NoOutput(String),
}

/// Check an edited output against the step's output schema, if it has one.
pub fn validate_edit(recipe: &Recipe, step_id: &str, output: &Value) -> Result<(), EditError> {
    let step = recipe.step(step_id).ok_or_else(|| EditError::NoSuchStep(step_id.to_string()))?;
    let schema = match step {
        Step::Model(m) => m.output_schema.as_ref(),
        Step::Tool(t) => t.output_schema.as_ref(),
        _ => None,
    };
    if let Some(schema) = schema {
        crate::executor::validate_against(schema, output)
            .map_err(|message| EditError::Schema { step: step_id.to_string(), message })?;
    }
    Ok(())
}

/// The edits to carry into a rerun of `prior`: everything the prior run was asked to apply,
/// plus a new edit to `step_id`, pinned to the inputs that step had in `prior`.
pub fn edits_for_rerun(prior: &Run, step_id: &str, output: Value) -> Result<BTreeMap<String, StepEdit>, EditError> {
    let rec = prior
        .steps
        .iter()
        .find(|s| s.step_id == step_id)
        .ok_or_else(|| EditError::NoSuchStep(step_id.to_string()))?;
    if rec.output.is_none() {
        return Err(EditError::NoOutput(step_id.to_string()));
    }
    let mut edits = prior.edits.clone();
    edits.insert(step_id.to_string(), StepEdit { output, inputs_fingerprint: inputs_fingerprint(&rec.inputs) });
    Ok(edits)
}

/// Pin every slot to the model the prior run used, so unchanged steps can be reused.
pub fn pins_from_run(prior: &Run) -> BTreeMap<String, String> {
    prior
        .resolutions
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(slot, r)| r.get("modelId").and_then(Value::as_str).map(|id| (slot.clone(), id.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_transitive_dependents_in_recipe_order() {
        let yaml = include_str!("../../../recipes/learning-kit/recipe.yaml");
        let recipe = Recipe::from_yaml(yaml).unwrap();
        assert_eq!(dependents(&recipe, "script"), vec!["narrate"]);
        assert_eq!(dependents(&recipe, "notes"), vec!["script", "narrate"]);
        assert_eq!(
            dependents(&recipe, "extract"),
            vec!["notes", "flashcards", "script", "narrate", "slides", "concept_map", "diagram"]
        );
        assert!(dependents(&recipe, "diagram").is_empty());
    }

    #[test]
    fn edits_are_checked_against_the_schema() {
        let yaml = include_str!("../../../recipes/learning-kit/recipe.yaml");
        let mut recipe = Recipe::from_yaml(yaml).unwrap();
        // Inline the script schema the way the bundle loader would.
        let schema: Value = serde_json::from_str(include_str!("../../../recipes/learning-kit/schemas/script.json")).unwrap();
        for s in &mut recipe.steps {
            if let Step::Model(m) = s {
                if m.id == "script" {
                    m.output_schema = Some(schema.clone());
                }
            }
        }
        let good = serde_json::json!({ "title": "t", "chapters": [{ "title": "a", "text": "b", "concept": "c" }] });
        assert!(validate_edit(&recipe, "script", &good).is_ok());
        let bad = serde_json::json!({ "title": "t" });
        assert!(matches!(validate_edit(&recipe, "script", &bad), Err(EditError::Schema { .. })));
        assert!(matches!(validate_edit(&recipe, "nope", &good), Err(EditError::NoSuchStep(_))));
        // Steps without a schema accept anything.
        assert!(validate_edit(&recipe, "notes", &Value::String("new notes".into())).is_ok());
    }
}
