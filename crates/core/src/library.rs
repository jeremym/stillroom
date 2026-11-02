//! Finds recipes on disk. Built-in recipes ship with the app; user recipes live in the app data dir.

use crate::bundle::{RecipeBundle, RecipeSource};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct RecipeLibrary {
    roots: Vec<(PathBuf, RecipeSource)>,
    /// The running app's version, which a recipe's `runtime.minVersion` is judged against.
    app_version: String,
}

#[derive(Debug, Default)]
pub struct ScanResult {
    pub bundles: Vec<RecipeBundle>,
    /// Directories that looked like recipes but failed to load, with the reason.
    pub errors: Vec<(PathBuf, String)>,
}

impl RecipeLibrary {
    /// A library judged against this crate's own version. The shell uses `with_app_version`.
    pub fn new(roots: Vec<(PathBuf, RecipeSource)>) -> Self {
        Self::with_app_version(roots, env!("CARGO_PKG_VERSION"))
    }

    pub fn with_app_version(roots: Vec<(PathBuf, RecipeSource)>, app_version: &str) -> Self {
        Self { roots, app_version: app_version.to_string() }
    }

    pub fn app_version(&self) -> &str {
        &self.app_version
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
                match RecipeBundle::load_for(&dir, *source, &self.app_version) {
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

    #[test]
    fn a_too_new_recipe_is_listed_as_an_error_in_words() {
        let tmp = std::env::temp_dir().join(format!("stillroom-lib-newer-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(tmp.join("future/prompts")).unwrap();
        std::fs::write(tmp.join("future/prompts/summarize.md"), "Summarize.").unwrap();
        let yaml = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../recipes/summarize-file/recipe.yaml")).unwrap();
        std::fs::write(tmp.join("future/recipe.yaml"), yaml.replace("license: MIT", "license: MIT\nruntime: { minVersion: \"0.5\" }")).unwrap();
        let lib = RecipeLibrary::with_app_version(vec![(tmp.clone(), RecipeSource::User)], "0.3.0");
        let r = lib.scan();
        assert!(r.bundles.is_empty());
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.errors[0].1, "This recipe needs Stillroom 0.5 or newer; you have 0.3.0.");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
