//! The catalog side of the Library: copying a recipe into the user's folder (Duplicate) and
//! remembering where the copy came from (`x-source`), so the app can say "a newer version
//! exists" while the copy is still as copied, and never after the user has edited its steps;
//! a bundle's saved example run; getting a recipe from a folder or a zip through quarantine; and
//! sharing one the other way as a zip.

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
    #[error("This recipe is larger than {} MB, which is more than a recipe should be; nothing was copied.", IMPORT_CAP / (1024 * 1024))]
    TooLarge,
    #[error("{path}: a hidden file or folder, which a recipe does not carry")]
    Hidden { path: PathBuf },
    #[error("{0}: a path that would land outside the recipe")]
    Escape(String),
    #[error("No recipe.yaml here: pick the folder (or zip) that holds one.")]
    NoRecipeYaml,
    #[error("that import is gone")]
    BadToken,
    #[error("{path}: not a zip Stillroom can read: {source}")]
    Zip { path: PathBuf, source: zip::result::ZipError },
}

/// The most a recipe may weigh when it comes in from a folder or a zip, unpacked.
pub const IMPORT_CAP: u64 = 64 * 1024 * 1024;

/// A recipe copied into quarantine: where it sits and how much it weighs.
#[derive(Debug, Clone)]
pub struct Imported {
    pub token: String,
    pub root: PathBuf,
    pub size_bytes: u64,
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

fn io_at(path: &Path) -> impl FnOnce(std::io::Error) -> CatalogError + '_ {
    move |source| CatalogError::Io { path: path.to_path_buf(), source }
}

/// What a Mac adds to every folder and zip, skipped quietly rather than refused.
fn is_mac_litter(name: &str) -> bool {
    name == ".DS_Store" || name == "__MACOSX"
}

/// Copy a folder file by file into `to`, checking as it goes: a link is refused, a hidden entry
/// is refused (Finder litter is skipped), and the running total is held under the cap.
fn copy_checked(from: &Path, to: &Path, total: &mut u64) -> Result<(), CatalogError> {
    std::fs::create_dir_all(to).map_err(io_at(to))?;
    let mut entries: Vec<_> = std::fs::read_dir(from).map_err(io_at(from))?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        if is_mac_litter(&name) {
            continue;
        }
        if name.starts_with('.') {
            return Err(CatalogError::Hidden { path });
        }
        let meta = std::fs::symlink_metadata(&path).map_err(io_at(&path))?;
        if meta.file_type().is_symlink() {
            return Err(CatalogError::Symlink { path });
        }
        let dest = to.join(&name);
        if meta.is_dir() {
            copy_checked(&path, &dest, total)?;
        } else if meta.is_file() {
            *total += meta.len();
            if *total > IMPORT_CAP {
                return Err(CatalogError::TooLarge);
            }
            std::fs::copy(&path, &dest).map_err(io_at(&path))?;
        }
    }
    Ok(())
}

/// Unpack a zip into `to` with the same checks: an entry that is a link, lands outside `to`
/// (`..`, an absolute path), or is hidden is refused; the unpacked total is held under the cap
/// by reading through it, since an entry's declared size is only a claim.
fn unzip_checked(zip_path: &Path, to: &Path, total: &mut u64) -> Result<(), CatalogError> {
    use std::io::Read;
    let zip_err = |source| CatalogError::Zip { path: zip_path.to_path_buf(), source };
    let file = std::fs::File::open(zip_path).map_err(io_at(zip_path))?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_err)?;
    std::fs::create_dir_all(to).map_err(io_at(to))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(zip_err)?;
        let raw = entry.name().map(|n| n.into_owned()).unwrap_or_default();
        let Some(rel) = entry.enclosed_name() else {
            return Err(CatalogError::Escape(raw));
        };
        let names: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        if names.iter().any(|n| is_mac_litter(n)) {
            continue;
        }
        if let Some(hidden) = names.iter().find(|n| n.starts_with('.')) {
            let _ = hidden;
            return Err(CatalogError::Hidden { path: rel });
        }
        if entry.is_symlink() || entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err(CatalogError::Symlink { path: rel });
        }
        let dest = to.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&dest).map_err(io_at(&dest))?;
            continue;
        }
        if entry.size() > IMPORT_CAP || *total + entry.size() > IMPORT_CAP {
            return Err(CatalogError::TooLarge);
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(io_at(parent))?;
        }
        let room = IMPORT_CAP - *total;
        let mut out = std::fs::File::create(&dest).map_err(io_at(&dest))?;
        let written = std::io::copy(&mut (&mut entry).take(room + 1), &mut out).map_err(io_at(&dest))?;
        *total += written;
        if *total > IMPORT_CAP {
            return Err(CatalogError::TooLarge);
        }
    }
    Ok(())
}

/// The folder inside an unpacked import that holds `recipe.yaml`: the top itself, or its one
/// subfolder when the zip was made of a folder (as the Finder does).
fn bundle_dir(unpacked: &Path) -> Result<PathBuf, CatalogError> {
    if unpacked.join("recipe.yaml").is_file() {
        return Ok(unpacked.to_path_buf());
    }
    let entries: Vec<PathBuf> = std::fs::read_dir(unpacked).map_err(io_at(unpacked))?.flatten().map(|e| e.path()).collect();
    match entries.as_slice() {
        [one] if one.is_dir() && one.join("recipe.yaml").is_file() => Ok(one.clone()),
        _ => Err(CatalogError::NoRecipeYaml),
    }
}

/// A quarantine token: the uuid the import was given, and nothing that could be a path.
pub fn valid_token(token: &str) -> bool {
    token.len() == 36 && token.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// Where an import sits in quarantine, once the token is known to be one.
pub fn quarantine_dir(quarantine_root: &Path, token: &str) -> Result<PathBuf, CatalogError> {
    if !valid_token(token) {
        return Err(CatalogError::BadToken);
    }
    let dir = quarantine_root.join(token);
    if !dir.join("recipe.yaml").is_file() {
        return Err(CatalogError::BadToken);
    }
    Ok(dir)
}

/// Copy a recipe from a folder or a `.zip` into `quarantine_root/<token>/`, where `recipe.yaml`
/// sits at the top. Everything is checked on the way in (see `copy_checked` and
/// `unzip_checked`); a refused import leaves nothing behind. The recipe is not yet read: the
/// caller migrates and checks it next.
pub fn import(src: &Path, quarantine_root: &Path) -> Result<Imported, CatalogError> {
    let token = uuid::Uuid::now_v7().to_string();
    let tmp = quarantine_root.join(format!("{token}.unpacking"));
    let dest = quarantine_root.join(&token);
    let mut total = 0u64;
    let result = (|| {
        std::fs::create_dir_all(quarantine_root).map_err(io_at(quarantine_root))?;
        let meta = std::fs::symlink_metadata(src).map_err(io_at(src))?;
        if meta.file_type().is_symlink() {
            return Err(CatalogError::Symlink { path: src.to_path_buf() });
        }
        if meta.is_dir() {
            copy_checked(src, &tmp, &mut total)?;
        } else if src.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("zip")).unwrap_or(false) {
            unzip_checked(src, &tmp, &mut total)?;
        } else {
            return Err(CatalogError::NoRecipeYaml);
        }
        let bundle = bundle_dir(&tmp)?;
        std::fs::rename(&bundle, &dest).map_err(io_at(&bundle))?;
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&dest);
        return Err(e);
    }
    Ok(Imported { token, root: dest, size_bytes: total })
}

/// Remove quarantine folders older than `max_age` (an import never confirmed). Returns how many.
pub fn prune_quarantine(quarantine_root: &Path, max_age: std::time::Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(quarantine_root) else { return 0 };
    let now = std::time::SystemTime::now();
    let mut n = 0;
    for e in entries.flatten() {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| now.duration_since(t).ok()).map(|age| age > max_age).unwrap_or(false);
        if old && std::fs::remove_dir_all(e.path()).is_ok() {
            n += 1;
        }
    }
    n
}

/// Move a quarantined import into the user's recipes folder as `<id>/`. An id that is taken,
/// in the catalog or among the user's own, is refused and the import stays where it is.
pub fn install(quarantine_root: &Path, token: &str, library: &RecipeLibrary) -> Result<RecipeBundle, CatalogError> {
    let root = quarantine_dir(quarantine_root, token)?;
    let bundle = RecipeBundle::load_for(&root, crate::bundle::RecipeSource::User, library.app_version()).map_err(|e| CatalogError::LoadBack(e.to_string()))?;
    let id = bundle.recipe.id.clone();
    if !valid_id(&id) {
        return Err(CatalogError::BadId(id));
    }
    if library.find(&id).is_some() {
        return Err(CatalogError::Taken(id));
    }
    let user_root = library.roots().iter().find(|(_, s)| s.is_user()).map(|(r, _)| r.clone()).ok_or(CatalogError::NoUserRoot)?;
    let dest = user_root.join(&id);
    if dest.exists() {
        return Err(CatalogError::Taken(id));
    }
    std::fs::create_dir_all(&user_root).map_err(io_at(&user_root))?;
    if std::fs::rename(&root, &dest).is_err() {
        // Another volume: copy, then drop the quarantine copy.
        copy_tree(&root, &dest, &[])?;
        let _ = std::fs::remove_dir_all(&root);
    }
    library.find(&id).ok_or_else(|| CatalogError::LoadBack(id))
}

/// Folders inside a bundle that are never shared: what runs made, not the method.
const EXPORT_SKIP_DIRS: [&str; 3] = ["runs", "keeps", "recipe-data"];

/// A `.stillroomignore` at the bundle root: one entry per line, a bundle-relative path or a bare
/// name (which matches at any depth); blank lines and `#` comments are skipped.
fn stillroomignore(root: &Path) -> Vec<String> {
    std::fs::read_to_string(root.join(".stillroomignore"))
        .map(|t| t.lines().map(|l| l.trim().trim_end_matches('/').to_string()).filter(|l| !l.is_empty() && !l.starts_with('#')).collect())
        .unwrap_or_default()
}

fn ignored(rel: &str, name: &str, ignore: &[String]) -> bool {
    ignore.iter().any(|i| i == rel || rel.starts_with(&format!("{i}/")) || i == name)
}

fn export_walk(dir: &Path, rel: &str, ignore: &[String], out: &mut Vec<String>) -> Result<(), CatalogError> {
    let mut entries: Vec<_> = std::fs::read_dir(dir).map_err(io_at(dir))?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        let child = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        if name.starts_with('.') || ignored(&child, &name, ignore) {
            continue;
        }
        let meta = std::fs::symlink_metadata(&path).map_err(io_at(&path))?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            if rel.is_empty() && EXPORT_SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            export_walk(&path, &child, ignore, out)?;
        } else if meta.is_file() {
            out.push(child);
        }
    }
    Ok(())
}

/// The bundle-relative files an export carries: everything in the bundle except hidden entries,
/// `runs/`, `keeps/`, `recipe-data/`, links (never followed), and what `.stillroomignore` names.
/// Keeps, runs, pins, and shelf state live outside the bundle, so they are never in it anyway.
pub fn export_members(root: &Path) -> Result<Vec<String>, CatalogError> {
    let mut out = Vec::new();
    export_walk(root, "", &stillroomignore(root), &mut out)?;
    Ok(out)
}

/// Write the bundle as a zip at `dest`, every member under a top folder named after the recipe
/// id (so unzipping gives a folder, and `import` strips it). Returns the members written.
pub fn export_zip(bundle: &RecipeBundle, dest: &Path) -> Result<Vec<String>, CatalogError> {
    use std::io::Write;
    let members = export_members(&bundle.root)?;
    let zip_err = |source| CatalogError::Zip { path: dest.to_path_buf(), source };
    let file = std::fs::File::create(dest).map_err(io_at(dest))?;
    let mut w = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).unix_permissions(0o644);
    for rel in &members {
        let bytes = std::fs::read(bundle.root.join(rel)).map_err(io_at(&bundle.root.join(rel)))?;
        w.start_file(format!("{}/{rel}", bundle.recipe.id), opts).map_err(zip_err)?;
        w.write_all(&bytes).map_err(io_at(dest))?;
    }
    w.finish().map_err(zip_err)?;
    Ok(members)
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

    /// A zip of these entries: a name ending in `/` is a folder, `Some(target)` makes a symbolic link.
    fn zip_of(entries: &[(&str, &[u8], Option<&str>)]) -> PathBuf {
        use std::io::Write;
        let path = fresh("zip").join("recipe.zip");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for (name, bytes, link) in entries {
            let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            if let Some(target) = link {
                w.add_symlink(*name, *target, opts).unwrap();
            } else if name.ends_with('/') {
                w.add_directory(*name, opts).unwrap();
            } else {
                w.start_file(*name, opts).unwrap();
                w.write_all(bytes).unwrap();
            }
        }
        w.finish().unwrap();
        path
    }

    fn summarize_yaml() -> String {
        std::fs::read_to_string(repo().join("recipes/summarize-file/recipe.yaml")).unwrap().replace("id: summarize-file", "id: got-this").replace("tags: [built-in, text]", "tags: [text]")
    }

    #[test]
    fn a_zipped_folder_imports_through_quarantine_and_installs_once() {
        let q = fresh("quarantine");
        let yaml = summarize_yaml();
        let zip = zip_of(&[
            ("got-this/", b"", None),
            ("got-this/recipe.yaml", yaml.as_bytes(), None),
            ("got-this/prompts/summarize.md", b"Summarize it.", None),
            ("__MACOSX/got-this/._recipe.yaml", b"litter", None),
            ("got-this/.DS_Store", b"litter", None),
        ]);
        let got = import(&zip, &q).unwrap();
        assert!(valid_token(&got.token));
        assert_eq!(got.root, q.join(&got.token));
        assert!(got.root.join("recipe.yaml").is_file(), "the single top folder was stripped");
        assert!(got.root.join("prompts/summarize.md").is_file());
        assert!(!got.root.join(".DS_Store").exists() && !q.join("__MACOSX").exists(), "Finder litter is left out");
        assert_eq!(got.size_bytes, yaml.len() as u64 + "Summarize it.".len() as u64);
        assert!(!q.join(format!("{}.unpacking", got.token)).exists());
        assert_eq!(quarantine_dir(&q, &got.token).unwrap(), got.root);

        let user = fresh("user");
        let lib = library(&user);
        let installed = install(&q, &got.token, &lib).unwrap();
        assert_eq!(installed.recipe.id, "got-this");
        assert_eq!(installed.source, RecipeSource::User);
        assert!(user.join("got-this/prompts/summarize.md").is_file());
        assert!(!got.root.exists(), "moved, not copied");
        assert!(matches!(quarantine_dir(&q, &got.token), Err(CatalogError::BadToken)), "the token is spent");

        // The same recipe again: its id is taken now, and the import stays in quarantine.
        let again = import(&zip, &q).unwrap();
        let err = install(&q, &again.token, &lib).unwrap_err();
        assert!(matches!(err, CatalogError::Taken(_)), "{err}");
        assert_eq!(err.to_string(), "A recipe with the id “got-this” already exists; pick another.");
        assert!(again.root.is_dir());
        // A built-in's id is taken too.
        let shipped = std::fs::read_to_string(repo().join("recipes/summarize-file/recipe.yaml")).unwrap();
        let b = import(&zip_of(&[("recipe.yaml", shipped.as_bytes(), None), ("prompts/summarize.md", b"S.", None)]), &q).unwrap();
        assert!(matches!(install(&q, &b.token, &lib).unwrap_err(), CatalogError::Taken(_)));
        for bad in ["", "..", "x/y", "../../etc", "0123456789abcdef0123456789abcdef0123"] {
            assert!(matches!(quarantine_dir(&q, bad), Err(CatalogError::BadToken)), "{bad}");
        }
        let _ = std::fs::remove_dir_all(&q);
        let _ = std::fs::remove_dir_all(&user);
    }

    #[test]
    fn a_folder_imports_and_a_link_or_hidden_entry_inside_it_is_refused() {
        let q = fresh("quarantine-folder");
        let src = fresh("folder");
        std::fs::create_dir_all(src.join("prompts")).unwrap();
        std::fs::write(src.join("recipe.yaml"), summarize_yaml()).unwrap();
        std::fs::write(src.join("prompts/summarize.md"), "Summarize.").unwrap();
        std::fs::write(src.join(".DS_Store"), "litter").unwrap();
        let got = import(&src, &q).unwrap();
        assert!(got.root.join("prompts/summarize.md").is_file());
        assert!(!got.root.join(".DS_Store").exists());

        std::fs::write(src.join(".secret"), "no").unwrap();
        let err = import(&src, &q).unwrap_err();
        assert!(matches!(err, CatalogError::Hidden { .. }), "{err}");
        std::fs::remove_file(src.join(".secret")).unwrap();

        std::os::unix::fs::symlink("/etc/hosts", src.join("prompts/hosts.md")).unwrap();
        let err = import(&src, &q).unwrap_err();
        assert!(matches!(err, CatalogError::Symlink { .. }), "{err}");
        let left: Vec<_> = std::fs::read_dir(&q).unwrap().flatten().filter(|e| e.path() != got.root).collect();
        assert!(left.is_empty(), "a refused import leaves nothing behind: {left:?}");

        let not_a_recipe = fresh("plain");
        std::fs::write(not_a_recipe.join("notes.txt"), "hi").unwrap();
        assert!(matches!(import(&not_a_recipe, &q).unwrap_err(), CatalogError::NoRecipeYaml));
        assert!(matches!(import(&not_a_recipe.join("notes.txt"), &q).unwrap_err(), CatalogError::NoRecipeYaml), "not a folder, not a zip");
        let _ = std::fs::remove_dir_all(&q);
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&not_a_recipe);
    }

    #[test]
    fn a_zip_with_a_link_an_escaping_path_or_too_much_in_it_is_refused() {
        let q = fresh("quarantine-bad");
        let yaml = summarize_yaml();
        let leftover = |q: &Path| std::fs::read_dir(q).map(|d| d.flatten().count()).unwrap_or(0);

        // A symbolic link (a zip entry whose mode says S_IFLNK and whose body is the target).
        let linked = zip_of(&[("recipe.yaml", yaml.as_bytes(), None), ("prompts/summarize.md", b"", Some("/etc/hosts"))]);
        let err = import(&linked, &q).unwrap_err();
        assert!(matches!(err, CatalogError::Symlink { .. }), "{err}");
        assert_eq!(leftover(&q), 0);

        // An entry that climbs out.
        let climbing = zip_of(&[("recipe.yaml", yaml.as_bytes(), None), ("../evil.md", b"x", None)]);
        let err = import(&climbing, &q).unwrap_err();
        assert!(matches!(err, CatalogError::Escape(_)), "{err}");
        assert!(!q.join("../evil.md").exists());
        assert_eq!(leftover(&q), 0);

        // An absolute path. The zip writer itself will not record one (it drops the leading
        // slash), and the reader's `enclosed_name` refuses one the same way it refuses `..`; so
        // the check here is that the entry stays inside quarantine rather than landing in /tmp.
        let absolute = zip_of(&[("recipe.yaml", yaml.as_bytes(), None), ("/tmp/evil-stillroom-test.md", b"x", None)]);
        let got = import(&absolute, &q).unwrap();
        assert!(!Path::new("/tmp/evil-stillroom-test.md").exists());
        assert!(got.root.join("tmp/evil-stillroom-test.md").is_file(), "contained under the import's own folder");
        std::fs::remove_dir_all(&got.root).unwrap();

        // A hidden entry.
        let hidden = zip_of(&[("recipe.yaml", yaml.as_bytes(), None), (".git/config", b"x", None)]);
        assert!(matches!(import(&hidden, &q).unwrap_err(), CatalogError::Hidden { .. }));

        // Over the cap: the sizes are checked as the bytes come out, not from the header alone.
        let big = vec![b'a'; (IMPORT_CAP / 2 + 1) as usize];
        let heavy = zip_of(&[("recipe.yaml", yaml.as_bytes(), None), ("assets/a.bin", &big, None), ("assets/b.bin", &big, None)]);
        let err = import(&heavy, &q).unwrap_err();
        assert!(matches!(err, CatalogError::TooLarge), "{err}");
        assert!(err.to_string().starts_with("This recipe is larger than 64 MB"), "{err}");
        assert_eq!(leftover(&q), 0);

        // Not a zip at all.
        let junk = fresh("junk").join("x.zip");
        std::fs::write(&junk, "not a zip").unwrap();
        assert!(matches!(import(&junk, &q).unwrap_err(), CatalogError::Zip { .. }));

        // No recipe.yaml anywhere, or two folders at the top.
        let none = zip_of(&[("a/notes.md", b"x", None)]);
        assert!(matches!(import(&none, &q).unwrap_err(), CatalogError::NoRecipeYaml));
        let two = zip_of(&[("a/recipe.yaml", yaml.as_bytes(), None), ("b/recipe.yaml", yaml.as_bytes(), None)]);
        assert!(matches!(import(&two, &q).unwrap_err(), CatalogError::NoRecipeYaml));
        assert_eq!(leftover(&q), 0);
        let _ = std::fs::remove_dir_all(&q);
    }

    #[test]
    fn an_export_carries_the_method_and_nothing_a_run_made_and_imports_back() {
        let user = fresh("export-user");
        let lib = library(&user);
        let copy = duplicate(&lib, "summarize-file", "shared", "Shared", None, &[]).unwrap();
        // Things that must never travel: run keeps, runs, hidden files, a link, and what the user marks private.
        std::fs::create_dir_all(copy.root.join("runs/01")).unwrap();
        std::fs::write(copy.root.join("runs/01/result.json"), "{}").unwrap();
        std::fs::create_dir_all(copy.root.join("keeps/index")).unwrap();
        std::fs::write(copy.root.join("keeps/index/db.sqlite"), "x").unwrap();
        std::fs::write(copy.root.join(".DS_Store"), "x").unwrap();
        std::fs::create_dir_all(copy.root.join("assets/private")).unwrap();
        std::fs::write(copy.root.join("assets/private/notes.md"), "mine").unwrap();
        std::fs::write(copy.root.join("assets/logo.svg"), "<svg/>").unwrap();
        std::fs::write(copy.root.join("assets/draft.txt"), "wip").unwrap();
        std::os::unix::fs::symlink("/etc/hosts", copy.root.join("assets/hosts")).unwrap();
        std::fs::write(copy.root.join(".stillroomignore"), "# keep these to yourself\nassets/private/\ndraft.txt\n").unwrap();
        assert!(copy.root.join("example/result.json").is_file(), "the duplicate brought the example along");

        let members = export_members(&copy.root).unwrap();
        assert_eq!(members, vec!["assets/logo.svg", "example/reading-list.md", "example/result.json", "prompts/summarize.md", "recipe.yaml"]);

        let dest = fresh("export-zip").join("shared.zip");
        let written = export_zip(&copy, &dest).unwrap();
        assert_eq!(written, members);
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&dest).unwrap()).unwrap();
        let names: Vec<String> = (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().unwrap().into_owned()).collect();
        assert_eq!(names, vec!["shared/assets/logo.svg", "shared/example/reading-list.md", "shared/example/result.json", "shared/prompts/summarize.md", "shared/recipe.yaml"]);

        // The zip comes back in through the import path, as another user would get it.
        let q = fresh("export-quarantine");
        let got = import(&dest, &q).unwrap();
        assert!(got.root.join("recipe.yaml").is_file() && got.root.join("prompts/summarize.md").is_file());
        assert!(!got.root.join("runs").exists() && !got.root.join("assets/private").exists());
        let other = fresh("export-other");
        let other_lib = library(&other);
        let back = install(&q, &got.token, &other_lib).unwrap();
        assert_eq!(back.recipe.id, "shared");
        for d in [&user, &other, &q] {
            let _ = std::fs::remove_dir_all(d);
        }
        let _ = std::fs::remove_dir_all(dest.parent().unwrap());
    }

    #[test]
    fn old_quarantine_folders_are_pruned() {
        let q = fresh("quarantine-prune");
        let old = q.join(uuid::Uuid::now_v7().to_string());
        let young = q.join(uuid::Uuid::now_v7().to_string());
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&young).unwrap();
        let yesterday = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 24 * 3600);
        std::fs::File::open(&old).unwrap().set_modified(yesterday).unwrap();
        assert_eq!(prune_quarantine(&q, std::time::Duration::from_secs(24 * 3600)), 1);
        assert!(!old.exists() && young.exists());
        assert_eq!(prune_quarantine(&q.join("missing"), std::time::Duration::from_secs(1)), 0);
        let _ = std::fs::remove_dir_all(&q);
    }

    #[test]
    fn serialized_from_is_camel_case() {
        let v = serde_json::to_value(CatalogFrom { catalog: "a".into(), version: "1.0.0".into(), newer: true }).unwrap();
        assert_eq!(v, json!({ "catalog": "a", "version": "1.0.0", "newer": true }));
    }
}
