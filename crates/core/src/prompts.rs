//! System prompts the engine itself runs on (the planner, later the step envelope), as files the
//! user can read, edit, and reset. The original text ships inside the app; the user's revisions
//! live under `<dir>/<name>/v<n>.md`, and `<dir>/<name>/current` names the revision in use
//! (`0` or no file means the original). Nothing is ever overwritten: saving makes a new revision,
//! resetting points back at the original and keeps the history.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The planner prompt as shipped with the app (`prompts/planner.md` in the repository).
pub static PLANNER_PROMPT: &str = include_str!("../../../prompts/planner.md");

/// A prompt the engine knows about: its id, what it is for, and the text it ships with.
#[derive(Debug, Clone, Copy)]
pub struct BuiltinPrompt {
    pub name: &'static str,
    pub title: &'static str,
    pub purpose: &'static str,
    pub original: &'static str,
}

/// The strip prompt as shipped (`prompts/strip.md`): the first, short planning call.
pub static STRIP_PROMPT: &str = include_str!("../../../prompts/strip.md");

/// The edit prompt as shipped (`prompts/edit.md`): changing a recipe from an instruction.
pub static EDIT_PROMPT: &str = include_str!("../../../prompts/edit.md");

pub const BUILTIN_PROMPTS: &[BuiltinPrompt] = &[
    BuiltinPrompt {
        name: "strip",
        title: "Strip",
        purpose: "The first planning call: picks the stages of the recipe, ready-made ones by id and custom ones by name and why. The list of ready-made stages is appended at run time.",
        original: STRIP_PROMPT,
    },
    BuiltinPrompt {
        name: "planner",
        title: "Planner",
        purpose: "The second planning call: writes the steps, prompts, and outputs for the stages that are not ready-made. The list of tools and the answer format are appended at run time.",
        original: PLANNER_PROMPT,
    },
    BuiltinPrompt {
        name: "edit",
        title: "Edit",
        purpose: "Ask for a change on the Recipe card: the recipe as it is and one instruction go in, the whole recipe with only that change comes out. The planner prompt's format section and the tool list are appended at run time.",
        original: EDIT_PROMPT,
    },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptVersion {
    /// 0 is the original; 1, 2, … are the user's revisions in the order they were saved.
    pub n: u32,
    pub saved_at: Option<DateTime<Utc>>,
}

/// Everything the UI shows for one prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptState {
    pub name: String,
    pub title: String,
    pub purpose: String,
    /// The text in use right now.
    pub text: String,
    /// The revision in use (0 = original).
    pub current: u32,
    pub original: String,
    pub versions: Vec<PromptVersion>,
}

#[derive(Debug, thiserror::Error)]
pub enum PromptError {
    #[error("no prompt named {0}")]
    Unknown(String),
    #[error("{0} has no revision {1}")]
    NoVersion(String, u32),
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> PromptError + '_ {
    move |source| PromptError::Io { path: path.to_path_buf(), source }
}

/// The user's prompt revisions on disk.
#[derive(Debug, Clone)]
pub struct SystemPrompts {
    dir: PathBuf,
}

impl SystemPrompts {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn names() -> Vec<&'static str> {
        BUILTIN_PROMPTS.iter().map(|p| p.name).collect()
    }

    fn builtin(name: &str) -> Result<&'static BuiltinPrompt, PromptError> {
        BUILTIN_PROMPTS.iter().find(|p| p.name == name).ok_or_else(|| PromptError::Unknown(name.to_string()))
    }

    fn folder(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn version_path(&self, name: &str, n: u32) -> PathBuf {
        self.folder(name).join(format!("v{n}.md"))
    }

    /// The saved revisions, oldest first, without the original.
    fn saved_versions(&self, name: &str) -> Vec<PromptVersion> {
        let Ok(entries) = std::fs::read_dir(self.folder(name)) else { return Vec::new() };
        let mut out: Vec<PromptVersion> = entries
            .flatten()
            .filter_map(|e| {
                let file = e.file_name().to_string_lossy().into_owned();
                let n: u32 = file.strip_prefix('v')?.strip_suffix(".md")?.parse().ok()?;
                let saved_at = e.metadata().ok().and_then(|m| m.modified().ok()).map(DateTime::<Utc>::from);
                Some(PromptVersion { n, saved_at })
            })
            .collect();
        out.sort_by_key(|v| v.n);
        out
    }

    fn current_n(&self, name: &str) -> u32 {
        std::fs::read_to_string(self.folder(name).join("current")).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0)
    }

    fn set_current(&self, name: &str, n: u32) -> Result<(), PromptError> {
        let folder = self.folder(name);
        std::fs::create_dir_all(&folder).map_err(io(&folder))?;
        let path = folder.join("current");
        std::fs::write(&path, n.to_string()).map_err(io(&path))
    }

    /// The text of one revision; 0 is the original.
    pub fn text_of(&self, name: &str, n: u32) -> Result<String, PromptError> {
        let b = Self::builtin(name)?;
        if n == 0 {
            return Ok(b.original.to_string());
        }
        let path = self.version_path(name, n);
        std::fs::read_to_string(&path).map_err(|_| PromptError::NoVersion(name.to_string(), n))
    }

    /// The text in use: the current revision, or the original when that revision is gone.
    pub fn current_text(&self, name: &str) -> Result<String, PromptError> {
        let n = self.current_n(name);
        match self.text_of(name, n) {
            Ok(t) => Ok(t),
            Err(PromptError::NoVersion(..)) => self.text_of(name, 0),
            Err(e) => Err(e),
        }
    }

    pub fn state(&self, name: &str) -> Result<PromptState, PromptError> {
        let b = Self::builtin(name)?;
        let mut versions = vec![PromptVersion { n: 0, saved_at: None }];
        versions.extend(self.saved_versions(name));
        let mut current = self.current_n(name);
        if !versions.iter().any(|v| v.n == current) {
            current = 0;
        }
        Ok(PromptState {
            name: b.name.to_string(),
            title: b.title.to_string(),
            purpose: b.purpose.to_string(),
            text: self.text_of(name, current)?,
            current,
            original: b.original.to_string(),
            versions,
        })
    }

    pub fn states(&self) -> Vec<PromptState> {
        BUILTIN_PROMPTS.iter().filter_map(|b| self.state(b.name).ok()).collect()
    }

    /// Save `text` as a new revision and make it current. Text equal to what is current is not
    /// saved twice; text equal to the original just resets.
    pub fn save(&self, name: &str, text: &str) -> Result<PromptState, PromptError> {
        let b = Self::builtin(name)?;
        let text = text.replace("\r\n", "\n");
        if text.trim() == b.original.trim() {
            self.set_current(name, 0)?;
            return self.state(name);
        }
        if self.current_text(name)?.trim() == text.trim() {
            return self.state(name);
        }
        let next = self.saved_versions(name).last().map(|v| v.n + 1).unwrap_or(1);
        let path = self.version_path(name, next);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        std::fs::write(&path, &text).map_err(io(&path))?;
        self.set_current(name, next)?;
        self.state(name)
    }

    /// Use revision `n` (0 resets to the original). The other revisions stay on disk.
    pub fn use_version(&self, name: &str, n: u32) -> Result<PromptState, PromptError> {
        Self::builtin(name)?;
        if n != 0 && !self.version_path(name, n).is_file() {
            return Err(PromptError::NoVersion(name.to_string(), n));
        }
        self.set_current(name, n)?;
        self.state(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> (SystemPrompts, PathBuf) {
        let dir = std::env::temp_dir().join(format!("stillroom-prompts-{}", uuid::Uuid::now_v7()));
        (SystemPrompts::new(&dir), dir)
    }

    #[test]
    fn original_until_saved_then_versions_then_reset() {
        let (p, dir) = fresh();
        let s = p.state("planner").unwrap();
        assert_eq!(s.current, 0);
        assert_eq!(s.text, PLANNER_PROMPT);
        assert_eq!(s.versions.len(), 1);

        let s = p.save("planner", "first edit").unwrap();
        assert_eq!(s.current, 1);
        assert_eq!(s.text, "first edit");
        assert_eq!(p.current_text("planner").unwrap(), "first edit");

        // Saving the same text again makes no new revision.
        let s = p.save("planner", "first edit").unwrap();
        assert_eq!(s.versions.len(), 2);

        let s = p.save("planner", "second edit").unwrap();
        assert_eq!(s.current, 2);
        assert_eq!(s.versions.iter().map(|v| v.n).collect::<Vec<_>>(), vec![0, 1, 2]);

        // Reset keeps history and points at the original.
        let s = p.use_version("planner", 0).unwrap();
        assert_eq!(s.current, 0);
        assert_eq!(s.text, PLANNER_PROMPT);
        assert_eq!(s.versions.len(), 3);

        // An older revision can be picked again.
        let s = p.use_version("planner", 1).unwrap();
        assert_eq!(s.text, "first edit");
        assert!(matches!(p.use_version("planner", 9), Err(PromptError::NoVersion(_, 9))));

        // Saving the original text is a reset, not a revision.
        let s = p.save("planner", PLANNER_PROMPT).unwrap();
        assert_eq!(s.current, 0);
        assert_eq!(s.versions.len(), 3);

        assert!(matches!(p.state("nope"), Err(PromptError::Unknown(_))));
        let _ = std::fs::remove_dir_all(dir);
    }
}
