//! Live check of the model-call phases against a local Ollama. Ignored by default; run with
//! `cargo test -p stillroom-models --test phases_live -- --ignored --nocapture`.
//! Pick the model with `STILLROOM_MODEL=<ollama name>`; otherwise the smallest chat model installed.
//! Prints each phase with the time it arrived, then Ollama's own load and read times.

use std::sync::Mutex;
use std::time::Instant;
use stillroom_core::model::{CallPhase, ChatMessage, ChatRequest, StreamEvent};
use stillroom_models::ollama::Ollama;
use stillroom_models::Provider;

#[tokio::test]
#[ignore]
async fn phases_come_from_ollama_in_order() {
    let ollama = Ollama::default_local();
    if !ollama.is_available().await {
        eprintln!("Ollama not running; skipping");
        return;
    }
    let model = match std::env::var("STILLROOM_MODEL") {
        Ok(m) => m,
        Err(_) => {
            let mut models = ollama.list_models().await.unwrap();
            models.retain(|m| m.capabilities.contains(&stillroom_core::recipe::Capability::Text));
            models.sort_by_key(|m| m.size_bytes.unwrap_or(u64::MAX));
            models.first().map(|m| m.name.clone()).expect("a chat model is installed")
        }
    };
    let loaded_before = ollama.is_loaded(&model).await;
    eprintln!("model {model}; loaded before the call: {loaded_before:?}");
    let started = Instant::now();
    let seen = Mutex::new(Vec::<(u128, String)>::new());
    let on = |ev: StreamEvent<'_>| {
        let ms = started.elapsed().as_millis();
        match ev {
            StreamEvent::Phase(p) => {
                eprintln!("  {ms:>6} ms  phase {p:?}");
                seen.lock().unwrap().push((ms, format!("{p:?}")));
            }
            StreamEvent::Text(t) => {
                let mut s = seen.lock().unwrap();
                if s.last().is_none_or(|(_, w)| w != "text") {
                    eprintln!("  {ms:>6} ms  first text {:?}", t.chars().take(30).collect::<String>());
                    s.push((ms, "text".into()));
                }
            }
        }
    };
    let req = ChatRequest { messages: vec![ChatMessage::new("user", "Name three colours, one per line, nothing else.")], think: Some(false.into()), ..Default::default() };
    let resp = ollama.chat_events(&model, req, &on).await.unwrap();
    eprintln!("  done in {} ms; Ollama says load {:?} ms, read {:?} ms, wrote {:?} tokens at {:?} tok/s", started.elapsed().as_millis(), resp.load_ms, resp.prompt_ms, resp.completion_tokens, resp.completion_tokens_per_second);
    let seen = seen.lock().unwrap();
    let order: Vec<&str> = seen.iter().map(|(_, w)| w.as_str()).collect();
    assert!(order.len() >= 3, "{order:?}");
    assert!(matches!(order[0], "Loading" | "Reading"), "{order:?}");
    assert_eq!(order[0] == "Loading", loaded_before == Some(false), "the first phase follows /api/ps: {order:?}");
    assert_eq!(&order[1..], ["Writing", "text"], "{order:?}");
    assert!(resp.prompt_ms.is_some(), "Ollama reports the read time");
    let _ = CallPhase::Writing;
}
