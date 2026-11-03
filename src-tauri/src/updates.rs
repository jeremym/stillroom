//! Is there a newer Stillroom? The one call the app makes on its own to anything beyond this Mac:
//! GitHub's releases feed for the repository, read on request from Settings › About, or once a day
//! on launch when the person has turned that on. Nothing is downloaded or installed here; the
//! answer is a version number, the release's notes, and a link.

use crate::{AppState, CmdResult};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use stillroom_core::recipe::compare_versions;
use tauri::{AppHandle, Emitter, Manager};

/// The settings key: `{ daily: bool, last: UpdateCheck }`.
pub const SETTING: &str = "updates";
/// Written by the engine after a check; a patch from the screen that leaves it out keeps it.
pub const LAST_FIELD: &str = "last";
/// The page the app asks. The repository is public, so no token is sent.
pub const FEED: &str = "https://api.github.com/repos/jeremym/stillroom/releases/latest";

/// What the last check found.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    /// The version running now.
    pub current: String,
    /// The newest release's version, or null when the feed could not be read.
    pub latest: Option<String>,
    /// True when `latest` is newer than `current`.
    pub newer: bool,
    /// The release page, to read the notes and download from.
    pub url: Option<String>,
    /// The release notes as written on GitHub, Markdown, capped.
    pub notes: Option<String>,
    pub published_at: Option<String>,
    pub checked_at: DateTime<Utc>,
    /// Why there is no answer: the Mac is offline, GitHub refused, the feed changed shape.
    pub error: Option<String>,
}

/// Read one release document (GitHub's `releases/latest` shape) against the running version.
pub fn read_release(current: &str, release: &Value, now: DateTime<Utc>) -> UpdateCheck {
    let tag = release["tag_name"].as_str().map(str::trim).filter(|t| !t.is_empty());
    let Some(tag) = tag else {
        return UpdateCheck {
            current: current.into(),
            latest: None,
            newer: false,
            url: None,
            notes: None,
            published_at: None,
            checked_at: now,
            error: Some("the release feed had no version in it".into()),
        };
    };
    let latest = tag.trim_start_matches('v').to_string();
    let notes = release["body"].as_str().map(|b| {
        let b = b.trim();
        if b.chars().count() > 4000 {
            format!("{}…", b.chars().take(4000).collect::<String>())
        } else {
            b.to_string()
        }
    });
    UpdateCheck {
        current: current.into(),
        newer: compare_versions(&latest, current) == std::cmp::Ordering::Greater,
        latest: Some(latest),
        url: release["html_url"].as_str().map(String::from),
        notes,
        published_at: release["published_at"].as_str().map(String::from),
        checked_at: now,
        error: None,
    }
}

fn failed(current: &str, why: String) -> UpdateCheck {
    UpdateCheck {
        current: current.into(),
        latest: None,
        newer: false,
        url: None,
        notes: None,
        published_at: None,
        checked_at: Utc::now(),
        error: Some(why),
    }
}

/// Ask the feed. Never fails as a call: an unreachable feed comes back as a check with `error` set,
/// so the screen can say so and the daily task can try again tomorrow.
pub async fn check(current: &str) -> UpdateCheck {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent(format!("Stillroom/{current}"))
        .build()
    {
        Ok(c) => c,
        Err(e) => return failed(current, e.to_string()),
    };
    let response = match client.get(FEED).header("Accept", "application/vnd.github+json").send().await {
        Ok(r) => r,
        Err(e) if e.is_timeout() => return failed(current, "GitHub did not answer in 15 seconds".into()),
        Err(e) if e.is_connect() => return failed(current, "could not reach GitHub (offline?)".into()),
        Err(e) => return failed(current, e.to_string()),
    };
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return failed(current, "no release has been published yet".into());
    }
    if !response.status().is_success() {
        return failed(current, format!("GitHub answered {}", response.status()));
    }
    // A release document is a few kilobytes; a megabyte is something else.
    let body = match response.bytes().await {
        Ok(b) if b.len() <= 1 << 20 => b,
        Ok(_) => return failed(current, "the release feed was larger than expected".into()),
        Err(e) => return failed(current, e.to_string()),
    };
    match serde_json::from_slice::<Value>(&body) {
        Ok(v) => read_release(current, &v, Utc::now()),
        Err(e) => failed(current, format!("the release feed was not JSON ({e})")),
    }
}

/// Remember a check under `updates.last` and tell every window.
fn remember(app: &AppHandle, state: &AppState, found: &UpdateCheck) -> CmdResult<()> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let mut updates = store.get_setting(SETTING).map_err(|e| e.to_string())?.and_then(|v| v.as_object().cloned()).unwrap_or_default();
    updates.insert(LAST_FIELD.into(), serde_json::to_value(found).map_err(|e| e.to_string())?);
    store.set_setting(SETTING, &Value::Object(updates)).map_err(|e| e.to_string())?;
    drop(store);
    let _ = app.emit("settings:changed", Value::Object(state.settings()?));
    Ok(())
}

/// Settings › About › Check now.
#[tauri::command]
pub async fn check_for_updates(app: AppHandle, state: tauri::State<'_, AppState>) -> CmdResult<UpdateCheck> {
    let found = check(env!("CARGO_PKG_VERSION")).await;
    remember(&app, &state, &found)?;
    Ok(found)
}

/// Whether the daily check is due: it is on, and the last one is older than a day or never happened.
pub fn daily_due(updates: Option<&Value>, now: DateTime<Utc>) -> bool {
    let Some(u) = updates else { return false };
    if u["daily"].as_bool() != Some(true) {
        return false;
    }
    match u[LAST_FIELD]["checkedAt"].as_str().and_then(|s| s.parse::<DateTime<Utc>>().ok()) {
        Some(last) => now - last >= Duration::hours(23),
        None => true,
    }
}

/// The daily check: a task that looks once shortly after launch and then every hour, and asks the
/// feed only when `updates.daily` is on and a day has passed since the last answer.
pub fn start_daily(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        loop {
            let due = {
                let state = app.state::<AppState>();
                let settings = state.settings().unwrap_or_default();
                daily_due(settings.get(SETTING), Utc::now())
            };
            if due {
                let found = check(env!("CARGO_PKG_VERSION")).await;
                let state = app.state::<AppState>();
                if let Err(e) = remember(&app, &state, &found) {
                    tracing::warn!("update check not remembered: {e}");
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn a_newer_tag_is_newer_and_a_v_prefix_is_dropped() {
        let r = read_release("0.3.3", &json!({"tag_name": "v0.3.4", "html_url": "https://x/r", "body": "notes", "published_at": "2026-11-04T00:00:00Z"}), at("2026-11-04T01:00:00Z"));
        assert_eq!(r.latest.as_deref(), Some("0.3.4"));
        assert!(r.newer);
        assert_eq!(r.url.as_deref(), Some("https://x/r"));
        assert_eq!(r.notes.as_deref(), Some("notes"));
        assert!(r.error.is_none());
    }

    #[test]
    fn the_same_or_an_older_release_is_not_newer() {
        assert!(!read_release("0.3.3", &json!({"tag_name": "v0.3.3"}), Utc::now()).newer);
        assert!(!read_release("0.3.3", &json!({"tag_name": "v0.1.0"}), Utc::now()).newer);
        assert!(read_release("0.3.3", &json!({"tag_name": "1.0"}), Utc::now()).newer);
    }

    #[test]
    fn a_feed_without_a_tag_is_an_error_not_a_crash() {
        let r = read_release("0.3.3", &json!({"message": "Not Found"}), Utc::now());
        assert!(r.latest.is_none() && !r.newer);
        assert!(r.error.is_some());
    }

    #[test]
    fn long_notes_are_capped() {
        let r = read_release("0.3.3", &json!({"tag_name": "v9", "body": "x".repeat(5000)}), Utc::now());
        assert_eq!(r.notes.unwrap().chars().count(), 4001);
    }

    /// Asks the real feed. `cargo test -p stillroom --lib updates::tests::live -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn live_feed_answers_with_a_version_or_a_reason() {
        let r = check("0.3.3").await;
        println!("{r:#?}");
        assert!(r.latest.is_some() || r.error.is_some());
        if let Some(l) = &r.latest {
            assert!(l.chars().next().unwrap().is_ascii_digit(), "{l}");
        }
    }

    #[test]
    fn the_daily_check_is_due_only_when_on_and_a_day_old() {
        let now = at("2026-11-04T12:00:00Z");
        assert!(!daily_due(None, now));
        assert!(!daily_due(Some(&json!({"daily": false})), now));
        assert!(daily_due(Some(&json!({"daily": true})), now), "never checked");
        assert!(!daily_due(Some(&json!({"daily": true, "last": {"checkedAt": "2026-11-04T02:00:00Z"}})), now), "ten hours ago");
        assert!(daily_due(Some(&json!({"daily": true, "last": {"checkedAt": "2026-11-03T02:00:00Z"}})), now), "yesterday");
        assert!(daily_due(Some(&json!({"daily": true, "last": {"checkedAt": "garbage"}})), now), "an unreadable stamp counts as never");
    }
}
