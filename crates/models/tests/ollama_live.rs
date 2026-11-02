//! Live test against a local Ollama. Ignored by default; run with
//! `STILLROOM_RECIPE=learning-kit cargo test -p stillroom-models --test ollama_live -- --ignored --nocapture`.
//! Pin slots with `STILLROOM_PIN_<slot>=ollama:model` (e.g. STILLROOM_PIN_writer).
//! Set `STILLROOM_DB=/path/to/stillroom.db` to save the run where the app will show it,
//! and `STILLROOM_ARTIFACTS=/path/to/artifacts` so audio files land where the app can play them.
//! Set `STILLROOM_EDIT=<step>=<json or @file>` to follow the run with a change-propagation
//! rerun that replaces that step's output with the given value (e.g. `STILLROOM_EDIT=script=@edited.json`).
//! Set `STILLROOM_KEEPS=/path` so stages with a run keep remember between runs.
//! Set `STILLROOM_INPUT_<name>=<value>` to supply inputs other than the built-in `file` sample
//! (e.g. STILLROOM_INPUT_inbox=/path). With `STILLROOM_APPROVE_ALL=1`, a run that stops for review
//! is continued with every item approved. With `STILLROOM_RESUME_RUN=<run id>` (and a store),
//! no fresh run happens: that stored run is continued with every review item approved.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use stillroom_core::tools::ToolRegistry;
use stillroom_core::{edits_for_rerun, pins_from_run, resolve_slots, validate_edit, Executor, RecipeLibrary, RecipeSource, Run, RunStatus, Store};
use stillroom_models::ollama::Ollama;
use stillroom_models::{ModelRouter, Provider};

const SAMPLE: &str = r#"# OAuth 2.0 in brief

OAuth 2.0 is an authorization framework. It lets a client application obtain limited access to a protected resource on behalf of a user, without the client ever seeing the user's password.

Four roles take part. The resource owner is the user who owns the data. The client is the application that wants to use the data. The authorization server verifies the user and issues tokens. The resource server hosts the protected data and checks tokens on every request.

An access token is a credential the client presents to the resource server. It is short-lived, usually minutes to an hour, and carries a scope that limits what it can do. Because access tokens expire quickly, a refresh token is often issued alongside. The refresh token is long-lived and is sent only to the authorization server, never to the resource server, to obtain a new access token without asking the user again.

The authorization code flow is the recommended flow for web and mobile apps. The client redirects the user to the authorization server, the user signs in and consents, and the authorization server redirects back with a one-time authorization code. The client exchanges that code for tokens over a back channel. PKCE, which stands for Proof Key for Code Exchange, adds a per-request secret so that an intercepted code cannot be exchanged by an attacker. PKCE is required for public clients such as single-page and mobile apps.
"#;

#[tokio::test]
#[ignore]
async fn run_recipe_against_local_ollama() {
    let ollama = Ollama::default_local();
    if !ollama.is_available().await {
        eprintln!("Ollama not running; skipping");
        return;
    }
    let router = Arc::new(ModelRouter::new(vec![Arc::new(ollama)]));
    let installed = router.list_all().await;
    assert!(!installed.is_empty(), "no models installed in Ollama");

    let recipe_id = std::env::var("STILLROOM_RECIPE").unwrap_or_else(|_| "summarize-file".into());
    let lib = RecipeLibrary::new(vec![(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../recipes"), RecipeSource::Builtin)]);
    let bundle = lib.find(&recipe_id).expect("recipe in library");

    let pins: BTreeMap<String, String> = std::env::vars()
        .filter_map(|(k, v)| k.strip_prefix("STILLROOM_PIN_").map(|s| (s.to_string(), v)))
        .collect();
    let resolutions = resolve_slots(&bundle.recipe, &installed, &pins).expect("slot resolution");
    for r in resolutions.values() {
        eprintln!("slot {} -> {} ({:?})", r.slot, r.model_id, r.via);
    }

    let dir = std::env::temp_dir().join(format!("stillroom-live-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("oauth.md");
    std::fs::write(&file, SAMPLE).unwrap();

    let mut inputs = indexmap::IndexMap::new();
    if bundle.recipe.inputs.contains_key("file") {
        inputs.insert("file".to_string(), serde_json::Value::String(file.to_string_lossy().into()));
    }
    if let Ok(focus) = std::env::var("STILLROOM_FOCUS") {
        inputs.insert("focus".to_string(), serde_json::Value::String(focus));
    }
    for (k, v) in std::env::vars() {
        if let Some(name) = k.strip_prefix("STILLROOM_INPUT_") {
            // A JSON array or object is taken as such (lists of links); anything else is a string.
            let value = match serde_json::from_str::<serde_json::Value>(&v) {
                Ok(j @ (serde_json::Value::Array(_) | serde_json::Value::Object(_))) => j,
                _ => serde_json::Value::String(v),
            };
            inputs.insert(name.to_string(), value);
        }
    }

    let mut exec = Executor::new(Arc::new(ToolRegistry::builtin()), router);
    let store = std::env::var("STILLROOM_DB").ok().map(|db| Arc::new(std::sync::Mutex::new(Store::open(&db).expect("open store"))));
    if let Some(s) = &store {
        exec = exec.with_cache(s.clone());
    }
    let run = if let Ok(id) = std::env::var("STILLROOM_RESUME_RUN") {
        let stored = store.as_ref().expect("STILLROOM_DB is required with STILLROOM_RESUME_RUN").lock().unwrap().get_run(&id).expect("read store").expect("run exists");
        eprintln!("resuming stored run {} ({:?})", stored.id, stored.status);
        stored
    } else {
        let mut request = bundle.request(inputs, resolutions);
        request.reuse = std::env::var("STILLROOM_NO_REUSE").is_err();
        request.artifact_base = std::env::var("STILLROOM_ARTIFACTS").ok().map(std::path::PathBuf::from);
        request.keep_base = std::env::var("STILLROOM_KEEPS").ok().map(std::path::PathBuf::from);
        let started = std::time::Instant::now();
        let run = exec.run(request, None).await;
        eprintln!("run {} finished in {:?}: {:?}", run.id, started.elapsed(), run.status);
        print_steps(&run);
        run
    };
    for (name, value) in &run.outputs {
        let text = match value {
            serde_json::Value::String(s) => s.clone(),
            v => serde_json::to_string_pretty(v).unwrap(),
        };
        let shown: String = text.chars().take(1800).collect();
        eprintln!("--- {name} ({} chars) ---\n{shown}{}", text.len(), if text.len() > 1800 { "\n…" } else { "" });
    }
    if let Some(s) = &store {
        s.lock().unwrap().save_run(&run).expect("save run");
        eprintln!("saved run to the store");
    }
    assert!(matches!(run.status, RunStatus::Succeeded | RunStatus::NeedsReview), "{:?}", run.error);

    // A run waiting for review: approve everything and continue, if asked.
    if run.status == RunStatus::NeedsReview && std::env::var("STILLROOM_APPROVE_ALL").is_ok() {
        let review = run.steps.iter().find(|s| s.kind == "review" && s.status == stillroom_core::StepStatus::Skipped).expect("pending review step");
        let items = review.output.as_ref().and_then(|o| o.get("items")).cloned().unwrap_or(serde_json::Value::Array(vec![]));
        eprintln!("=== APPROVING {} item(s) at step {}", items.as_array().map(Vec::len).unwrap_or(0), review.step_id);
        let decision = serde_json::json!({ "decided": true, "approved": items, "rejected": [] });
        let edits = edits_for_rerun(&run, &review.step_id, decision).expect("edit applies");
        let resolutions = resolve_slots(&bundle.recipe, &installed, &pins_from_run(&run)).expect("pins from prior run");
        let mut request = bundle.request(run.inputs.clone(), resolutions);
        request.reuse = true;
        request.artifact_base = std::env::var("STILLROOM_ARTIFACTS").ok().map(std::path::PathBuf::from);
        request.edits = edits;
        request.resume_from = Some(Box::new(run.clone()));
        let started = std::time::Instant::now();
        let continued = exec.run(request, None).await;
        eprintln!("=== CONTINUED: run {} finished in {:?}: {:?}", continued.id, started.elapsed(), continued.status);
        print_steps(&continued);
        for (name, value) in &continued.outputs {
            let text = serde_json::to_string_pretty(value).unwrap();
            let shown: String = text.chars().take(1500).collect();
            eprintln!("--- {name} ---\n{shown}");
        }
        if let Some(s) = &store {
            s.lock().unwrap().save_run(&continued).expect("save continued run");
        }
        assert_eq!(continued.status, RunStatus::Succeeded, "{:?}", continued.error);
    }

    // Optional change-propagation pass: replace one step's output and rerun from the saved run.
    if let Ok(spec) = std::env::var("STILLROOM_EDIT") {
        let (step_id, value) = spec.split_once('=').expect("STILLROOM_EDIT=<step>=<json or @file>");
        let text = match value.strip_prefix('@') {
            Some(path) => std::fs::read_to_string(path).expect("read edit file"),
            None => value.to_string(),
        };
        let output: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text));
        validate_edit(&bundle.recipe, step_id, &output).expect("edit matches schema");
        let edits = edits_for_rerun(&run, step_id, output).expect("edit applies to a step of the run");
        let resolutions = resolve_slots(&bundle.recipe, &installed, &pins_from_run(&run)).expect("pins from prior run");
        let mut request = bundle.request(run.inputs.clone(), resolutions);
        request.reuse = true;
        request.artifact_base = std::env::var("STILLROOM_ARTIFACTS").ok().map(std::path::PathBuf::from);
        request.edits = edits;
        request.resume_from = Some(Box::new(run.clone()));
        let started = std::time::Instant::now();
        let edited = exec.run(request, None).await;
        eprintln!("=== EDIT {step_id}: run {} finished in {:?}: {:?}", edited.id, started.elapsed(), edited.status);
        print_steps(&edited);
        if let Some(s) = &store {
            s.lock().unwrap().save_run(&edited).expect("save edited run");
        }
        assert_eq!(edited.status, RunStatus::Succeeded, "{:?}", edited.error);
        assert!(edited.steps.iter().any(|s| s.step_id == step_id && s.edited), "edit applied");
    }
    std::fs::remove_dir_all(dir).ok();
}

fn print_steps(run: &Run) {
    for s in &run.steps {
        let how = if s.edited {
            " (edited by user)".to_string()
        } else if let Some(r) = &s.skip_reason {
            format!(" ({r})")
        } else {
            s.reused_from.as_ref().map(|r| format!(" (reused from {})", &r[r.len().saturating_sub(8)..])).unwrap_or_default()
        };
        let calls = if s.tool_calls.is_empty() {
            String::new()
        } else {
            format!(" tools=[{}]", s.tool_calls.iter().map(|c| format!("{}{}", c.tool, if c.error.is_some() { "!" } else { "" })).collect::<Vec<_>>().join(","))
        };
        eprintln!(
            "  step {:<22} [{}] {:?}{}{} model={} tokens={:?}->{:?} {}ms{}",
            s.step_id,
            s.kind,
            s.status,
            how,
            calls,
            s.model.as_deref().unwrap_or("-"),
            s.prompt_tokens,
            s.completion_tokens,
            s.duration_ms.unwrap_or(0),
            s.error.as_ref().map(|e| format!(" ERROR {e}")).unwrap_or_default()
        );
    }
}
