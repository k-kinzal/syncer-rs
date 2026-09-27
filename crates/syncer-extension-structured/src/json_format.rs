use anyhow::{Context, Result, bail, ensure};
use json_five::rt::parser::{self, JSONKeyValuePair, JSONValue, KeyValuePairContext};
use serde_json::Value;
use std::collections::BTreeSet;
use syncer_language::Rule;

fn parse(content: &str) -> Result<parser::JSONText> {
    let mut tokens = json_five::tokenize::tokenize_rt_str(content)
        .map_err(|e| anyhow::anyhow!("invalid JSON5: {e}"))?;
    // json-five 0.3.1 reports a block comment's end at the closing slash,
    // although token ranges are exclusive. Correct only that observed case.
    for (start, kind, end) in &mut tokens.tok_spans {
        if *kind == json_five::tokenize::TokType::BlockComment
            && !content[*start..*end].ends_with("*/")
            && content
                .get(*start..*end + 1)
                .is_some_and(|text| text.ends_with("*/"))
        {
            *end += 1;
        }
    }
    let parsed = parser::from_tokens(&tokens).map_err(|e| anyhow::anyhow!("invalid JSON5: {e}"))?;
    ensure!(
        parsed.to_string() == content,
        "JSON5 syntax cannot be preserved by this parser; no content changed"
    );
    Ok(parsed)
}

fn key(pair: &JSONKeyValuePair) -> Result<String> {
    let object: Value = json_five::from_str(&format!("{{{}:null}}", pair.key))?;
    Ok(object
        .as_object()
        .context("invalid object key")?
        .keys()
        .next()
        .context("missing object key")?
        .clone())
}

fn validate(node: &JSONValue, depth: usize) -> Result<()> {
    ensure!(depth <= 128, "document nesting exceeds 128");
    match node {
        JSONValue::JSONObject {
            key_value_pairs, ..
        } => {
            let mut seen = BTreeSet::new();
            for pair in key_value_pairs {
                ensure!(seen.insert(key(pair)?), "duplicate object key");
                validate(&pair.value, depth + 1)?;
            }
        }
        JSONValue::JSONArray { values, .. } => {
            for item in values {
                validate(&item.value, depth + 1)?;
            }
        }
        JSONValue::NaN | JSONValue::Infinity => {
            bail!("non-finite JSON5 numbers are not supported by value constraints")
        }
        JSONValue::Float(number) | JSONValue::Exponent(number) => {
            ensure!(
                json_five::from_str::<f64>(number)?.is_finite(),
                "non-finite JSON5 number"
            );
        }
        JSONValue::Unary { value, .. } => validate(value, depth + 1)?,
        _ => {}
    }
    Ok(())
}

fn new_node(value: &Value) -> Result<JSONValue> {
    Ok(parse(&serde_json::to_string(value)?)?.value)
}

fn assign(node: &mut JSONValue, path: &[String], value: Option<&Value>) -> Result<()> {
    let Some((name, rest)) = path.split_first() else {
        *node = new_node(value.context("cannot remove document root")?)?;
        return Ok(());
    };
    match node {
        JSONValue::JSONObject {
            key_value_pairs, ..
        } => {
            let position = key_value_pairs
                .iter()
                .position(|pair| key(pair).is_ok_and(|key| key == *name));
            if let Some(position) = position {
                if rest.is_empty() && value.is_none() {
                    key_value_pairs.remove(position);
                } else {
                    assign(&mut key_value_pairs[position].value, rest, value)?;
                }
            } else if let Some(value) = value {
                let mut child = serde_json::json!({});
                syncer_document::assign(&mut child, rest, Some(value.clone()))?;
                if let Some(last) = key_value_pairs.last_mut() {
                    let context = last.context.get_or_insert_with(|| KeyValuePairContext {
                        wsc: (String::new(), String::new(), String::new(), None),
                    });
                    if context.wsc.3.is_none() {
                        context.wsc.3 = Some(" ".into());
                    }
                }
                key_value_pairs.push(JSONKeyValuePair {
                    key: new_node(&Value::String(name.clone()))?,
                    value: new_node(&child)?,
                    context: Some(KeyValuePairContext {
                        wsc: (String::new(), " ".into(), String::new(), None),
                    }),
                });
            }
        }
        JSONValue::JSONArray { values, .. } => {
            let index = syncer_document::index(name)?;
            ensure!(index < values.len(), "array index is out of bounds");
            if rest.is_empty() && value.is_none() {
                values.remove(index);
            } else {
                assign(&mut values[index].value, rest, value)?;
            }
        }
        _ => bail!("patch refuses to traverse a scalar parent"),
    }
    Ok(())
}

fn validate_jsonc(content: &str) -> Result<()> {
    let options = jsonc_parser::ParseOptions {
        allow_comments: true,
        allow_trailing_commas: true,
        allow_loose_object_property_names: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
    };
    jsonc_parser::parse_to_serde_value::<Value>(content, &options)?;
    Ok(())
}

pub fn document(content: &str, rule: &Rule, action: &str) -> Result<(String, bool)> {
    let source = if content.trim().is_empty() {
        "{}"
    } else {
        content
    };
    if rule.kind == "jsonc" {
        validate_jsonc(source)?;
    }
    let mut ast = parse(source)?;
    validate(&ast.value, 0)?;
    let mut expected: Value = json_five::from_str(source)?;
    let edit = syncer_document::apply(&mut expected, rule, action)?;
    if edit.replacement.is_none() {
        return Ok((content.into(), edit.compliant));
    }
    assign(
        &mut ast.value,
        &syncer_document::validate(rule)?,
        edit.replacement.as_ref().unwrap().as_ref(),
    )?;
    let output = ast.to_string();
    if rule.kind == "jsonc" {
        validate_jsonc(&output)?;
    }
    ensure!(
        json_five::from_str::<Value>(&output)? == expected,
        "JSON5 edit did not round-trip"
    );
    Ok((output, edit.compliant))
}
