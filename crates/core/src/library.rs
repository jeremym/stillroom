//! Finds recipes on disk. Built-in recipes ship with the app; user recipes live in the app data dir.

use crate::bundle::{RecipeBundle, RecipeSource};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct RecipeLibrary {
    roots: Vec<(PathBuf, RecipeSource)>,
}

#[derive(Debug, Default)]
pub struct ScanResult {
    pub bundles: Vec<RecipeBundle>,
    /// Directories that looked like recipes but failed to load, with the reason.
    pub errors: Vec<(PathBuf, String)>,
}

impl RecipeLibrary {
    pub fn new(roots: Vec<(PathBuf, RecipeSource)>) -> Self {
        Self { roots }
    }

    pub fn roots(&self) -> &[(PathBuf, RecipeSource)] {
        &self.roots
    }

    /// Every `<root>/<dir>/recipe.yaml`. User recipes with the same id as a built-in win.
    pub fn scan(&self) -> ScanResult {
        let mut result = ScanResult::default();
        for (root, source) in &self.roots {
            let Ok(entries) = std::fs::read_dir(root) else { continue };
            let mut dirs: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.join("recipe.yaml").is_file())
                .collect();
            dirs.sort();
            for dir in dirs {
                match RecipeBundle::load(&dir, *source) {
                    Ok(b) => {
                        result.bundles.retain(|x| x.recipe.id != b.recipe.id || x.source == RecipeSource::User);
                        if !result.bundles.iter().any(|x| x.recipe.id == b.recipe.id) {
                            result.bundles.push(b);
                        }
                    }
                    Err(e) => result.errors.push((dir, e.to_string())),
                }
            }
        }
        result
    }

    pub fn find(&self, id: &str) -> Option<RecipeBundle> {
        self.scan().bundles.into_iter().find(|b| b.recipe.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn scans_repo_recipes() {
        let lib = RecipeLibrary::new(vec![(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../recipes"),
            RecipeSource::Builtin,
        )]);
        let r = lib.scan();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let ids: Vec<&str> = r.bundles.iter().map(|b| b.recipe.id.as_str()).collect();
        assert!(ids.contains(&"summarize-file"));
        assert!(ids.contains(&"learning-kit"));
        assert!(lib.find("learning-kit").is_some());
    }
}
