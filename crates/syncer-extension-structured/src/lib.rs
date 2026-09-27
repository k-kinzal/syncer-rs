//! Native structured document adapters for Syncer.
mod hcl_format;
mod json_format;
mod lines;
mod plist_format;
mod tables;
mod toml_format;
mod xml_format;

use anyhow::{Result, bail, ensure};
use serde_json::{Value, json};
use syncer_extension_sdk::{Request, string};
use syncer_language::Rule;

pub const KINDS: &[&str] = &[
    "yaml",
    "toml",
    "hcl",
    "xml",
    "jsonc",
    "json5",
    "ini",
    "dotenv",
    "properties",
    "csv",
    "tsv",
    "plist",
];

pub fn dispatch(request: Request) -> Result<Value> {
    match request.method.as_str() {
        "manifest" => Ok(
            json!({"abi_version":1,"name":"structured","version":env!("CARGO_PKG_VERSION"),"methods":["document"],"kinds":KINDS,"schemes":[],"targets":[]}),
        ),
        "document" => {
            let rule: Rule = serde_json::from_value(request.params["rule"].clone())?;
            let content = string(&request.params, "content")?;
            let action = string(&request.params, "action")?;
            ensure!(
                content.len() <= syncer_extension_sdk::MAX_BYTES,
                "document too large"
            );
            syncer_document::validate(&rule)?;
            let (output, compliant) = match rule.kind.as_str() {
                "yaml" => yaml(content, &rule, action)?,
                "jsonc" | "json5" => json_format::document(content, &rule, action)?,
                "toml" => toml_format::document(content, &rule, action)?,
                "hcl" => hcl_format::document(content, &rule, action)?,
                "xml" | "plist" => xml_format::document(content, &rule, action)?,
                "ini" | "dotenv" | "properties" => lines::document(content, &rule, action)?,
                "csv" | "tsv" => tables::document(content, &rule, action)?,
                _ => bail!("unsupported structured document kind"),
            };
            ensure!(
                output.len() <= syncer_extension_sdk::MAX_BYTES,
                "edited document too large"
            );
            Ok(json!({"content":output,"compliant":compliant}))
        }
        _ => bail!("unsupported structured extension method"),
    }
}

fn yaml(content: &str, rule: &Rule, action: &str) -> Result<(String, bool)> {
    let mut document = yaml_serde_edit::YamlObject::<Value>::parse(if content.trim().is_empty() {
        "{}\n"
    } else {
        content
    })?;
    let mut value = document.get().clone();
    let edit = syncer_document::apply(&mut value, rule, action)?;
    if edit.replacement.is_none() {
        return Ok((content.into(), edit.compliant));
    }
    document.set(value.clone())?;
    let output = document.get_string().to_string();
    let checked = yaml_serde_edit::YamlObject::<Value>::parse(&output)?;
    ensure!(checked.get() == &value, "YAML edit did not round-trip");
    let comments = |text: &str| {
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for (kind, token) in yaml_edit::lex(text) {
            if kind == yaml_edit::SyntaxKind::COMMENT {
                *counts.entry(token.into()).or_default() += 1;
            }
        }
        counts
    };
    let after_comments = comments(&output);
    ensure!(
        comments(content)
            .iter()
            .all(|(comment, count)| after_comments
                .get(comment)
                .is_some_and(|after| after >= count)),
        "YAML edit would discard comments; select a narrower field or edit this construct manually"
    );
    Ok((output, edit.compliant))
}

syncer_extension_sdk::export_extension!(dispatch);
