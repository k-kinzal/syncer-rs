//! Format-independent, non-destructive value patches for document extensions.
mod json;
use anyhow::{Context, Result, bail, ensure};
pub use json::parse_json;
use serde_json::{Value, json};
use syncer_language::Rule;

/// A replacement is absent for an unchanged value; `Some(None)` means removal.
#[derive(Debug)]
pub struct Edit {
    pub compliant: bool,
    pub replacement: Option<Option<Value>>,
}

pub fn validate(rule: &Rule) -> Result<Vec<String>> {
    let path = tokens(
        rule.pointer
            .as_deref()
            .context("document rule requires pointer")?,
    )?;
    ensure!(
        matches!(
            rule.operation.as_str(),
            "set" | "ensure" | "array" | "remove"
        ),
        "document supports set, ensure, array or remove"
    );
    if rule.operation == "set" {
        ensure!(rule.value.is_some(), "set requires value");
    }
    if matches!(rule.operation.as_str(), "ensure" | "array") {
        ensure!(
            !rule.constraints.is_empty(),
            "ensure/array requires constraints"
        );
    }
    if rule.operation == "remove" {
        ensure!(!path.is_empty(), "cannot remove document root");
        ensure!(
            rule.constraints.is_empty(),
            "remove cannot have value constraints"
        );
    }
    Ok(path)
}

pub fn compliant(current: Option<&Value>, rule: &Rule, constraints_only: bool) -> bool {
    if constraints_only {
        return current.is_some_and(|v| rule.constraints.check(v).is_ok());
    }
    if rule.operation == "remove" {
        return current.is_none();
    }
    current.is_some_and(|v| {
        rule.constraints.check(v).is_ok()
            && (rule.operation != "set" || rule.value.as_ref() == Some(v))
    })
}

/// Plan one selected value. No filesystem or format-specific work occurs here.
pub fn edit(current: Option<&Value>, rule: &Rule, action: &str) -> Result<Edit> {
    validate(rule)?;
    ensure!(
        matches!(action, "apply" | "check" | "constraints"),
        "invalid document action"
    );
    let before = compliant(current, rule, action == "constraints");
    if action != "apply" || before {
        return Ok(Edit {
            compliant: before,
            replacement: None,
        });
    }
    let next = match rule.operation.as_str() {
        "set" | "ensure" => Some(rule.value.clone().context(
            "constraint violation has no repair value; provide value or repair the file",
        )?),
        "remove" => None,
        "array" => {
            let mut array = match current {
                Some(Value::Array(a)) => a.clone(),
                None => vec![],
                _ => bail!("array operation refuses to overwrite a non-array value"),
            };
            array.retain(|x| {
                !rule.constraints.forbidden_items.contains(x)
                    && rule
                        .constraints
                        .allowed_items
                        .as_ref()
                        .is_none_or(|allowed| allowed.contains(x))
            });
            if rule.constraints.empty == Some(true) {
                array.clear();
            }
            for item in &rule.constraints.required_items {
                if !array.contains(item) {
                    array.push(item.clone());
                }
            }
            Some(Value::Array(array))
        }
        _ => unreachable!(),
    };
    if let Some(value) = &next {
        rule.constraints
            .check(value)
            .context("repair value violates constraints")?;
    }
    ensure!(
        compliant(next.as_ref(), rule, false),
        "repair did not satisfy rule"
    );
    Ok(Edit {
        compliant: before,
        replacement: Some(next),
    })
}

pub fn tokens(pointer: &str) -> Result<Vec<String>> {
    if pointer.is_empty() {
        return Ok(vec![]);
    }
    ensure!(pointer.starts_with('/'), "pointer must start with '/'");
    let tokens = pointer[1..]
        .split('/')
        .map(|s| {
            let mut out = String::new();
            let mut chars = s.chars();
            while let Some(c) = chars.next() {
                if c == '~' {
                    out.push(match chars.next() {
                        Some('0') => '~',
                        Some('1') => '/',
                        _ => bail!("invalid pointer escape"),
                    });
                } else {
                    out.push(c);
                }
            }
            Ok(out)
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(tokens.len() <= 128, "pointer exceeds 128 components");
    Ok(tokens)
}

pub fn index(key: &str) -> Result<usize> {
    ensure!(
        !key.is_empty()
            && key.bytes().all(|b| b.is_ascii_digit())
            && (key == "0" || !key.starts_with('0')),
        "array index must be a canonical nonnegative integer"
    );
    key.parse().context("array index too large")
}

pub fn lookup<'a>(doc: &'a Value, path: &[String]) -> Result<Option<&'a Value>> {
    let Some((key, rest)) = path.split_first() else {
        return Ok(Some(doc));
    };
    let child = match doc {
        Value::Object(map) => map.get(key),
        Value::Array(array) => array.get(index(key)?),
        _ => bail!("patch refuses to traverse a scalar parent; select its exact pointer"),
    };
    match child {
        Some(child) => lookup(child, rest),
        None => Ok(None),
    }
}

pub fn assign(doc: &mut Value, path: &[String], value: Option<Value>) -> Result<()> {
    let Some((key, rest)) = path.split_first() else {
        *doc = value.context("cannot remove document root")?;
        return Ok(());
    };
    match doc {
        Value::Object(map) => {
            if rest.is_empty() {
                if let Some(value) = value {
                    map.insert(key.clone(), value);
                } else {
                    map.remove(key);
                }
            } else {
                if value.is_none() && !map.contains_key(key) {
                    return Ok(());
                }
                assign(
                    map.entry(key.clone()).or_insert_with(|| json!({})),
                    rest,
                    value,
                )?;
            }
        }
        Value::Array(array) => {
            let index = index(key)?;
            ensure!(index < array.len(), "array index is out of bounds");
            if rest.is_empty() {
                if let Some(value) = value {
                    array[index] = value;
                } else {
                    array.remove(index);
                }
            } else {
                assign(&mut array[index], rest, value)?;
            }
        }
        _ => bail!("patch refuses to replace a scalar parent; select its exact pointer"),
    }
    Ok(())
}

/// Apply a rule to a JSON-compatible value tree; return original compliance.
pub fn apply(doc: &mut Value, rule: &Rule, action: &str) -> Result<Edit> {
    let path = validate(rule)?;
    let plan = edit(lookup(doc, &path)?, rule, action)?;
    if let Some(next) = &plan.replacement {
        let mut candidate = doc.clone();
        assign(&mut candidate, &path, next.clone())?;
        ensure!(
            compliant(lookup(&candidate, &path)?, rule, false),
            "patch is not idempotent at this pointer; use array constraints to remove members"
        );
        *doc = candidate;
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_index_shifting_without_mutating_the_input() {
        let mut doc = json!({"items":["first","second"]});
        let original = doc.clone();
        let rule: Rule = serde_json::from_value(
            json!({"target":"x","kind":"json","operation":"remove","pointer":"/items/0"}),
        )
        .unwrap();
        assert!(apply(&mut doc, &rule, "apply").is_err());
        assert_eq!(doc, original);
    }
    #[test]
    fn enforces_canonical_indices_and_pointer_escapes() {
        let doc = json!({"a/b":{"~key":[7]}});
        assert_eq!(
            lookup(&doc, &tokens("/a~1b/~0key/0").unwrap()).unwrap(),
            Some(&json!(7))
        );
        assert!(lookup(&doc, &tokens("/a~1b/~0key/00").unwrap()).is_err());
        assert!(tokens("/bad~2escape").is_err());
    }
}
