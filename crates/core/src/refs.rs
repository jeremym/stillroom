//! `${inputs.x}` and `${steps.id.output.path}` references.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ref {
    Input { name: String, path: Vec<String> },
    Step { step: String, path: Vec<String> },
}

#[derive(Debug, thiserror::Error)]
pub enum RefError {
    #[error("malformed reference {0}")]
    Malformed(String),
    #[error("unknown input {0}")]
    UnknownInput(String),
    #[error("step {0} has not produced output")]
    MissingStep(String),
    #[error("path {path} not found in {root}")]
    MissingPath { root: String, path: String },
}

/// Parse a whole-string reference. Returns None if the string is not a reference at all.
pub fn parse_ref(s: &str) -> Option<Result<Ref, RefError>> {
    let inner = s.strip_prefix("${")?.strip_suffix('}')?;
    let mut parts = inner.split('.').map(str::to_string);
    let root = parts.next()?;
    let parts: Vec<String> = parts.collect();
    Some(match root.as_str() {
        "inputs" => match parts.split_first() {
            Some((name, rest)) => Ok(Ref::Input { name: name.clone(), path: rest.to_vec() }),
            None => Err(RefError::Malformed(s.into())),
        },
        "steps" => match parts.as_slice() {
            [step, output, rest @ ..] if output == "output" => {
                Ok(Ref::Step { step: step.clone(), path: rest.to_vec() })
            }
            _ => Err(RefError::Malformed(s.into())),
        },
        _ => Err(RefError::Malformed(s.into())),
    })
}

/// Everything a reference can point at during one run.
pub struct Context<'a> {
    pub inputs: &'a BTreeMap<String, Value>,
    pub step_outputs: &'a HashMap<String, Value>,
}

fn walk<'v>(mut v: &'v Value, path: &[String]) -> Option<&'v Value> {
    for p in path {
        v = match v {
            Value::Object(m) => m.get(p)?,
            Value::Array(a) => a.get(p.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(v)
}

/// Replace every whole-string reference inside `v` (recursively) with its value.
pub fn resolve_value(v: &Value, ctx: &Context) -> Result<Value, RefError> {
    match v {
        Value::String(s) => match parse_ref(s) {
            None => Ok(v.clone()),
            Some(r) => {
                let r = r?;
                let (root, root_name, path) = match &r {
                    Ref::Input { name, path } => (
                        ctx.inputs.get(name).ok_or_else(|| RefError::UnknownInput(name.clone()))?,
                        format!("inputs.{name}"),
                        path,
                    ),
                    Ref::Step { step, path } => (
                        ctx.step_outputs.get(step).ok_or_else(|| RefError::MissingStep(step.clone()))?,
                        format!("steps.{step}.output"),
                        path,
                    ),
                };
                walk(root, path).cloned().ok_or_else(|| RefError::MissingPath {
                    root: root_name,
                    path: path.join("."),
                })
            }
        },
        Value::Array(a) => a.iter().map(|x| resolve_value(x, ctx)).collect::<Result<_, _>>().map(Value::Array),
        Value::Object(m) => m
            .iter()
            .map(|(k, x)| resolve_value(x, ctx).map(|r| (k.clone(), r)))
            .collect::<Result<_, _>>()
            .map(Value::Object),
        _ => Ok(v.clone()),
    }
}

/// Step ids referenced anywhere inside `v`.
pub fn step_refs(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::String(s) => {
            if let Some(Ok(Ref::Step { step, .. })) = parse_ref(s) {
                out.insert(step);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| step_refs(x, out)),
        Value::Object(m) => m.values().for_each(|x| step_refs(x, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_input_and_step_refs() {
        assert_eq!(
            parse_ref("${inputs.file}").unwrap().unwrap(),
            Ref::Input { name: "file".into(), path: vec![] }
        );
        assert_eq!(
            parse_ref("${steps.read.output.text}").unwrap().unwrap(),
            Ref::Step { step: "read".into(), path: vec!["text".into()] }
        );
        assert!(parse_ref("plain text").is_none());
        assert!(parse_ref("${steps.read.text}").unwrap().is_err());
    }

    #[test]
    fn resolves_nested_values() {
        let inputs = BTreeMap::from([("file".to_string(), json!("/tmp/a.txt"))]);
        let mut outputs = HashMap::new();
        outputs.insert("read".to_string(), json!({ "text": "hello", "bytes": 5 }));
        let ctx = Context { inputs: &inputs, step_outputs: &outputs };
        let v = json!({ "path": "${inputs.file}", "body": "${steps.read.output.text}", "n": 1 });
        assert_eq!(
            resolve_value(&v, &ctx).unwrap(),
            json!({ "path": "/tmp/a.txt", "body": "hello", "n": 1 })
        );
        let mut deps = BTreeSet::new();
        step_refs(&v, &mut deps);
        assert_eq!(deps, BTreeSet::from(["read".to_string()]));
    }
}
