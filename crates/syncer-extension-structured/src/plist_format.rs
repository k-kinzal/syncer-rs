use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use plist::Value as Plist;
use serde_json::{Value, json};
use syncer_language::Rule;

fn parse(content: &str) -> Result<Plist> {
    // Apple XML plists conventionally include a DOCTYPE. roxmltree performs no
    // external resolution without a resolver; plist's reader also does no I/O.
    let syntax = roxmltree::Document::parse_with_options(
        content,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            nodes_limit: 500_000,
            ..Default::default()
        },
    )?;
    ensure!(
        syntax.root_element().tag_name().name() == "plist",
        "XML plist requires a plist root"
    );
    for node in syntax.descendants().filter(|node| node.is_element()) {
        ensure!(
            node.tag_name().namespace().is_none(),
            "namespaced plist elements are unsupported"
        );
        if node.tag_name().name() != "dict" {
            continue;
        }
        let mut keys = std::collections::BTreeSet::new();
        let children: Vec<_> = node.children().filter(|child| child.is_element()).collect();
        ensure!(
            children.len().is_multiple_of(2),
            "plist dictionary must contain key/value pairs"
        );
        for pair in children.chunks_exact(2) {
            ensure!(
                pair[0].tag_name().name() == "key",
                "plist dictionary entry has no key"
            );
            let key: String = pair[0]
                .children()
                .filter(|child| child.is_text())
                .filter_map(|child| child.text())
                .collect();
            ensure!(keys.insert(key), "duplicate plist dictionary key");
        }
    }
    Ok(Plist::from_reader_xml(content.as_bytes())?)
}

fn value(node: &Plist) -> Result<Value> {
    Ok(match node {
        Plist::Array(a) => Value::Array(a.iter().map(value).collect::<Result<_>>()?),
        Plist::Dictionary(d) => Value::Object(
            d.iter()
                .map(|(k, v)| Ok((k.clone(), value(v)?)))
                .collect::<Result<_>>()?,
        ),
        Plist::String(v) => json!(v),
        Plist::Boolean(v) => json!(v),
        Plist::Integer(v) => {
            if let Some(n) = v.as_signed() {
                json!(n)
            } else {
                json!(v.as_unsigned().context("invalid plist integer")?)
            }
        }
        Plist::Real(v) => {
            ensure!(v.is_finite(), "non-finite plist numbers are unsupported");
            json!(v)
        }
        Plist::Data(v) => {
            json!({"$plist_data":base64::engine::general_purpose::STANDARD.encode(v)})
        }
        Plist::Date(v) => json!({"$plist_date":v.to_xml_format()}),
        Plist::Uid(_) => {
            bail!("UID values require binary plists, which are not supported by this text adapter")
        }
        _ => bail!("unsupported plist value type"),
    })
}
fn from_value(node: &Value) -> Result<Plist> {
    Ok(match node {
        Value::Null => bail!("plist has no null value; use remove"),
        Value::Bool(v) => Plist::Boolean(*v),
        Value::String(v) => Plist::String(v.clone()),
        Value::Number(v) => {
            if let Some(n) = v.as_i64() {
                Plist::Integer(n.into())
            } else if let Some(n) = v.as_u64() {
                Plist::Integer(n.into())
            } else {
                Plist::Real(v.as_f64().context("invalid number")?)
            }
        }
        Value::Array(a) => Plist::Array(a.iter().map(from_value).collect::<Result<_>>()?),
        Value::Object(map) => {
            if map.len() == 1 && map.contains_key("$plist_data") {
                Plist::Data(
                    base64::engine::general_purpose::STANDARD.decode(
                        map["$plist_data"]
                            .as_str()
                            .context("plist data must be base64")?,
                    )?,
                )
            } else if map.len() == 1 && map.contains_key("$plist_date") {
                Plist::Date(plist::Date::from_xml_format(
                    map["$plist_date"]
                        .as_str()
                        .context("plist date must be a string")?,
                )?)
            } else {
                Plist::Dictionary(
                    map.iter()
                        .map(|(k, v)| Ok((k.clone(), from_value(v)?)))
                        .collect::<Result<_>>()?,
                )
            }
        }
    })
}
fn get<'a>(node: &'a Plist, path: &[String]) -> Result<Option<&'a Plist>> {
    let Some((key, rest)) = path.split_first() else {
        return Ok(Some(node));
    };
    let child = match node {
        Plist::Dictionary(d) => d.get(key),
        Plist::Array(a) => a.get(syncer_document::index(key)?),
        _ => bail!("patch refuses to traverse a scalar plist parent"),
    };
    match child {
        Some(child) => get(child, rest),
        None => Ok(None),
    }
}
fn set(node: &mut Plist, path: &[String], next: Option<&Value>) -> Result<()> {
    let Some((key, rest)) = path.split_first() else {
        *node = from_value(next.context("cannot remove plist root")?)?;
        return Ok(());
    };
    match node {
        Plist::Dictionary(d) => {
            if rest.is_empty() && next.is_none() {
                d.remove(key);
            } else if let Some(child) = d.get_mut(key) {
                set(child, rest, next)?;
            } else if next.is_some() {
                let mut child = Plist::Dictionary(Default::default());
                set(&mut child, rest, next)?;
                d.insert(key.clone(), child);
            }
        }
        Plist::Array(a) => {
            let index = syncer_document::index(key)?;
            ensure!(index < a.len(), "plist array index is out of bounds");
            if rest.is_empty() && next.is_none() {
                a.remove(index);
            } else {
                set(&mut a[index], rest, next)?;
            }
        }
        _ => bail!("patch refuses to traverse a scalar plist parent"),
    }
    Ok(())
}
pub fn document(content: &str, rule: &Rule, action: &str) -> Result<(String, bool)> {
    let mut node = if content.trim().is_empty() {
        Plist::Dictionary(Default::default())
    } else {
        parse(content)?
    };
    let path = syncer_document::validate(rule)?;
    let current = get(&node, &path)?.map(value).transpose()?;
    let edit = syncer_document::edit(current.as_ref(), rule, action)?;
    let Some(next) = &edit.replacement else {
        return Ok((content.into(), edit.compliant));
    };
    set(&mut node, &path, next.as_ref())?;
    let mut output = Vec::new();
    node.to_writer_xml(&mut output)?;
    let output = String::from_utf8(output)?;
    let checked = parse(&output)?;
    ensure!(
        checked == node && get(&checked, &path)?.map(value).transpose()? == *next,
        "plist edit did not round-trip or is not idempotent"
    );
    Ok((output, edit.compliant))
}
