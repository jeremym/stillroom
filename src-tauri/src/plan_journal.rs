//! The planner's journal: one Markdown file per plan under `<app data>/plans/`, so what the
//! planning model wrote can be studied after the window is gone. Written after every plan,
//! whether it ended in a draft or not; the newest 50 are kept.

use std::path::{Path, PathBuf};
use stillroom_core::{PlanEvent, PlanOutcome, PlanRequest};

/// How many journal files stay on disk.
pub const KEEP: usize = 50;

/// What one plan came to, in the words the journal writes.
#[derive(Debug, Clone, PartialEq)]
pub enum PlanResult {
    /// The checker found nothing; the draft came back ready to save.
    Clean,
    /// The draft came back with these problems still in it.
    WithProblems(Vec<String>),
    /// No draft: cancelled, out of time, or the model never answered usably.
    Failed(String),
}

/// Everything the journal writes about one plan.
#[derive(Debug, Clone)]
pub struct PlanRecord {
    pub request: PlanRequest,
    pub model_id: String,
    /// The revision of each system prompt in use, `(name, revision)`; 0 is the original.
    pub prompt_revisions: Vec<(String, u32)>,
    pub attempts: u32,
    pub duration_ms: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// The checker's problems on each attempt that got a fix round, `(attempt, problems)`.
    pub fix_rounds: Vec<(u32, Vec<String>)>,
    pub result: PlanResult,
    pub transcript: String,
    pub started_at: chrono::DateTime<chrono::Local>,
    /// The custom stages were composed one call each (a small or tight model).
    pub per_stage: bool,
}

impl PlanRecord {
    /// A record from a finished plan.
    pub fn from_outcome(request: &PlanRequest, out: &PlanOutcome, prompt_revisions: Vec<(String, u32)>, fix_rounds: Vec<(u32, Vec<String>)>, started_at: chrono::DateTime<chrono::Local>) -> Self {
        Self {
            request: request.clone(),
            model_id: out.model_id.clone(),
            prompt_revisions,
            attempts: out.attempts,
            duration_ms: out.duration_ms,
            prompt_tokens: out.prompt_tokens,
            completion_tokens: out.completion_tokens,
            fix_rounds,
            result: if out.problems.is_empty() { PlanResult::Clean } else { PlanResult::WithProblems(out.problems.clone()) },
            transcript: out.transcript.clone(),
            started_at,
            per_stage: out.per_stage,
        }
    }

    /// The file name: `<yyyy-mm-dd-hhmmss>-<recipe id>.md`, the id reduced to safe characters.
    pub fn file_name(&self) -> String {
        format!("{}-{}.md", self.started_at.format("%Y-%m-%d-%H%M%S"), safe_id(&self.request.id))
    }

    /// The journal entry as Markdown.
    pub fn render(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("# Plan: {}\n\n", self.request.name));
        s.push_str(&format!("- **When:** {}\n", self.started_at.format("%Y-%m-%d %H:%M:%S")));
        s.push_str(&format!("- **Recipe id:** `{}`\n", self.request.id));
        s.push_str(&format!("- **Model:** `{}`\n", self.model_id));
        let revisions: Vec<String> = self.prompt_revisions.iter().map(|(n, r)| if *r == 0 { format!("{n} (original)") } else { format!("{n} (revision {r})") }).collect();
        s.push_str(&format!("- **Prompts:** {}\n", if revisions.is_empty() { "unknown".to_string() } else { revisions.join(", ") }));
        s.push_str(&format!("- **Attempts:** {}\n", self.attempts));
        if self.per_stage {
            s.push_str("- **Mode:** one call per custom stage (a small or tight model)\n");
        }
        s.push_str(&format!("- **Duration:** {}\n", fmt_duration(self.duration_ms)));
        s.push_str(&format!("- **Tokens:** {} read, {} written\n", self.prompt_tokens, self.completion_tokens));
        s.push_str(&format!("- **Result:** {}\n\n", self.result_line()));

        s.push_str("## Goal\n\n");
        s.push_str(self.request.goal.trim());
        s.push_str("\n\n");

        s.push_str("## Material\n\n");
        match self.request.material.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
            Some(m) => s.push_str(m),
            None => s.push_str("_none_"),
        }
        s.push_str("\n\n");

        s.push_str("## Problems the checker found\n\n");
        if self.fix_rounds.is_empty() && !matches!(self.result, PlanResult::WithProblems(_)) {
            s.push_str("_none_\n\n");
        } else {
            for (attempt, problems) in &self.fix_rounds {
                s.push_str(&format!("### Attempt {attempt}, sent back to the model\n\n"));
                for p in problems {
                    s.push_str(&format!("- {p}\n"));
                }
                s.push('\n');
            }
            if let PlanResult::WithProblems(problems) = &self.result {
                s.push_str("### Still in the draft\n\n");
                for p in problems {
                    s.push_str(&format!("- {p}\n"));
                }
                s.push('\n');
            }
        }

        s.push_str("## Transcript\n\n");
        if self.transcript.trim().is_empty() {
            s.push_str("_the model wrote nothing_\n");
        } else {
            s.push_str("```text\n");
            s.push_str(self.transcript.trim_end());
            s.push_str("\n```\n");
        }
        s
    }

    fn result_line(&self) -> String {
        match &self.result {
            PlanResult::Clean => "clean draft, ready to save".to_string(),
            PlanResult::WithProblems(p) => format!("draft with {} problem{} left (not saved as it is)", p.len(), if p.len() == 1 { "" } else { "s" }),
            PlanResult::Failed(why) => format!("failed: {why}"),
        }
    }
}

/// Collects, from the planner's events, what the journal needs when the plan does not return an
/// outcome: the fix rounds and a transcript rebuilt from Phase and Delta events.
#[derive(Debug, Default)]
pub struct EventLog {
    pub fix_rounds: Vec<(u32, Vec<String>)>,
    pub transcript: String,
    pub attempts: u32,
    pub last_elapsed_ms: u64,
    pub tokens: u64,
}

impl EventLog {
    pub fn see(&mut self, e: &PlanEvent) {
        match e {
            PlanEvent::Fixing { attempt, problems, elapsed_ms, tokens } => {
                self.fix_rounds.push((*attempt, problems.clone()));
                self.note(*elapsed_ms, *tokens);
            }
            PlanEvent::Phase { which, attempt, elapsed_ms, tokens, .. } => {
                if !self.transcript.is_empty() {
                    self.transcript.push_str("\n\n");
                }
                self.transcript.push_str(&format!("── call {} ({}) ──\n", self.calls() + 1, serde_json::to_value(which).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()));
                self.attempts = self.attempts.max(*attempt);
                self.note(*elapsed_ms, *tokens);
            }
            PlanEvent::Delta { text, elapsed_ms, tokens } => {
                self.transcript.push_str(text);
                self.note(*elapsed_ms, *tokens);
            }
            PlanEvent::Choosing { elapsed_ms, tokens }
            | PlanEvent::Model { elapsed_ms, tokens, .. }
            | PlanEvent::Thinking { elapsed_ms, tokens, .. }
            | PlanEvent::Checking { elapsed_ms, tokens, .. }
            | PlanEvent::Done { elapsed_ms, tokens } => self.note(*elapsed_ms, *tokens),
        }
    }

    fn calls(&self) -> usize {
        self.transcript.matches("── call ").count()
    }

    fn note(&mut self, elapsed_ms: u64, tokens: u64) {
        self.last_elapsed_ms = self.last_elapsed_ms.max(elapsed_ms);
        self.tokens = self.tokens.max(tokens);
    }
}

/// Write the record into `dir` and drop the oldest files past [`KEEP`]. Returns the path written.
pub fn write(dir: &Path, record: &PlanRecord) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(record.file_name());
    std::fs::write(&path, record.render())?;
    prune(dir, KEEP)?;
    Ok(path)
}

/// Keep the newest `keep` journal files (by name, which starts with the date) and remove the rest.
pub fn prune(dir: &Path, keep: usize) -> std::io::Result<()> {
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    let extra = names.len().saturating_sub(keep);
    for name in names.into_iter().take(extra) {
        let _ = std::fs::remove_file(dir.join(name));
    }
    Ok(())
}

fn safe_id(id: &str) -> String {
    let s: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "recipe".to_string() } else { s.chars().take(60).collect() }
}

fn fmt_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.1} s", ms as f64 / 1000.0)
    } else {
        format!("{} min {} s", ms / 60_000, (ms % 60_000) / 1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use stillroom_core::Draft;

    /// A folder that is removed when dropped (no tempfile crate here).
    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("stillroom-plans-{}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn request() -> PlanRequest {
        PlanRequest { goal: "Sort my inbox into folders".into(), material: Some("Folders: bills, family".into()), id: "inbox-sort".into(), name: "Inbox sort".into(), icon: None }
    }

    fn outcome(problems: Vec<String>) -> PlanOutcome {
        PlanOutcome {
            draft: Draft { yaml: serde_json::json!({}), files: Default::default() },
            problems,
            schema_version: "1".into(),
            attempts: 2,
            model_id: "ollama:qwen3:8b".into(),
            prompt_tokens: 1200,
            completion_tokens: 340,
            duration_ms: 48_500,
            transcript: "── call 1 (strip) ──\n{\"stages\":[]}\n\n── call 2 (steps) ──\n{\"inputs\":{}}".into(),
            per_stage: false,
        }
    }

    fn when() -> chrono::DateTime<chrono::Local> {
        chrono::Local.with_ymd_and_hms(2026, 11, 2, 14, 5, 9).unwrap()
    }

    #[test]
    fn the_file_is_named_by_time_and_recipe_id() {
        let r = PlanRecord::from_outcome(&request(), &outcome(vec![]), vec![], vec![], when());
        assert_eq!(r.file_name(), "2026-11-02-140509-inbox-sort.md");
        let mut odd = r.clone();
        odd.request.id = "../weird id!".into();
        assert_eq!(odd.file_name(), "2026-11-02-140509-weird-id.md");
    }

    #[test]
    fn a_fake_outcome_renders_every_section() {
        let fixes = vec![(1, vec!["step `write` names an input that does not exist".to_string()])];
        let r = PlanRecord::from_outcome(&request(), &outcome(vec!["the output `done` is never written".into()]), vec![("strip".into(), 0), ("planner".into(), 2)], fixes, when());
        let md = r.render();
        let want = [
            "# Plan: Inbox sort",
            "- **Recipe id:** `inbox-sort`",
            "- **Model:** `ollama:qwen3:8b`",
            "- **Prompts:** strip (original), planner (revision 2)",
            "- **Attempts:** 2",
            "- **Duration:** 48.5 s",
            "- **Tokens:** 1200 read, 340 written",
            "- **Result:** draft with 1 problem left (not saved as it is)",
            "## Goal\n\nSort my inbox into folders",
            "## Material\n\nFolders: bills, family",
            "## Problems the checker found",
            "### Attempt 1, sent back to the model\n\n- step `write` names an input that does not exist",
            "### Still in the draft\n\n- the output `done` is never written",
            "## Transcript\n\n```text\n── call 1 (strip) ──",
        ];
        for w in want {
            assert!(md.contains(w), "missing {w:?} in:\n{md}");
        }
    }

    #[test]
    fn a_clean_plan_without_material_says_so() {
        let mut req = request();
        req.material = None;
        let r = PlanRecord::from_outcome(&req, &outcome(vec![]), vec![], vec![], when());
        let md = r.render();
        assert!(md.contains("- **Result:** clean draft, ready to save"));
        assert!(md.contains("## Material\n\n_none_"));
        assert!(md.contains("## Problems the checker found\n\n_none_"));
    }

    #[test]
    fn a_failed_plan_is_rebuilt_from_events() {
        let mut log = EventLog::default();
        log.see(&PlanEvent::Phase { which: stillroom_core::planner::PlanPhase::Strip, attempt: 1, of: 1, elapsed_ms: 10, tokens: 0 });
        log.see(&PlanEvent::Delta { text: "{\"stages\"".into(), elapsed_ms: 500, tokens: 3 });
        log.see(&PlanEvent::Fixing { attempt: 1, problems: vec!["no stages".into()], elapsed_ms: 900, tokens: 3 });
        log.see(&PlanEvent::Phase { which: stillroom_core::planner::PlanPhase::Fix, attempt: 2, of: 3, elapsed_ms: 1000, tokens: 3 });
        log.see(&PlanEvent::Delta { text: "...".into(), elapsed_ms: 2000, tokens: 9 });
        assert_eq!(log.transcript, "── call 1 (strip) ──\n{\"stages\"\n\n── call 2 (fix) ──\n...");
        assert_eq!(log.fix_rounds, vec![(1, vec!["no stages".to_string()])]);
        assert_eq!(log.attempts, 2);
        let r = PlanRecord {
            request: request(),
            model_id: "ollama:qwen3:8b".into(),
            prompt_revisions: vec![],
            attempts: log.attempts,
            duration_ms: log.last_elapsed_ms,
            prompt_tokens: 0,
            completion_tokens: log.tokens,
            fix_rounds: log.fix_rounds.clone(),
            result: PlanResult::Failed("stopped".into()),
            transcript: log.transcript.clone(),
            started_at: when(),
            per_stage: false,
        };
        let md = r.render();
        assert!(md.contains("- **Result:** failed: stopped"));
        assert!(md.contains("- **Duration:** 2.0 s"));
        assert!(md.contains("── call 2 (fix) ──"));
    }

    #[test]
    fn write_keeps_only_the_newest_files() {
        let dir = TempDir::new();
        for i in 0..(KEEP + 3) {
            let mut r = PlanRecord::from_outcome(&request(), &outcome(vec![]), vec![], vec![], when() + chrono::Duration::seconds(i as i64));
            r.request.id = format!("r{i}");
            write(dir.path(), &r).unwrap();
        }
        let mut names: Vec<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        assert_eq!(names.len(), KEEP);
        assert!(names[0].ends_with("-r3.md"), "the three oldest went: {}", names[0]);
        assert!(std::fs::read_to_string(dir.path().join(&names[0])).unwrap().starts_with("# Plan: Inbox sort"));
    }
}
