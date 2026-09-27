use anyhow::{Context, Result, bail, ensure};
use roxmltree::{Document, Node};
use serde_json::Value;
use std::ops::Range;
use syncer_language::Rule;

struct Slot {
    value: Option<String>,
    set_range: Range<usize>,
    remove_range: Option<Range<usize>>,
    prefix: String,
    suffix: String,
}
fn expanded(namespace: Option<&str>, name: &str) -> String {
    namespace.map_or_else(|| name.into(), |ns| format!("{{{ns}}}{name}"))
}
fn parse(content: &str) -> Result<Document<'_>> {
    Ok(Document::parse_with_options(
        content,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 500_000,
            ..Default::default()
        },
    )?)
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
        .replace('\r', "&#13;")
        .replace('\n', "&#10;")
        .replace('\t', "&#9;")
}
fn opening_end(content: &str, node: Node<'_, '_>) -> Result<usize> {
    let start = node.range().start;
    let mut quote = None;
    for (offset, c) in content[start..node.range().end].char_indices() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
        } else if c == '\'' || c == '"' {
            quote = Some(c);
        } else if c == '>' {
            return Ok(start + offset + 1);
        }
    }
    bail!("missing XML start tag")
}
fn qname(content: &str, node: Node<'_, '_>) -> String {
    content[node.range().start + 1..]
        .chars()
        .take_while(|c| !c.is_whitespace() && !matches!(c, '/' | '>'))
        .collect()
}
fn new_name(name: &str, node: Node<'_, '_>, attribute: bool) -> Result<String> {
    let (namespace, local) = if let Some(rest) = name.strip_prefix('{') {
        let (namespace, local) = rest.split_once('}').context("invalid expanded XML name")?;
        (Some(namespace), local)
    } else {
        (None, name)
    };
    ensure!(
        !local.is_empty()
            && local
                .chars()
                .next()
                .is_some_and(|c| c == '_' || c.is_alphabetic())
            && local
                .chars()
                .all(|c| c == '_' || c == '-' || c == '.' || c.is_alphanumeric()),
        "invalid XML local name"
    );
    if let Some(namespace) = namespace {
        let prefix = node
            .lookup_prefix(namespace)
            .context("namespace must already be declared before adding a qualified XML name")?;
        ensure!(
            !attribute || !prefix.is_empty(),
            "qualified attribute needs a declared namespace prefix"
        );
        Ok(if prefix.is_empty() {
            local.into()
        } else {
            format!("{prefix}:{local}")
        })
    } else {
        Ok(local.into())
    }
}
fn text_slot(content: &str, node: Node<'_, '_>, removable: bool) -> Result<Slot> {
    ensure!(
        !node.children().any(|child| child.is_element()),
        "XML text pointer must select a leaf element; mixed-content replacement is refused"
    );
    let value = node
        .children()
        .filter(|c| c.is_text())
        .filter_map(|c| c.text())
        .collect::<String>();
    let opening = opening_end(content, node)?;
    let (set_range, prefix, suffix) = if content[..opening].ends_with("/>") {
        (
            opening - 2..opening,
            ">".into(),
            format!("</{}>", qname(content, node)),
        )
    } else {
        let closing = content[..node.range().end]
            .rfind("</")
            .context("missing XML closing tag")?;
        (opening..closing, String::new(), String::new())
    };
    Ok(Slot {
        value: Some(value),
        set_range,
        remove_range: removable.then(|| node.range()),
        prefix,
        suffix,
    })
}
fn select(content: &str, document: &Document<'_>, path: &[String]) -> Result<Slot> {
    ensure!(
        !path.is_empty(),
        "XML pointer must start with the root element name"
    );
    let mut node = document.root_element();
    ensure!(
        path[0] == expanded(node.tag_name().namespace(), node.tag_name().name()),
        "XML pointer root does not match document"
    );
    let mut offset = 1;
    while offset < path.len() {
        let key = &path[offset];
        if let Some(name) = key.strip_prefix('@') {
            ensure!(
                offset + 1 == path.len(),
                "XML attribute must end the pointer"
            );
            if let Some(attribute) = node
                .attributes()
                .find(|a| expanded(a.namespace(), a.name()) == name)
            {
                return Ok(Slot {
                    value: Some(attribute.value().into()),
                    set_range: attribute.range_value(),
                    remove_range: Some(attribute.range()),
                    prefix: String::new(),
                    suffix: String::new(),
                });
            }
            let end = opening_end(content, node)?;
            let position = if content[..end].ends_with("/>") {
                end - 2
            } else {
                end - 1
            };
            return Ok(Slot {
                value: None,
                set_range: position..position,
                remove_range: None,
                prefix: format!(" {}=\"", new_name(name, node, true)?),
                suffix: "\"".into(),
            });
        }
        if key == "#text" {
            ensure!(offset + 1 == path.len(), "#text must end XML pointer");
            return text_slot(content, node, false);
        }
        ensure!(
            offset + 1 < path.len(),
            "XML child selector requires an index, for example /config/server/0/@port"
        );
        let index = syncer_document::index(&path[offset + 1])?;
        let children: Vec<_> = node
            .children()
            .filter(|c| {
                c.is_element() && expanded(c.tag_name().namespace(), c.tag_name().name()) == *key
            })
            .collect();
        if let Some(child) = children.get(index) {
            node = *child;
            offset += 2;
        } else {
            ensure!(
                offset + 2 == path.len() && index == children.len(),
                "XML parent or child index is missing"
            );
            let name = new_name(key, node, false)?;
            let declaration = if !key.starts_with('{') && node.default_namespace().is_some() {
                " xmlns=\"\""
            } else {
                ""
            };
            let end = opening_end(content, node)?;
            let (range, prefix, suffix) = if content[..end].ends_with("/>") {
                (
                    end - 2..end,
                    format!("><{name}{declaration}>"),
                    format!("</{name}></{}>", qname(content, node)),
                )
            } else {
                let position = content[..node.range().end]
                    .rfind("</")
                    .context("missing XML close tag")?;
                (
                    position..position,
                    format!("<{name}{declaration}>"),
                    format!("</{name}>"),
                )
            };
            return Ok(Slot {
                value: None,
                set_range: range,
                remove_range: None,
                prefix,
                suffix,
            });
        }
    }
    text_slot(content, node, path.len() > 1)
}
pub fn document(content: &str, rule: &Rule, action: &str) -> Result<(String, bool)> {
    if rule.kind == "plist" {
        return crate::plist_format::document(content, rule, action);
    }
    let document = parse(content)?;
    let path = syncer_document::validate(rule)?;
    let slot = select(content, &document, &path)?;
    let current = slot.value.map(Value::String);
    let edit = syncer_document::edit(current.as_ref(), rule, action)?;
    let Some(next) = &edit.replacement else {
        return Ok((content.into(), edit.compliant));
    };
    let mut output = content.to_string();
    if let Some(value) = next {
        let value = value
            .as_str()
            .context("XML leaf text and attribute values must be strings")?;
        output.replace_range(
            slot.set_range,
            &format!("{}{}{}", slot.prefix, escape(value), slot.suffix),
        );
    } else {
        output.replace_range(
            slot.remove_range
                .context("cannot remove XML root or #text; set text to an empty string")?,
            "",
        );
    }
    let checked = parse(&output)?;
    let actual = select(&output, &checked, &path)?.value.map(Value::String);
    ensure!(
        actual == *next,
        "XML edit is not idempotent at this pointer (removing repeated elements can shift indices)"
    );
    Ok((output, edit.compliant))
}
