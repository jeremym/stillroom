use stillroom_core::Recipe;

/// Smoke command: proves the webview can reach the Rust runtime and that the
/// embedded recipe schema compiles. Replaced by real commands as the executor lands.
#[tauri::command]
fn runtime_status() -> Result<String, String> {
    let schema: serde_json::Value =
        serde_json::from_str(stillroom_core::RECIPE_SCHEMA).map_err(|e| e.to_string())?;
    Recipe::validate_value(&serde_json::json!({})).err(); // exercise the validator; empty object is expected to fail
    let title = schema["title"].as_str().unwrap_or("?");
    Ok(format!("ok · schema loaded ({title} v0)"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![runtime_status])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
