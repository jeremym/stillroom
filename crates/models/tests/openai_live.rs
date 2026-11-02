//! Live check of the OpenAI-compatible adapter against whatever `STILLROOM_OPENAI_URL` points at
//! (Ollama's `/v1` on this Mac is enough; set `STILLROOM_OPENAI_MODEL` to a chat model there).
//! Ignored by default. Run with: cargo test -p stillroom-models --test openai_live -- --ignored
use stillroom_core::model::{ChatMessage, ChatRequest, ToolSpec};
use stillroom_models::openai::OpenAi;
use stillroom_models::Provider;

fn target() -> Option<(String, String)> {
    Some((std::env::var("STILLROOM_OPENAI_URL").ok()?, std::env::var("STILLROOM_OPENAI_MODEL").ok()?))
}

#[tokio::test]
#[ignore]
async fn lists_answers_and_fits_a_schema() {
    let Some((url, model)) = target() else { return };
    let p = OpenAi::new(url, std::env::var("STILLROOM_OPENAI_KEY").ok(), vec![]);
    assert!(p.is_available().await, "nothing answered at {}", p.base_url());
    let models = p.list_models().await.unwrap();
    assert!(models.iter().any(|m| m.name == model), "{model} not in {:?}", models.iter().map(|m| &m.name).collect::<Vec<_>>());
    let req = ChatRequest {
        messages: vec![ChatMessage::new("user", "Reply with the single word pong.")],
        think: Some(serde_json::json!(false)),
        ..Default::default()
    };
    let r = p.chat(&model, req).await.unwrap();
    assert!(r.content.to_lowercase().contains("pong"), "{}", r.content);
    let schema = serde_json::json!({"type":"object","properties":{"word":{"type":"string"}},"required":["word"]});
    let req = ChatRequest {
        messages: vec![ChatMessage::new("user", "Give the word pong.")],
        json_schema: Some(schema),
        think: Some(serde_json::json!(false)),
        ..Default::default()
    };
    let r = p.chat(&model, req).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&r.content).expect("schema-shaped answer");
    assert!(v.get("word").is_some(), "{}", r.content);
}

#[tokio::test]
#[ignore]
async fn calls_a_tool() {
    let Some((url, model)) = target() else { return };
    let p = OpenAi::new(url, None, vec![]);
    let req = ChatRequest {
        messages: vec![ChatMessage::new("user", "What is the weather in Paris? Use the tool.")],
        tools: vec![ToolSpec {
            name: "weather".into(),
            description: "Current weather for a city".into(),
            parameters: serde_json::json!({"type":"object","properties":{"city":{"type":"string"}},"required":["city"]}),
        }],
        think: Some(serde_json::json!(false)),
        ..Default::default()
    };
    let r = p.chat(&model, req).await.unwrap();
    assert_eq!(r.tool_calls.len(), 1, "content was: {}", r.content);
    assert_eq!(r.tool_calls[0].name, "weather");
}
