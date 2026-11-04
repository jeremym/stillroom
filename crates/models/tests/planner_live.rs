//! Live planner test against a local Ollama. Ignored by default; run with
//! `cargo test -p stillroom-models --test planner_live -- --ignored --nocapture`.
//!
//! - `STILLROOM_GOAL`: what the recipe should do (default: a summarize-a-file goal, so the draft
//!   can be compared with the built-in recipe).
//! - `STILLROOM_MATERIAL`: notes or links to hand the planner.
//! - `STILLROOM_PLANNER_MODEL`: pin the planning model (`ollama:name`); otherwise Auto.
//! - `STILLROOM_PLAN_ID`, `STILLROOM_PLAN_NAME`, `STILLROOM_PLAN_ICON`: what the recipe is called
//!   (default `planned`, "Planned recipe", `notes`).
//! - `STILLROOM_PLAN_OUT`: a directory to write the drafted recipe bundle into.
//! - `STILLROOM_PLAN_ANSWERS_OUT`: a directory to write the raw model answers into, as a
//!   planner-answer fixture `<id>.json` in the shape `crates/core/tests/compat/planner-answers/`
//!   uses (request, strip, the last composing answer, the merge's problems) plus the full
//!   transcript `<id>.transcript.md`. Copy the fixture into that folder to pin the goal.
//! - `STILLROOM_PLAN_RUN=1`: if the draft is clean, run it with `file` = the OAuth sample and any
//!   `STILLROOM_INPUT_<name>` values, and print the outputs.

use std::collections::BTreeMap;
use std::sync::Arc;
use stillroom_core::planner::{planner_recipe, PLANNER_SLOT};
use stillroom_core::prompts::PLANNER_PROMPT;
use stillroom_core::tools::ToolRegistry;
use stillroom_core::{resolve_slots, Executor, ModelDefaults, PlanEvent, PlanRequest, Planner, RecipeBundle, RecipeSource, RunStatus};
use stillroom_models::ollama::Ollama;
use stillroom_models::{ModelRouter, Provider};

const SAMPLE: &str = include_str!("sample_oauth.md");

/// The raw answers of a plan, split out of its transcript: one `(phase, text)` per model call, in
/// order. The transcript heads each call with `── call N (strip|steps|fix) ──`; a thinking model's
/// text is shown first under `(thinking)`, the answer proper after `(answer)`.
fn raw_answers(transcript: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for part in transcript.split("── call ").skip(1) {
        let (head, body) = part.split_once('\n').unwrap_or((part, ""));
        let phase = head.split('(').nth(1).and_then(|p| p.split(')').next()).unwrap_or("?").to_string();
        let answer = body.rsplit("(answer)\n").next().unwrap_or(body);
        out.push((phase, answer.to_string()));
    }
    out
}

/// The JSON object in a model's answer, as `extract_json` in the planner finds it.
fn json_in(text: &str) -> Option<serde_json::Value> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(t) {
        return Some(v);
    }
    let (start, end) = (t.find('{')?, t.rfind('}')?);
    serde_json::from_str(&t[start..=end]).ok()
}

/// A planner-answer fixture in the compat tests' shape: `about`, `request`, `strip` (the first
/// call's answer, when there was a strip call), `answer` (the last composing or fixing answer,
/// null when every stage was ready-made), and `problems` (what the merge of those two reports,
/// computed by replaying them through `draft_from_answers`).
fn answers_fixture(req: &PlanRequest, out: &stillroom_core::PlanOutcome, lib: &stillroom_core::StageLibrary) -> serde_json::Value {
    let calls = raw_answers(&out.transcript);
    let strip = calls.iter().find(|(p, _)| p == "strip").and_then(|(_, t)| json_in(t));
    let answer = calls.iter().rev().find(|(p, _)| p == "steps" || p == "fix").and_then(|(_, t)| json_in(t));
    let problems = stillroom_core::planner::draft_from_answers(req, strip.as_ref().map(|v| v.to_string()).as_deref(), answer.as_ref().map(|v| v.to_string()).as_deref(), Some(lib))
        .map(|(_, problems)| problems)
        .unwrap_or_else(|e| vec![format!("the recorded answers do not merge: {e}")]);
    let fixes = calls.iter().filter(|(p, _)| p == "fix").count();
    serde_json::json!({
        "about": format!(
            "Recorded live from {} in {} s, {} model call(s){}: the goal \"{}\".",
            out.model_id,
            out.duration_ms / 1000,
            calls.len(),
            if fixes > 0 { format!(" ({fixes} fix round(s); the last answer is the one kept)") } else { String::new() },
            req.goal
        ),
        "request": req,
        "strip": strip,
        "answer": answer,
        "problems": problems,
    })
}

#[tokio::test]
#[ignore]
async fn plan_a_recipe_against_local_ollama() {
    let ollama = Ollama::default_local();
    if !ollama.is_available().await {
        eprintln!("Ollama not running; skipping");
        return;
    }
    let router = Arc::new(ModelRouter::new(vec![Arc::new(ollama)]));
    let installed = router.list_all().await;
    assert!(!installed.is_empty(), "no models installed in Ollama");

    let mut pins = BTreeMap::new();
    if let Ok(m) = std::env::var("STILLROOM_PLANNER_MODEL") {
        pins.insert(PLANNER_SLOT.to_string(), m);
    }
    let res = resolve_slots(&planner_recipe(), &installed, &pins).expect("a model that answers in JSON");
    let model_id = res[PLANNER_SLOT].model_id.clone();
    eprintln!("planner -> {model_id} ({:?})", res[PLANNER_SLOT].via);

    let goal = std::env::var("STILLROOM_GOAL").unwrap_or_else(|_| {
        "Read one text or Markdown file the user picks and write a short summary in Markdown: one sentence saying what the document is, then the main points as bullets in the document's own order.".into()
    });
    let req = PlanRequest {
        goal,
        material: std::env::var("STILLROOM_MATERIAL").ok(),
        id: std::env::var("STILLROOM_PLAN_ID").unwrap_or_else(|_| "planned".into()),
        name: std::env::var("STILLROOM_PLAN_NAME").unwrap_or_else(|_| "Planned recipe".into()),
        icon: Some(std::env::var("STILLROOM_PLAN_ICON").unwrap_or_else(|_| "notes".into())),
    };
    let tools = ToolRegistry::builtin();
    let planner = Planner {
        library: Some(std::sync::Arc::new(stillroom_core::StageLibrary::load(&stillroom_core::stagelib::repo_stages_dir()).expect("stage library"))),
        models: router.clone(),
        model_id,
        tools: tools.catalog(),
        prompt: PLANNER_PROMPT.into(),
        strip_prompt: stillroom_core::prompts::STRIP_PROMPT.into(),
        timeout: Some(std::time::Duration::from_secs(900)),
        cancel: None,
        model_defaults: ModelDefaults::default(),
        max_attempts: 3,
        per_stage: std::env::var("STILLROOM_PER_STAGE").is_ok(),
    };
    let started = std::time::Instant::now();
    let observer = Arc::new(|e: PlanEvent| eprintln!("  {e:?}"));
    let out = planner.plan(&req, Some(observer)).await.expect("the planner answered");
    eprintln!(
        "planned in {:?}: {} attempt(s), {} prompt + {} completion tokens, {} problem(s)",
        started.elapsed(),
        out.attempts,
        out.prompt_tokens,
        out.completion_tokens,
        out.problems.len()
    );
    for p in &out.problems {
        eprintln!("  problem: {p}");
    }
    if let Some(stages) = out.draft.yaml.get("stages").and_then(|v| v.as_array()) {
        eprintln!("--- strip ---");
        for (i, st) in stages.iter().enumerate() {
            eprintln!("  {} {}  —  {}", i + 1, st["name"].as_str().unwrap_or("?"), st["why"].as_str().unwrap_or(""));
        }
    }
    eprintln!("--- recipe.yaml ---\n{}", serde_json::to_string_pretty(&out.draft.yaml).unwrap());
    for (path, text) in &out.draft.files {
        eprintln!("--- {path} ---\n{text}\n");
    }

    if let Ok(d) = std::env::var("STILLROOM_PLAN_ANSWERS_OUT") {
        let dir = std::path::PathBuf::from(d);
        std::fs::create_dir_all(&dir).expect("the answers folder");
        let lib = planner.library.as_deref().expect("stage library");
        let fixture = answers_fixture(&req, &out, lib);
        let path = dir.join(format!("{}.json", req.id));
        std::fs::write(&path, serde_json::to_string_pretty(&fixture).unwrap() + "\n").unwrap();
        std::fs::write(dir.join(format!("{}.transcript.md", req.id)), &out.transcript).unwrap();
        eprintln!("wrote the planner-answer fixture to {}", path.display());
    }

    let dir = match std::env::var("STILLROOM_PLAN_OUT") {
        Ok(d) => std::path::PathBuf::from(d),
        Err(_) => std::env::temp_dir().join(format!("stillroom-plan-{}", uuid::Uuid::now_v7())),
    };
    let known: Vec<String> = tools.ids().into_iter().map(String::from).collect();
    if out.problems.is_empty() {
        out.draft.write(&dir, &known).expect("write the bundle");
        eprintln!("wrote the drafted recipe to {}", dir.display());
    }
    assert!(out.problems.is_empty(), "the draft still has problems");
    let n = out.draft.yaml.get("stages").and_then(|v| v.as_array()).map(Vec::len).unwrap_or(0);
    assert!((2..=6).contains(&n), "expected two to six stages, got {n}");

    if std::env::var("STILLROOM_PLAN_RUN").is_err() {
        return;
    }
    let bundle = RecipeBundle::load(&dir, RecipeSource::User).expect("the drafted bundle loads");
    let resolutions = resolve_slots(&bundle.recipe, &installed, &BTreeMap::new()).expect("slots resolve");
    for r in resolutions.values() {
        eprintln!("slot {} -> {} ({:?})", r.slot, r.model_id, r.via);
    }
    let sample = dir.join("sample.md");
    std::fs::write(&sample, SAMPLE).unwrap();
    let mut inputs = indexmap::IndexMap::new();
    for (name, spec) in &bundle.recipe.inputs {
        if spec.get("type").and_then(serde_json::Value::as_str) == Some("file") {
            inputs.insert(name.clone(), serde_json::Value::String(sample.to_string_lossy().into()));
        }
    }
    for (k, v) in std::env::vars() {
        if let Some(name) = k.strip_prefix("STILLROOM_INPUT_") {
            let value = match serde_json::from_str::<serde_json::Value>(&v) {
                Ok(j @ (serde_json::Value::Array(_) | serde_json::Value::Object(_))) => j,
                _ => serde_json::Value::String(v),
            };
            inputs.insert(name.to_string(), value);
        }
    }
    let exec = Executor::new(Arc::new(tools), router);
    let request = bundle.request(inputs, resolutions);
    let started = std::time::Instant::now();
    let run = exec.run(request, None).await;
    eprintln!("run {} finished in {:?}: {:?} {:?}", run.id, started.elapsed(), run.status, run.error);
    for s in &run.steps {
        eprintln!("  {} {:?} {}", s.step_id, s.status, s.error.clone().unwrap_or_default());
    }
    for (name, value) in &run.outputs {
        let text = match value {
            serde_json::Value::String(s) => s.clone(),
            v => serde_json::to_string_pretty(v).unwrap(),
        };
        let shown: String = text.chars().take(1500).collect();
        eprintln!("--- {name} ({} chars) ---\n{shown}", text.len());
    }
    assert!(matches!(run.status, RunStatus::Succeeded | RunStatus::NeedsReview), "{:?}", run.error);
}
