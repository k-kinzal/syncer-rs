//! JSON field patches preserve all unmanaged values. Formatting is normalized only on change.
use anyhow::{Context, Result, bail};
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
fn document(params: &Value) -> Result<Value> {
    let rule: Rule = serde_json::from_value(params["rule"].clone())?;
    let content = string(params, "content")?;
    let mut doc: Value = if content.trim().is_empty() {
        json!({})
    } else {
        syncer_document::parse_json(content).context("invalid target JSON; no content changed")?
    };
    let edit = syncer_document::apply(&mut doc, &rule, string(params, "action")?)?;
    let output = if edit.replacement.is_some() {
        format!("{}\n", serde_json::to_string_pretty(&doc)?)
    } else {
        content.into()
    };
    Ok(json!({"content":output,"compliant":edit.compliant}))
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
