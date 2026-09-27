//! JSON field patches preserve all unmanaged values. Formatting is normalized only on change.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use syncer_extension_sdk::{Request, string};
use syncer_language::Rule;

pub fn dispatch(request: Request) -> Result<Value> {
    match request.method.as_str() {
        "manifest" => Ok(
            json!({"abi_version":1,"name":"json","version":env!("CARGO_PKG_VERSION"),"methods":["document"],"kinds":["json"],"schemes":[],"targets":[]}),
        ),
        "document" => document(&request.params),
        _ => bail!("unsupported JSON extension method"),
    }
}
fn validate(rule: &Rule) -> Result<&str> {
    let pointer = rule
        .pointer
        .as_deref()
        .context("json rule requires pointer (RFC 6901)")?;
    tokens(pointer)?;
    ensure!(
        matches!(
            rule.operation.as_str(),
            "set" | "ensure" | "array" | "remove"
        ),
        "json supports set, ensure, array or remove"
    );
    if rule.operation == "set" {
        ensure!(rule.value.is_some(), "set requires value");
    }
    if rule.operation == "ensure" || rule.operation == "array" {
        ensure!(
            !rule.constraints.is_empty(),
            "ensure/array requires constraints"
        );
    }
    if rule.operation == "remove" {
        ensure!(
            rule.constraints.is_empty(),
            "remove cannot have value constraints"
        );
    }
    Ok(pointer)
}
fn compliant(doc: &Value, rule: &Rule, constraints_only: bool) -> Result<bool> {
    let pointer = validate(rule)?;
    let current = doc.pointer(pointer);
    if constraints_only {
        return Ok(current.is_some_and(|v| rule.constraints.check(v).is_ok()));
    }
    if rule.operation == "remove" {
        return Ok(current.is_none());
    }
    let Some(current) = current else {
        return Ok(false);
    };
    if rule.constraints.check(current).is_err() {
        return Ok(false);
    }
    Ok(rule.operation != "set" || rule.value.as_ref() == Some(current))
}
fn document(params: &Value) -> Result<Value> {
    let rule: Rule = serde_json::from_value(params["rule"].clone())?;
    let pointer = validate(&rule)?;
    let content = string(params, "content")?;
    let mut doc: Value = if content.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(content).context("invalid target JSON; no content changed")?
    };
    let action = string(params, "action")?;
    let before = compliant(&doc, &rule, action == "constraints")?;
    ensure!(
        matches!(action, "apply" | "check" | "constraints"),
        "invalid document action"
    );
    if action != "apply" || before {
        return Ok(json!({"content":content,"compliant":before}));
    }
    match rule.operation.as_str() {
        "set" | "ensure" => {
            let value = rule.value.clone().context(
                "constraint violation has no repair value; provide value or repair the file",
            )?;
            rule.constraints
                .check(&value)
                .context("repair value violates constraints")?;
            assign(&mut doc, &tokens(pointer)?, Some(value))?;
        }
        "remove" => assign(&mut doc, &tokens(pointer)?, None)?,
        "array" => {
            let mut array = match doc.pointer(pointer) {
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
            let value = Value::Array(array);
            rule.constraints
                .check(&value)
                .context("array cannot be automatically repaired")?;
            assign(&mut doc, &tokens(pointer)?, Some(value))?;
        }
        _ => unreachable!(),
    }
    ensure!(
        compliant(&doc, &rule, false)?,
        "JSON repair did not satisfy rule"
    );
    Ok(json!({"content":format!("{}\n",serde_json::to_string_pretty(&doc)?),"compliant":before}))
}
fn tokens(pointer: &str) -> Result<Vec<String>> {
    if pointer.is_empty() {
        return Ok(vec![]);
    }
    ensure!(pointer.starts_with('/'), "JSON pointer must start with '/'");
    pointer[1..]
        .split('/')
        .map(|s| {
            let mut out = String::new();
            let mut chars = s.chars();
            while let Some(c) = chars.next() {
                if c == '~' {
                    out.push(match chars.next() {
                        Some('0') => '~',
                        Some('1') => '/',
                        _ => bail!("invalid JSON pointer escape"),
                    });
                } else {
                    out.push(c);
                }
            }
            Ok(out)
        })
        .collect()
}
fn assign(doc: &mut Value, path: &[String], value: Option<Value>) -> Result<()> {
    if path.is_empty() {
        *doc = value.context("cannot remove document root")?;
        return Ok(());
    }
    let key = &path[0];
    let last = path.len() == 1;
    match doc {
        Value::Object(map) => {
            if last {
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
                    &path[1..],
                    value,
                )?;
            }
        }
        Value::Array(array) => {
            ensure!(
                key == "0" || !key.starts_with('0'),
                "array index cannot have leading zeroes"
            );
            let index: usize = key.parse().context("invalid array index")?;
            ensure!(index < array.len(), "array index is out of bounds");
            if last {
                if let Some(value) = value {
                    array[index] = value;
                } else {
                    array.remove(index);
                }
            } else {
                assign(&mut array[index], &path[1..], value)?;
            }
        }
        _ => bail!("JSON patch refuses to replace a scalar parent; select its exact pointer"),
    }
    Ok(())
}
syncer_extension_sdk::export_extension!(dispatch);

#[cfg(test)]
mod tests {
    use super::*;
    fn request(content: &str, rule: Value) -> Value {
        dispatch(Request {
            method: "document".into(),
            params: json!({"content":content,"rule":rule,"action":"apply"}),
        })
        .unwrap()
    }
    #[test]
    fn keep_personal_array_entries() {
        let out = request(
            r#"{"domains":["mine","bad"],"personal":42}"#,
            json!({"target":"x","kind":"json","operation":"array","pointer":"/domains","constraints":{"required_items":["corp"],"forbidden_items":["bad"]}}),
        );
        let doc: Value = serde_json::from_str(out["content"].as_str().unwrap()).unwrap();
        assert_eq!(doc, json!({"domains":["mine","corp"],"personal":42}));
    }
    #[test]
    fn ensure_keeps_acceptable_personal_value_and_format() {
        let s = "{\"n\": 7}";
        let out = request(
            s,
            json!({"target":"x","kind":"json","operation":"ensure","pointer":"/n","value":1,"constraints":{"min":1}}),
        );
        assert_eq!(out["content"], s);
    }
    #[test]
    fn invalid_parent_is_not_destroyed() {
        assert!(dispatch(Request{method:"document".into(),params:json!({"content":"{\"a\":1}","action":"apply","rule":{"target":"x","kind":"json","operation":"set","pointer":"/a/b","value":2}})}).is_err());
    }
}
