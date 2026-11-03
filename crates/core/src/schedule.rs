//! Schedules: a saved binding of a recipe to a time of day, so the recipe runs by itself each
//! morning (or on the days chosen) while the app is open. Like a watch, a schedule is a
//! binding, not part of the recipe: it holds the inputs and the model pins a run needs, and
//! the id of its last run.
//!
//! Times are local wall-clock times ("07:30" means half past seven wherever the Mac is), so a
//! schedule set in one time zone keeps its morning after a flight. `next_at` is the one
//! calculation here, kept pure so it can be tested at any fixed moment.

use crate::recipe::Recipe;
use chrono::{DateTime, Datelike, Days, NaiveTime, TimeZone, Utc, Weekday};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error)]
pub enum ScheduleError {
    #[error("{0} is not a time of day; write it as hours and minutes, like 07:30")]
    BadTime(String),
    #[error("{0} is not a day of the week; use mon, tue, wed, thu, fri, sat, or sun")]
    BadDay(String),
}

/// The days of the week as a schedule names them.
pub const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/// A schedule as it is saved.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub id: String,
    pub recipe_id: String,
    pub recipe_name: String,
    /// Every input for the run.
    pub inputs: IndexMap<String, Value>,
    pub pins: BTreeMap<String, String>,
    /// Local time of day, 24-hour, `HH:MM`.
    pub time: String,
    /// Days it applies (`mon`…`sun`). Empty means every day.
    #[serde(default)]
    pub days: Vec<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub last_fired_at: Option<DateTime<Utc>>,
    pub last_run_id: Option<String>,
}

impl Schedule {
    /// A schedule of `recipe` at `time` on `days`, after checking both read as a time and as
    /// days of the week. The time is kept as `HH:MM`; the days in week order, each once.
    pub fn new(
        recipe: &Recipe,
        time: &str,
        days: &[String],
        inputs: IndexMap<String, Value>,
        pins: BTreeMap<String, String>,
    ) -> Result<Self, ScheduleError> {
        let time = normalize_time(time)?;
        let days = normalize_days(days)?;
        Ok(Self {
            id: uuid::Uuid::now_v7().to_string(),
            recipe_id: recipe.id.clone(),
            recipe_name: recipe.name.clone(),
            inputs,
            pins,
            time,
            days,
            enabled: true,
            created_at: Utc::now(),
            last_fired_at: None,
            last_run_id: None,
        })
    }

    /// The next moment this schedule fires after `now`, in `now`'s time zone.
    pub fn next_at<Tz: TimeZone>(&self, now: DateTime<Tz>) -> Option<DateTime<Tz>> {
        next_at(&self.time, &self.days, now)
    }

    /// Whether a scheduled moment went by with no run: the first moment after the last fire
    /// (or after the schedule was made, if it never fired) is already behind `now`. This is
    /// what the app checks at launch, so a morning the app was closed for is caught up.
    pub fn missed<Tz: TimeZone>(&self, now: DateTime<Tz>) -> bool {
        let since = self.last_fired_at.unwrap_or(self.created_at).with_timezone(&now.timezone());
        match next_at(&self.time, &self.days, since) {
            Some(due) => due <= now,
            None => false,
        }
    }
}

/// `HH:MM`, 24-hour, from anything `H:MM` or `HH:MM`.
pub fn normalize_time(time: &str) -> Result<String, ScheduleError> {
    let t = parse_time(time).ok_or_else(|| ScheduleError::BadTime(time.trim().to_string()))?;
    Ok(t.format("%H:%M").to_string())
}

fn parse_time(time: &str) -> Option<NaiveTime> {
    let time = time.trim();
    let (h, m) = time.split_once(':')?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return None;
    }
    NaiveTime::from_hms_opt(h.parse().ok()?, m.parse().ok()?, 0)
}

/// Lower-case three-letter days in week order, each once; an empty list stays empty.
pub fn normalize_days(days: &[String]) -> Result<Vec<String>, ScheduleError> {
    let mut chosen: Vec<Weekday> = Vec::new();
    for d in days {
        let w = parse_day(d).ok_or_else(|| ScheduleError::BadDay(d.trim().to_string()))?;
        if !chosen.contains(&w) {
            chosen.push(w);
        }
    }
    chosen.sort_by_key(|w| w.num_days_from_monday());
    Ok(chosen.iter().map(|w| day_name(*w).to_string()).collect())
}

fn parse_day(day: &str) -> Option<Weekday> {
    let d = day.trim().to_ascii_lowercase();
    DAYS.iter().position(|n| d.starts_with(n)).and_then(|i| match i {
        0 => Some(Weekday::Mon),
        1 => Some(Weekday::Tue),
        2 => Some(Weekday::Wed),
        3 => Some(Weekday::Thu),
        4 => Some(Weekday::Fri),
        5 => Some(Weekday::Sat),
        6 => Some(Weekday::Sun),
        _ => None,
    })
}

fn day_name(w: Weekday) -> &'static str {
    DAYS[w.num_days_from_monday() as usize]
}

/// The first moment strictly after `now` that falls at `time` on one of `days` (any day when
/// `days` is empty), in `now`'s time zone: today if the time is still ahead, else the next
/// allowed day. A time that does not exist on a day (a clock jumping forward) is skipped to
/// the next allowed day. `None` when the time or every day fails to parse.
pub fn next_at<Tz: TimeZone>(time: &str, days: &[String], now: DateTime<Tz>) -> Option<DateTime<Tz>> {
    let t = parse_time(time)?;
    let allowed: Vec<Weekday> = days.iter().filter_map(|d| parse_day(d)).collect();
    if !days.is_empty() && allowed.is_empty() {
        return None;
    }
    let today = now.date_naive();
    for ahead in 0..=8u64 {
        let date = today.checked_add_days(Days::new(ahead))?;
        if !allowed.is_empty() && !allowed.contains(&date.weekday()) {
            continue;
        }
        let Some(at) = now.timezone().from_local_datetime(&date.and_time(t)).earliest() else { continue };
        if at > now {
            return Some(at);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn east(s: &str) -> DateTime<FixedOffset> {
        // UTC+2, so local and UTC clearly differ.
        DateTime::parse_from_rfc3339(&format!("{s}+02:00")).unwrap()
    }

    fn test_recipe() -> Recipe {
        Recipe::from_yaml(
            r#"
schemaVersion: "0"
id: r
name: R
inputs:
  topic: { type: text }
modelSlots: {}
steps:
  - id: list
    kind: tool
    tool: fs.list
    inputs: { path: "/tmp" }
outputs:
  files: { from: "${steps.list.output}", type: json }
"#,
        )
        .unwrap()
    }

    fn days(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn today_when_the_time_is_still_ahead_else_tomorrow() {
        // 2026-11-02 is a Monday.
        let now = east("2026-11-02T07:00:00");
        assert_eq!(next_at("07:30", &[], now), Some(east("2026-11-02T07:30:00")));
        let now = east("2026-11-02T07:30:00");
        assert_eq!(next_at("07:30", &[], now), Some(east("2026-11-03T07:30:00")), "exactly on time counts as passed");
        let now = east("2026-11-02T09:15:00");
        assert_eq!(next_at("07:30", &[], now), Some(east("2026-11-03T07:30:00")));
    }

    #[test]
    fn the_days_filter_skips_to_the_next_allowed_day() {
        let monday = east("2026-11-02T09:00:00");
        assert_eq!(next_at("07:30", &days(&["mon", "wed"]), monday), Some(east("2026-11-04T07:30:00")));
        assert_eq!(next_at("07:30", &days(&["mon"]), monday), Some(east("2026-11-09T07:30:00")), "a week on");
        let monday_early = east("2026-11-02T06:00:00");
        assert_eq!(next_at("07:30", &days(&["mon"]), monday_early), Some(east("2026-11-02T07:30:00")));
        assert_eq!(next_at("07:30", &days(&["Sunday", "SAT"]), monday), Some(east("2026-11-07T07:30:00")), "any spelling that starts with the day");
    }

    #[test]
    fn month_and_year_ends_roll_over() {
        let now = east("2026-11-30T20:00:00");
        assert_eq!(next_at("07:30", &[], now), Some(east("2026-12-01T07:30:00")));
        let now = east("2026-12-31T23:59:00");
        assert_eq!(next_at("00:00", &[], now), Some(east("2027-01-01T00:00:00")));
    }

    #[test]
    fn the_answer_is_in_the_callers_time_zone() {
        let now = east("2026-11-02T07:00:00");
        let at = next_at("07:30", &[], now).unwrap();
        assert_eq!(at.with_timezone(&Utc).to_rfc3339(), "2026-11-02T05:30:00+00:00");
    }

    #[test]
    fn bad_times_and_days_are_refused_and_good_ones_normalized() {
        assert_eq!(normalize_time("7:05").unwrap(), "07:05");
        assert_eq!(normalize_time(" 23:59 ").unwrap(), "23:59");
        assert!(normalize_time("24:00").is_err());
        assert!(normalize_time("7").is_err());
        assert!(normalize_time("07:5").is_err());
        assert!(normalize_time("morning").is_err());
        assert_eq!(normalize_days(&days(&["Fri", "mon", "friday"])).unwrap(), days(&["mon", "fri"]));
        assert!(normalize_days(&days(&["mon", "someday"])).is_err());
        assert!(normalize_days(&[]).unwrap().is_empty());
        assert_eq!(next_at("07:30", &days(&["nope"]), east("2026-11-02T07:00:00")), None);
        assert_eq!(next_at("nope", &[], east("2026-11-02T07:00:00")), None);
    }

    #[test]
    fn missed_means_a_moment_went_by_since_the_last_fire() {
        let recipe = test_recipe();
        let mut s = Schedule::new(&recipe, "07:30", &[], IndexMap::new(), BTreeMap::new()).unwrap();
        s.created_at = east("2026-11-01T20:00:00").with_timezone(&Utc);
        assert!(!s.missed(east("2026-11-01T21:00:00")), "made this evening, nothing due yet");
        assert!(s.missed(east("2026-11-02T09:00:00")), "never fired and this morning went by");
        s.last_fired_at = Some(east("2026-11-02T07:30:00").with_timezone(&Utc));
        assert!(!s.missed(east("2026-11-02T09:00:00")), "fired this morning");
        assert!(!s.missed(east("2026-11-03T07:00:00")), "tomorrow's is still ahead");
        assert!(s.missed(east("2026-11-03T08:00:00")), "yesterday's fire, this morning missed");
        // Mondays only: a Wednesday launch after Monday's fire misses nothing.
        s.days = days(&["mon"]);
        s.last_fired_at = Some(east("2026-11-02T07:30:00").with_timezone(&Utc));
        assert!(!s.missed(east("2026-11-04T12:00:00")));
        assert!(s.missed(east("2026-11-09T12:00:00")));
    }

    #[test]
    fn serializes_camel_case_for_the_webview() {
        let recipe = test_recipe();
        let s = Schedule::new(&recipe, "7:30", &days(&["mon"]), IndexMap::new(), BTreeMap::new()).unwrap();
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["recipeId"], "r");
        assert_eq!(v["recipeName"], "R");
        assert_eq!(v["time"], "07:30");
        assert_eq!(v["days"], serde_json::json!(["mon"]));
        assert_eq!(v["enabled"], true);
        assert!(v.get("createdAt").is_some() && v.get("lastFiredAt").is_some() && v.get("lastRunId").is_some());
        let back: Schedule = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
    }
}
