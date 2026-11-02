//! Consent: what a recipe may touch, said before anything runs.
//!
//! A permission entry that is `input`, `output`, or `${inputs.<name>}` only reaches what the
//! user hands the recipe at Set up, so it needs no separate yes. Every other entry (a literal
//! path, a host, `shell`, `cloudModels`, a credential) is a *literal*: it names something the
//! user did not choose at run time, and a recipe that is not built-in may not run until the
//! user has granted each one. Grants are remembered per recipe id in the store.

use crate::bundle::RecipeSource;
use crate::recipe::Recipe;
use crate::refs::{parse_ref, Ref};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What kind of thing a grant is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GrantKind {
    Read,
    Write,
    Network,
    Shell,
    CloudModels,
    Credentials,
}

impl GrantKind {
    pub fn as_str(self) -> &'static str {
        match self {
            GrantKind::Read => "read",
            GrantKind::Write => "write",
            GrantKind::Network => "network",
            GrantKind::Shell => "shell",
            GrantKind::CloudModels => "cloudModels",
            GrantKind::Credentials => "credentials",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "read" => GrantKind::Read,
            "write" => GrantKind::Write,
            "network" => GrantKind::Network,
            "shell" => GrantKind::Shell,
            "cloudModels" => GrantKind::CloudModels,
            "credentials" => GrantKind::Credentials,
            _ => return None,
        })
    }
}

/// One literal permission entry, with the sentence Set up shows for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Grant {
    pub kind: GrantKind,
    /// The entry as written in the recipe (`~/Documents`, `example.com`, `true`, a credential name).
    pub entry: String,
    /// What it means, in words: "Read everything under ~/Documents".
    pub text: String,
    /// When the user said yes (RFC 3339), on a granted entry; absent on a needed one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted_at: Option<String>,
}

impl Grant {
    pub fn new(kind: GrantKind, entry: impl Into<String>) -> Self {
        let entry = entry.into();
        let text = sentence(kind, &entry);
        Self { kind, entry, text, granted_at: None }
    }

    /// The (kind, entry) pair the store keys a grant by.
    pub fn key(&self) -> (GrantKind, &str) {
        (self.kind, self.entry.as_str())
    }
}

/// Where a recipe stands with the user: what it still needs a yes for, and what it has.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Consent {
    /// Literal entries not yet granted. Empty means the recipe may run.
    pub needed: Vec<Grant>,
    /// Literal entries the user has granted.
    pub granted: Vec<Grant>,
}

/// Whether a filesystem or network entry reaches only what the user chose at Set up.
fn is_runtime_shorthand(entry: &str) -> bool {
    entry == "input" || entry == "output" || matches!(parse_ref(entry), Some(Ok(Ref::Input { .. })))
}

/// The sentence for one grant.
pub fn sentence(kind: GrantKind, entry: &str) -> String {
    match kind {
        GrantKind::Read => format!("Read everything under {entry}"),
        GrantKind::Write => format!("Write files under {entry}"),
        GrantKind::Network => match entry {
            "*" => "Reach any site on the internet".to_string(),
            e => match e.strip_prefix("*.") {
                Some(domain) => format!("Reach {domain} and every site under it"),
                None => format!("Reach {e}"),
            },
        },
        GrantKind::Shell => "Run programs on this Mac".to_string(),
        GrantKind::CloudModels => "Send steps to a cloud model when one is chosen".to_string(),
        GrantKind::Credentials => format!("Use the credential {entry} from your keychain"),
    }
}

/// Every literal entry in a recipe's `permissions`, in the order the recipe lists them. `input`,
/// `output`, and `${inputs.x}` are never literal. A recipe with no permissions block has none.
pub fn literal_entries(recipe: &Recipe) -> Vec<Grant> {
    let mut out = Vec::new();
    let Some(perms) = recipe.permissions.as_ref() else { return out };
    let strings = |v: Option<&Value>| -> Vec<String> { v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default() };
    let fs = perms.get("filesystem");
    for e in strings(fs.and_then(|f| f.get("read"))) {
        if !is_runtime_shorthand(&e) {
            out.push(Grant::new(GrantKind::Read, e));
        }
    }
    for e in strings(fs.and_then(|f| f.get("write"))) {
        if !is_runtime_shorthand(&e) {
            out.push(Grant::new(GrantKind::Write, e));
        }
    }
    for e in strings(perms.get("network")) {
        if !is_runtime_shorthand(&e) {
            out.push(Grant::new(GrantKind::Network, e));
        }
    }
    if perms.get("shell").and_then(Value::as_bool) == Some(true) {
        out.push(Grant::new(GrantKind::Shell, "true"));
    }
    if perms.get("cloudModels").and_then(Value::as_bool) == Some(true) {
        out.push(Grant::new(GrantKind::CloudModels, "true"));
    }
    for e in strings(perms.get("credentials")) {
        out.push(Grant::new(GrantKind::Credentials, e));
    }
    out.dedup();
    out
}

/// A grant as the store remembers it: kind, entry, and when the user said yes (RFC 3339).
pub type Granted = (GrantKind, String, String);

/// Where a recipe stands, given what the store says the user granted. A built-in ships reviewed
/// and needs nothing; every other source needs each literal entry.
pub fn consent_for(recipe: &Recipe, source: RecipeSource, granted: &[Granted]) -> Consent {
    if source == RecipeSource::Builtin {
        return Consent::default();
    }
    let mut out = Consent::default();
    for mut g in literal_entries(recipe) {
        match granted.iter().find(|(k, e, _)| *k == g.kind && e == &g.entry) {
            Some((_, _, at)) => {
                g.granted_at = Some(at.clone());
                out.granted.push(g);
            }
            None => out.needed.push(g),
        }
    }
    out
}

/// The refusal a run or a watch gets while anything is still needed, or `None` when it may go ahead.
pub fn refusal(consent: &Consent) -> Option<String> {
    if consent.needed.is_empty() {
        return None;
    }
    let texts: Vec<&str> = consent.needed.iter().map(|g| g.text.as_str()).collect();
    Some(format!("This recipe needs your permission first: {}.", texts.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe(permissions: &str) -> Recipe {
        let yaml = format!(
            r#"
schemaVersion: "0"
id: t
name: T
inputs:
  folder: {{ type: directory, label: Folder }}
  site: {{ type: url, label: Site }}
modelSlots: {{}}
{permissions}
steps:
  - id: list
    kind: tool
    tool: fs.list
    inputs: {{ path: "${{inputs.folder}}" }}
outputs:
  files: {{ from: "${{steps.list.output}}", type: json }}
"#
        );
        Recipe::from_yaml(&yaml).unwrap()
    }

    #[test]
    fn shorthands_are_never_literal_and_everything_else_is() {
        let r = recipe(
            r#"permissions:
  filesystem:
    read: [input, "${inputs.folder}", "~/Documents"]
    write: [output, "~/Desktop/out"]
  network: [input, "${inputs.site}", "*", "example.com", "*.wikipedia.org"]
  shell: true
  cloudModels: true
  credentials: [github]"#,
        );
        let got = literal_entries(&r);
        let texts: Vec<&str> = got.iter().map(|g| g.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "Read everything under ~/Documents",
                "Write files under ~/Desktop/out",
                "Reach any site on the internet",
                "Reach example.com",
                "Reach wikipedia.org and every site under it",
                "Run programs on this Mac",
                "Send steps to a cloud model when one is chosen",
                "Use the credential github from your keychain",
            ]
        );
        assert_eq!(got[0].kind, GrantKind::Read);
        assert_eq!(got[5].entry, "true");
        assert_eq!(got[7].kind, GrantKind::Credentials);
    }

    #[test]
    fn a_recipe_that_only_touches_what_you_choose_needs_nothing() {
        let r = recipe(
            r#"permissions:
  filesystem:
    read: [input, "${inputs.folder}"]
    write: ["${inputs.folder}"]
  network: [input]
  shell: false"#,
        );
        assert!(literal_entries(&r).is_empty());
        assert!(literal_entries(&recipe("")).is_empty(), "no permissions block, nothing literal");
        assert_eq!(consent_for(&r, RecipeSource::User, &[]), Consent::default());
    }

    #[test]
    fn consent_splits_needed_from_granted_and_built_ins_skip_it() {
        let r = recipe(
            r#"permissions:
  filesystem:
    read: ["~/Documents"]
  network: ["example.com"]"#,
        );
        let none = consent_for(&r, RecipeSource::User, &[]);
        assert_eq!(none.needed.len(), 2);
        assert!(none.granted.is_empty());
        assert_eq!(refusal(&none).as_deref(), Some("This recipe needs your permission first: Read everything under ~/Documents, Reach example.com."));

        let at = "2026-11-02T18:00:00+00:00".to_string();
        let half = consent_for(&r, RecipeSource::Community, &[(GrantKind::Read, "~/Documents".into(), at.clone())]);
        assert_eq!(half.needed.iter().map(|g| g.entry.as_str()).collect::<Vec<_>>(), vec!["example.com"]);
        assert_eq!(half.granted.iter().map(|g| g.entry.as_str()).collect::<Vec<_>>(), vec!["~/Documents"]);
        assert_eq!(half.granted[0].granted_at.as_deref(), Some(at.as_str()), "a granted entry says when");
        assert!(half.needed[0].granted_at.is_none());
        assert_eq!(refusal(&half).as_deref(), Some("This recipe needs your permission first: Reach example.com."));
        let json = serde_json::to_value(&half).unwrap();
        assert_eq!(json["granted"][0]["grantedAt"], at);
        assert!(json["needed"][0].get("grantedAt").is_none(), "{}", json["needed"][0]);

        let all = consent_for(&r, RecipeSource::Sample, &[(GrantKind::Read, "~/Documents".into(), at.clone()), (GrantKind::Network, "example.com".into(), at.clone())]);
        assert!(all.needed.is_empty());
        assert_eq!(all.granted.len(), 2);
        assert!(refusal(&all).is_none());

        // A grant for the wrong kind does not count.
        let wrong = consent_for(&r, RecipeSource::User, &[(GrantKind::Write, "~/Documents".into(), at)]);
        assert_eq!(wrong.needed.len(), 2);

        assert_eq!(consent_for(&r, RecipeSource::Builtin, &[]), Consent::default(), "a built-in ships reviewed");
    }

    #[test]
    fn serialized_shapes_are_camel_case() {
        let g = Grant::new(GrantKind::CloudModels, "true");
        assert_eq!(serde_json::to_value(&g).unwrap(), serde_json::json!({ "kind": "cloudModels", "entry": "true", "text": "Send steps to a cloud model when one is chosen" }));
        let c = Consent { needed: vec![Grant::new(GrantKind::Network, "*")], granted: vec![] };
        assert_eq!(serde_json::to_value(&c).unwrap(), serde_json::json!({ "needed": [{ "kind": "network", "entry": "*", "text": "Reach any site on the internet" }], "granted": [] }));
        let back: Grant = serde_json::from_value(serde_json::json!({ "kind": "read", "entry": "~", "text": "whatever the UI sent" })).unwrap();
        assert_eq!(back.kind, GrantKind::Read);
        for k in [GrantKind::Read, GrantKind::Write, GrantKind::Network, GrantKind::Shell, GrantKind::CloudModels, GrantKind::Credentials] {
            assert_eq!(GrantKind::parse(k.as_str()), Some(k));
        }
    }
}
