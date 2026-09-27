use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::{collections::BTreeMap, ops::Range};
use syncer_language::Rule;

#[derive(Clone)]
struct Entry {
    range: Range<usize>,
    value: String,
    prefix: String,
    suffix: String,
}
struct Document {
    entries: BTreeMap<Vec<String>, Entry>,
    sections: BTreeMap<String, usize>,
    first_section: usize,
}

fn properties_decode(text: &str) -> Result<String> {
    let mut chars = text.chars().peekable();
    let mut output = String::new();
    while let Some(c) = chars.next() {
        if c != '\\' {
            output.push(c);
            continue;
        }
        let c = chars.next().context("trailing properties escape")?;
        output.push(match c {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'f' => '\u{c}',
            'u' => {
                let digits: String = chars.by_ref().take(4).collect();
                ensure!(digits.len() == 4, "short properties unicode escape");
                let first = u16::from_str_radix(&digits, 16)?;
                if (0xd800..=0xdbff).contains(&first) {
                    ensure!(
                        chars.next() == Some('\\') && chars.next() == Some('u'),
                        "unpaired properties unicode surrogate"
                    );
                    let digits: String = chars.by_ref().take(4).collect();
                    ensure!(digits.len() == 4, "short properties unicode escape");
                    let second = u16::from_str_radix(&digits, 16)?;
                    ensure!(
                        (0xdc00..=0xdfff).contains(&second),
                        "invalid properties unicode surrogate"
                    );
                    char::from_u32(
                        0x10000 + ((u32::from(first) - 0xd800) << 10) + u32::from(second) - 0xdc00,
                    )
                    .context("invalid unicode")?
                } else {
                    char::from_u32(u32::from(first))
                        .context("invalid properties unicode surrogate")?
                }
            }
            other => other,
        });
    }
    Ok(output)
}
fn properties_encode(text: &str, key: bool) -> String {
    let mut output = String::new();
    for (index, c) in text.chars().enumerate() {
        match c {
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{c}' => output.push_str("\\f"),
            ' ' | '=' | ':' | '#' | '!' if key || index == 0 => {
                output.push('\\');
                output.push(c);
            }
            c if !c.is_ascii() || c.is_control() => {
                for unit in c.encode_utf16(&mut [0; 2]) {
                    output.push_str(&format!("\\u{unit:04x}"));
                }
            }
            c => output.push(c),
        }
    }
    output
}
fn env_decode(raw: &str) -> Result<(String, String)> {
    if let Some(quote) = raw.chars().next().filter(|c| *c == '\'' || *c == '"') {
        let mut output = String::new();
        let mut chars = raw[1..].char_indices();
        while let Some((index, c)) = chars.next() {
            if c == quote {
                let suffix = &raw[index + 2..];
                ensure!(
                    suffix.trim().is_empty() || suffix.trim_start().starts_with('#'),
                    "unexpected text after dotenv quote"
                );
                return Ok((output, suffix.into()));
            }
            if c == '\\' && quote == '"' {
                let escaped = match chars.next().context("trailing dotenv escape")?.1 {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    '"' => '"',
                    '\\' => '\\',
                    '$' => '$',
                    '`' => '`',
                    other => {
                        output.push('\\');
                        other
                    }
                };
                output.push(escaped);
            } else {
                output.push(c);
            }
        }
        bail!("unterminated dotenv quote; multiline quoted values are not supported");
    }
    let comment = raw
        .char_indices()
        .find(|(i, c)| *c == '#' && (*i == 0 || raw[..*i].ends_with(char::is_whitespace)))
        .map(|(i, _)| i)
        .unwrap_or(raw.len());
    let value = raw[..comment].trim_end();
    Ok((value.into(), raw[value.len()..].into()))
}
fn env_encode(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '"' | '\\' | '$' | '`' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}
fn env_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}
fn parse(content: &str, kind: &str) -> Result<Document> {
    ensure!(
        !content.contains('\0'),
        "NUL is not supported in configuration files"
    );
    let mut document = Document {
        entries: BTreeMap::new(),
        sections: BTreeMap::new(),
        first_section: content.len(),
    };
    let mut section = String::new();
    let mut offset = 0;
    let mut physical = content.split_inclusive('\n');
    while let Some(line) = physical.next() {
        let start = offset;
        offset += line.len();
        let mut logical = line.trim_end_matches(['\r', '\n']).to_string();
        if kind == "properties" {
            while logical.chars().rev().take_while(|c| *c == '\\').count() % 2 == 1 {
                logical.pop();
                let next = physical
                    .next()
                    .context("unterminated properties continuation")?;
                offset += next.len();
                logical.push_str(next.trim_end_matches(['\r', '\n']).trim_start());
            }
        }
        let text = logical.trim_start_matches('\u{feff}').trim_start();
        if text.is_empty()
            || text.starts_with('#')
            || (kind == "ini" && text.starts_with(';'))
            || (kind == "properties" && text.starts_with('!'))
        {
            continue;
        }
        if kind == "ini" && text.starts_with('[') {
            let close = text.find(']').context("unterminated INI section")?;
            let trailing = text[close + 1..].trim();
            ensure!(
                trailing.is_empty() || trailing.starts_with(['#', ';']),
                "invalid INI section suffix"
            );
            if !section.is_empty() {
                document.sections.insert(section.clone(), start);
            }
            section = text[1..close].trim().into();
            ensure!(
                !section.is_empty() && !document.sections.contains_key(&section),
                "empty or duplicate INI section"
            );
            document.sections.insert(section.clone(), content.len());
            document.first_section = document.first_section.min(start);
            continue;
        }
        let (key, value, prefix, suffix) = if kind == "properties" {
            let mut escaped = false;
            let mut split = text.len();
            for (i, c) in text.char_indices() {
                if !escaped && (c == '=' || c == ':' || c.is_ascii_whitespace()) {
                    split = i;
                    break;
                }
                if c == '\\' {
                    escaped = !escaped;
                } else {
                    escaped = false;
                }
            }
            let mut tail = text[split..].trim_start();
            if tail.starts_with(['=', ':']) {
                tail = tail[1..].trim_start();
            }
            let key = properties_decode(&text[..split])?;
            (
                key.clone(),
                properties_decode(tail)?,
                format!("{}=", properties_encode(&key, true)),
                String::new(),
            )
        } else {
            let assignment = if kind == "dotenv" {
                text.strip_prefix("export ")
                    .map(str::trim_start)
                    .unwrap_or(text)
            } else {
                text
            };
            let separator = assignment
                .find(|c| c == '=' || (kind == "ini" && c == ':'))
                .context("configuration line requires key=value")?;
            let key = assignment[..separator].trim();
            ensure!(!key.is_empty(), "empty configuration key");
            if kind == "dotenv" {
                ensure!(env_key(key), "invalid dotenv variable name");
            }
            let raw = assignment[separator + 1..].trim_start();
            let prefix_len = logical.len() - raw.len();
            let (value, suffix) = if kind == "dotenv" {
                env_decode(raw)?
            } else {
                let pos = raw
                    .char_indices()
                    .find(|(i, c)| {
                        matches!(c, '#' | ';')
                            && (*i == 0 || raw[..*i].ends_with(char::is_whitespace))
                    })
                    .map(|(i, _)| i)
                    .unwrap_or(raw.len());
                let value = raw[..pos].trim_end();
                (value.into(), raw[value.len()..].into())
            };
            (key.into(), value, logical[..prefix_len].into(), suffix)
        };
        let path = if kind == "ini" && !section.is_empty() {
            vec![section.clone(), key]
        } else {
            vec![key]
        };
        let entry = Entry {
            range: start..offset,
            value,
            prefix,
            suffix,
        };
        ensure!(
            document.entries.insert(path, entry).is_none(),
            "duplicate configuration key"
        );
    }
    Ok(document)
}
pub fn document(content: &str, rule: &Rule, action: &str) -> Result<(String, bool)> {
    let path = syncer_document::validate(rule)?;
    ensure!(
        path.len() == 1 || (rule.kind == "ini" && path.len() == 2),
        "pointer must select /key (or /section/key for INI)"
    );
    let document = parse(content, &rule.kind)?;
    let current = document
        .entries
        .get(&path)
        .map(|e| Value::String(e.value.clone()));
    let edit = syncer_document::edit(current.as_ref(), rule, action)?;
    let Some(next) = &edit.replacement else {
        return Ok((content.into(), edit.compliant));
    };
    let value = next
        .as_ref()
        .map(|v| {
            v.as_str()
                .context("INI, dotenv and properties values must be strings")
        })
        .transpose()?;
    let newline = if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let encode = |value: &str| -> Result<String> {
        Ok(match rule.kind.as_str() {
            "dotenv" => {
                ensure!(!value.contains('\0'), "dotenv value contains NUL");
                env_encode(value)
            }
            "properties" => properties_encode(value, false),
            _ => {
                ensure!(
                    !value.contains(['\r', '\n', '\0']) && value == value.trim(),
                    "INI values must be single-line strings without surrounding whitespace"
                );
                value.into()
            }
        })
    };
    let mut output = content.to_string();
    if let Some(entry) = document.entries.get(&path) {
        let replacement = if let Some(value) = value {
            let ending = if content[entry.range.clone()].ends_with('\n') {
                newline
            } else {
                ""
            };
            format!("{}{}{}{ending}", entry.prefix, encode(value)?, entry.suffix)
        } else {
            String::new()
        };
        output.replace_range(entry.range.clone(), &replacement);
    } else if let Some(value) = value {
        let key = path.last().unwrap();
        let key = match rule.kind.as_str() {
            "dotenv" => {
                ensure!(env_key(key), "invalid dotenv variable name");
                key.clone()
            }
            "properties" => properties_encode(key, true),
            _ => {
                ensure!(
                    !key.is_empty()
                        && key == key.trim()
                        && !key.contains(['\r', '\n', '=', ';', ':', '#', '[', ']']),
                    "invalid INI key"
                );
                key.clone()
            }
        };
        let mut insertion = String::new();
        let position = if rule.kind == "ini" && path.len() == 2 {
            let section = &path[0];
            ensure!(
                !section.is_empty()
                    && section == section.trim()
                    && !section.contains(['\r', '\n', '[', ']']),
                "invalid INI section"
            );
            if let Some(position) = document.sections.get(section) {
                *position
            } else {
                insertion.push_str(&format!("[{section}]{newline}"));
                content.len()
            }
        } else if rule.kind == "ini" {
            document.first_section
        } else {
            content.len()
        };
        if position > 0 && !content[..position].ends_with('\n') {
            insertion.insert_str(0, newline);
        }
        insertion.push_str(&format!("{key}={}{newline}", encode(value)?));
        output.insert_str(position, &insertion);
    }
    let checked = parse(&output, &rule.kind)?;
    let mut expected: BTreeMap<_, _> = document
        .entries
        .iter()
        .map(|(k, v)| (k.clone(), v.value.clone()))
        .collect();
    if let Some(value) = value {
        expected.insert(path, value.into());
    } else {
        expected.remove(&path);
    }
    ensure!(
        checked
            .entries
            .into_iter()
            .map(|(k, v)| (k, v.value))
            .collect::<BTreeMap<_, _>>()
            == expected,
        "configuration edit did not round-trip"
    );
    Ok((output, edit.compliant))
}
