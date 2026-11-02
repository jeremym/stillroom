//! The catalog side of the Library: copying a recipe into the user's folder (Duplicate) and
//! remembering where the copy came from (`x-source`), so the app can say "a newer version
//! exists" while the copy is still as copied, and never after the user has edited its steps.

use crate::bundle::RecipeBundle;
use crate::draft::{Draft, DraftError};
use crate::library::RecipeLibrary;
use crate::stagelib::steps_digest;
use crate::trace::Run;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// The top-level key a duplicated recipe carries: `x-source: { catalog, version, digest }`.
pub const SOURCE_KEY: &str = "x-source";

/// A saved run inside a bundle, to look at without a model: `example/result.json`, in the run
/// record shape, with any file it names kept beside it under `example/`.
pub const EXAMPLE_RESULT: &str = "example/result.json";

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("no recipe {0}")]
    NotFound(String),
    #[error("A recipe with the id “{0}” already exists; pick another.")]
    Taken(String),
    #[error("A recipe id is lowercase letters, digits, and single hyphens, like “my-recipe”; “{0}” is not.")]
    BadId(String),
    #[error("no folder for your recipes")]
    NoUserRoot,
    #[error("{path}: is a link, which a copy does not follow")]
    Symlink { path: PathBuf },
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{0}")]
    Draft(#[from] DraftError),
    #[error("copied, but the recipe did not load back: {0}")]
    LoadBack(String),
    #[error("This recipe has no example result to show.")]
    NoExample,
    #[error("{path}: the example result is not a run record: {source}")]
    BadExample { path: PathBuf, source: serde_json::Error },
}

/// Where a user recipe was copied from, as `list_recipes` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogFrom {
    /// The catalog recipe's id.
    pub catalog: String,
    /// The catalog version the copy was made from.
    pub version: String,
    /// True when the catalog now has a newer version and the user's steps are still as copied.
    pub newer: bool,
}

/// `^[a-z0-9]+(-[a-z0-9]+)*$`, the id pattern in the recipe schema.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && !id.starts_with('-') && !id.ends_with('-') && !id.contains("--") && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The `x-source` a copy of `bundle` carries, or `None` for a copy of the user's own recipe.
fn source_record(bundle: &RecipeBundle, draft: &Draft) -> Option<Value> {
    if bundle.source.is_user() {
        return None;
    }
    let steps = draft.yaml.get("steps").and_then(Value::as_array).cloned().unwrap_or_default();
    Some(json!({
        "catalog": bundle.recipe.id,
        "version": bundle.recipe.version.clone().unwrap_or_else(|| "0.0.0".into()),
        "digest": steps_digest(&steps),
    }))
}

/// Copy every regular file under `from` into `to`, keeping the layout. Dotfiles are skipped;
/// a symbolic link anywhere is refused, so nothing outside the bundle is ever copied.
fn copy_tree(from: &Path, to: &Path, skip: &[&str]) -> Result<(), CatalogError> {
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| CatalogError::Io { path, source }
    };
    std::fs::create_dir_all(to).map_err(io(to))?;
    let mut entries: Vec<_> = std::fs::read_dir(from).map_err(io(from))?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || skip.contains(&name.as_str()) {
            continue;
        }
        let path = e.path();
        let meta = std::fs::symlink_metadata(&path).map_err(io(&path))?;
        if meta.file_type().is_symlink() {
            return Err(CatalogError::Symlink { path });
        }
        let dest = to.join(&name);
        if meta.is_dir() {
            copy_tree(&path, &dest, &[])?;
        } else if meta.is_file() {
            std::fs::copy(&path, &dest).map_err(io(&path))?;
        }
    }
    Ok(())
}

/// Duplicate `recipe_id` into the user's recipes folder as `new_id`. Every file in the bundle is
/// copied (prompts, schemas, assets); `recipe.yaml` goes through `Draft`, so the checker runs,
/// with the new id, name, and icon (`None` keeps the original's), the `built-in` tag dropped,
/// and, for a copy of a catalog recipe, the author dropped and `x-source` written. An id that
/// exists already, or does not fit the id pattern, is refused before anything is written.
pub fn duplicate(library: &RecipeLibrary, recipe_id: &str, new_id: &str, name: &str, icon: Option<&str>, known_tools: &[String]) -> Result<RecipeBundle, CatalogError> {
    if !valid_id(new_id) {
        return Err(CatalogError::BadId(new_id.into()));
    }
    let scan = library.scan();
    let bundle = scan.bundles.iter().find(|b| b.recipe.id == recipe_id).ok_or_else(|| CatalogError::NotFound(recipe_id.into()))?;
    if scan.bundles.iter().any(|b| b.recipe.id == new_id) {
        return Err(CatalogError::Taken(new_id.into()));
    }
    let user_root = library.roots().iter().find(|(_, s)| s.is_user()).map(|(r, _)| r.clone()).ok_or(CatalogError::NoUserRoot)?;
    let dest = user_root.join(new_id);
    if dest.exists() {
        return Err(CatalogError::Taken(new_id.into()));
    }

    let mut draft = Draft::read(&bundle.root)?;
    let source = source_record(bundle, &draft);
    if let Some(o) = draft.yaml.as_object_mut() {
        o.insert("id".into(), Value::String(new_id.into()));
        o.insert("name".into(), Value::String(name.into()));
        if let Some(icon) = icon {
            o.insert("icon".into(), Value::String(icon.into()));
        }
        if let Some(Value::Array(tags)) = o.get_mut("tags") {
            tags.retain(|t| t.as_str() != Some("built-in"));
        }
        if !bundle.source.is_user() {
            o.remove("author");
        }
        match source {
            Some(s) => {
                o.insert(SOURCE_KEY.into(), s);
            }
            None => {
                o.remove(SOURCE_KEY);
            }
        }
    }
    // The checker runs before the copy starts, so a refused draft leaves nothing behind.
    let problems = draft.problems(known_tools);
    if !problems.is_empty() {
        return Err(DraftError::Invalid(problems).into());
    }

    let result = copy_tree(&bundle.root, &dest, &["recipe.yaml"]).and_then(|()| draft.write(&dest, known_tools).map_err(CatalogError::from));
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&dest);
        return Err(e);
    }
    library.find(new_id).ok_or_else(|| CatalogError::LoadBack(new_id.into()))
}

/// Rewrite every `example/...` path in a value to its absolute place under `root`: a `path`
/// key's string, and (when `all_strings` is set, for the run's inputs) any string.
fn absolutize(v: &mut Value, root: &Path, all_strings: bool) {
    match v {
        Value::String(s) if all_strings => {
            if let Some(rest) = s.strip_prefix("example/") {
                *s = root.join("example").join(rest).to_string_lossy().into_owned();
            }
        }
        Value::Object(m) => {
            for (k, child) in m.iter_mut() {
                absolutize(child, root, all_strings || k == "path");
            }
        }
        Value::Array(a) => {
            for child in a {
                absolutize(child, root, all_strings);
            }
        }
        _ => {}
    }
}

/// The bundle's saved example run, with every `example/` path (the run's inputs, each step's
/// inputs and output, the outputs) rewritten to its absolute place under the bundle, so the
/// ordinary viewers find the files.
pub fn example_result(bundle: &RecipeBundle) -> Result<Run, CatalogError> {
    let path = bundle.root.join(EXAMPLE_RESULT);
    if !path.is_file() {
        return Err(CatalogError::NoExample);
    }
    let text = std::fs::read_to_string(&path).map_err(|source| CatalogError::Io { path: path.clone(), source })?;
    let mut run: Run = serde_json::from_str(&text).map_err(|source| CatalogError::BadExample { path: path.clone(), source })?;
    for v in run.inputs.values_mut() {
        absolutize(v, &bundle.root, true);
    }
    for s in &mut run.steps {
        absolutize(&mut s.inputs, &bundle.root, false);
        if let Some(o) = s.output.as_mut() {
            absolutize(o, &bundle.root, false);
        }
    }
    for v in run.outputs.values_mut() {
        absolutize(v, &bundle.root, false);
    }
    Ok(run)
}

/// What `x-source` says about a user recipe, judged against the catalog as scanned: the catalog
/// id and version it was copied from, and whether a newer catalog version exists while the
/// user's steps are still the copied ones. `None` for a recipe that was not duplicated from
/// the catalog, or whose `x-source` is not in the expected shape.
pub fn provenance(bundle: &RecipeBundle, catalog: &[RecipeBundle]) -> Option<CatalogFrom> {
    if !bundle.source.is_user() {
        return None;
    }
    let src = bundle.recipe.extra.get(SOURCE_KEY)?;
    let catalog_id = src.get("catalog")?.as_str()?.to_string();
    let version = src.get("version")?.as_str()?.to_string();
    let digest = src.get("digest").and_then(Value::as_str);
    let newer = catalog
        .iter()
        .find(|b| !b.source.is_user() && b.recipe.id == catalog_id)
        .and_then(|c| {
            let shipped = c.recipe.version.as_deref()?;
            if crate::recipe::compare_versions(shipped, &version) != std::cmp::Ordering::Greater {
                return Some(false);
            }
            let digest = digest?;
            let draft = Draft::read(&bundle.root).ok()?;
            let steps = draft.yaml.get("steps").and_then(Value::as_array).cloned().unwrap_or_default();
            Some(steps_digest(&steps) == digest)
        })
        .unwrap_or(false);
    Some(CatalogFrom { catalog: catalog_id, version, newer })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::RecipeSource;

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn library(user_root: &Path) -> RecipeLibrary {
        RecipeLibrary::new(vec![
            (repo().join("recipes"), RecipeSource::Builtin),
            (repo().join("samples"), RecipeSource::Sample),
            (user_root.to_path_buf(), RecipeSource::User),
        ])
    }

    fn fresh(tag: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!("stillroom-catalog-{tag}-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&tmp).unwrap();
        tmp
    }

    #[test]
    fn a_copy_of_a_built_in_carries_x_source_and_every_file() {
        let tmp = fresh("builtin");
        let lib = library(&tmp);
        let copy = duplicate(&lib, "learning-kit", "my-kit", "My kit", Some("book"), &[]).unwrap();
        assert_eq!(copy.source, RecipeSource::User);
        assert_eq!(copy.recipe.id, "my-kit");
        assert_eq!(copy.recipe.name, "My kit");
        assert_eq!(copy.recipe.icon.as_deref(), Some("book"));
        assert!(copy.recipe.author.is_none(), "a copy of a built-in is not by Stillroom");
        assert!(!copy.recipe.tags.iter().any(|t| t == "built-in"), "{:?}", copy.recipe.tags);
        let original = lib.find("learning-kit").unwrap();
        assert_eq!(copy.prompts.len(), original.prompts.len(), "every prompt came along");
        for rel in original.prompts.keys() {
            assert!(tmp.join("my-kit").join(rel).is_file(), "{rel}");
        }
        assert!(tmp.join("my-kit/schemas").is_dir());
        let src = copy.recipe.extra.get(SOURCE_KEY).expect("x-source");
        assert_eq!(src["catalog"], "learning-kit");
        assert_eq!(src["version"], Value::String(original.recipe.version.clone().unwrap()));
        let digest = src["digest"].as_str().unwrap();
        assert!(digest.starts_with("sha256:") && digest.len() == 71, "{digest}");
        // The digest is of the steps as they read back from the copy, so an untouched copy still matches.
        let reread = Draft::read(&copy.root).unwrap();
        assert_eq!(steps_digest(reread.yaml["steps"].as_array().unwrap()), digest);
        // The YAML on disk has the key spelled as the format wants it.
        let text = std::fs::read_to_string(tmp.join("my-kit/recipe.yaml")).unwrap();
        assert!(text.contains("x-source:"), "{text}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn a_copy_of_your_own_recipe_has_no_x_source_and_keeps_the_author() {
        let tmp = fresh("user");
        let lib = library(&tmp);
        duplicate(&lib, "summarize-file", "mine", "Mine", None, &[]).unwrap();
        // Give the user's copy an author of their own, then copy that.
        let path = tmp.join("mine/recipe.yaml");
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}\nauthor:\n  name: Jeremy\n")).unwrap();
        let again = duplicate(&lib, "mine", "mine-again", "Mine again", None, &[]).unwrap();
        assert!(again.recipe.extra.get(SOURCE_KEY).is_none(), "{:?}", again.recipe.extra);
        assert_eq!(again.recipe.author.as_ref().and_then(|a| a.name.as_deref()), Some("Jeremy"));
        assert_eq!(again.recipe.icon, lib.find("summarize-file").unwrap().recipe.icon, "no icon given keeps the original's");
        assert!(provenance(&again, &lib.scan().bundles).is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn a_taken_or_malformed_id_is_refused_before_anything_is_written() {
        let tmp = fresh("taken");
        let lib = library(&tmp);
        let err = duplicate(&lib, "summarize-file", "learning-kit", "Kit", None, &[]).unwrap_err();
        assert!(matches!(err, CatalogError::Taken(_)), "{err}");
        assert!(!tmp.join("learning-kit").exists());
        duplicate(&lib, "summarize-file", "mine", "Mine", None, &[]).unwrap();
        let err = duplicate(&lib, "summarize-file", "mine", "Mine", None, &[]).unwrap_err();
        assert!(matches!(err, CatalogError::Taken(_)), "{err}");
        for bad in ["My Recipe", "-x", "x--y", "", "UPPER", "a/b"] {
            let err = duplicate(&lib, "summarize-file", bad, "Bad", None, &[]).unwrap_err();
            assert!(matches!(err, CatalogError::BadId(_)), "{bad}: {err}");
        }
        let err = duplicate(&lib, "nope", "fine", "Fine", None, &[]).unwrap_err();
        assert!(matches!(err, CatalogError::NotFound(_)), "{err}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn a_link_inside_the_bundle_is_refused_and_nothing_is_left_behind() {
        let tmp = fresh("link");
        let catalog = fresh("link-catalog");
        copy_tree(&repo().join("recipes/summarize-file"), &catalog.join("summarize-file"), &[]).unwrap();
        std::os::unix::fs::symlink("/etc", catalog.join("summarize-file/assets")).unwrap();
        let lib = RecipeLibrary::new(vec![(catalog.clone(), RecipeSource::Builtin), (tmp.clone(), RecipeSource::User)]);
        let err = duplicate(&lib, "summarize-file", "mine", "Mine", None, &[]).unwrap_err();
        assert!(matches!(err, CatalogError::Symlink { .. }), "{err}");
        assert!(!tmp.join("mine").exists(), "a refused copy leaves no half folder");
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(&catalog);
    }

    #[test]
    fn provenance_says_newer_only_while_the_steps_are_as_copied() {
        let tmp = fresh("newer");
        let catalog = fresh("newer-catalog");
        copy_tree(&repo().join("recipes/summarize-file"), &catalog.join("summarize-file"), &[]).unwrap();
        let yaml_path = catalog.join("summarize-file/recipe.yaml");
        let shipped = std::fs::read_to_string(&yaml_path).unwrap();
        let lib = RecipeLibrary::new(vec![(catalog.clone(), RecipeSource::Builtin), (tmp.clone(), RecipeSource::User)]);
        let copy = duplicate(&lib, "summarize-file", "mine", "Mine", None, &[]).unwrap();
        let from = provenance(&copy, &lib.scan().bundles).unwrap();
        let version = lib.find("summarize-file").unwrap().recipe.version.unwrap();
        assert_eq!(from, CatalogFrom { catalog: "summarize-file".into(), version: version.clone(), newer: false }, "same version: nothing newer");

        // The catalog moves on: a newer version, and the user's copy is untouched.
        let bumped = shipped.lines().map(|l| if l.starts_with("version: ") { "version: 9.0.0".to_string() } else { l.to_string() }).collect::<Vec<_>>().join("\n");
        std::fs::write(&yaml_path, &bumped).unwrap();
        let scan = lib.scan();
        let copy = scan.bundles.iter().find(|b| b.recipe.id == "mine").unwrap();
        let from = provenance(copy, &scan.bundles).unwrap();
        assert!(from.newer, "{from:?}");
        assert_eq!(from.version, version, "the copied version, not the shipped one");

        // The user edits a step: the digest no longer matches, so the card stays quiet.
        let mut draft = Draft::read(&copy.root).unwrap();
        draft.yaml["steps"][0]["name"] = Value::String("Read it my way".into());
        draft.write(&copy.root, &[]).unwrap();
        let scan = lib.scan();
        let copy = scan.bundles.iter().find(|b| b.recipe.id == "mine").unwrap();
        assert!(!provenance(copy, &scan.bundles).unwrap().newer);

        // A catalog recipe is never "from" anywhere.
        let shipped_bundle = scan.bundles.iter().find(|b| b.recipe.id == "summarize-file").unwrap();
        assert!(provenance(shipped_bundle, &scan.bundles).is_none());
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(&catalog);
    }

    #[test]
    fn the_two_shipped_examples_load_with_their_paths_made_absolute() {
        let lib = library(&fresh("examples"));
        for id in ["summarize-file", "describe-document"] {
            let b = lib.find(id).unwrap();
            assert!(b.info().example, "{id} ships an example");
            let run = example_result(&b).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(run.recipe_id, id);
            assert_eq!(run.status, crate::trace::RunStatus::Succeeded);
            let file = b.root.join("example/reading-list.md");
            assert!(file.is_file());
            assert_eq!(run.inputs["file"], Value::String(file.to_string_lossy().into_owned()), "the input path is the bundle's own copy");
            let read = run.steps.iter().find(|s| s.step_id == "read").unwrap();
            assert_eq!(read.output.as_ref().unwrap()["path"], run.inputs["file"]);
            assert_eq!(read.inputs["path"], run.inputs["file"]);
            assert!(read.output.as_ref().unwrap()["text"].as_str().unwrap().contains("Reading list on OAuth"));
            let text = serde_json::to_string(&run).unwrap();
            assert!(!text.contains("\"example/reading-list.md\""), "{id}: every example/ path was rewritten");
            assert!(run.steps.iter().any(|s| s.model.as_deref().map(|m| m.starts_with("ollama:")).unwrap_or(false)), "{id}: a model is named for the label");
            assert_eq!(run.outputs.len(), 1);
            assert!(run.outputs.values().next().unwrap().as_str().unwrap().len() > 100);
        }
        assert_eq!(example_result(&lib.find("describe-document").unwrap()).unwrap().note.as_deref(), Some("written by hand as an example"));
        assert!(example_result(&lib.find("summarize-file").unwrap()).unwrap().note.is_none());
        // A recipe without one says so, and info() agrees.
        let kit = lib.find("learning-kit").unwrap();
        assert!(!kit.info().example);
        assert!(matches!(example_result(&kit), Err(CatalogError::NoExample)));
        assert_eq!(serde_json::to_value(kit.info()).unwrap()["example"], Value::Bool(false));
    }

    #[test]
    fn serialized_from_is_camel_case() {
        let v = serde_json::to_value(CatalogFrom { catalog: "a".into(), version: "1.0.0".into(), newer: true }).unwrap();
        assert_eq!(v, json!({ "catalog": "a", "version": "1.0.0", "newer": true }));
    }
}
