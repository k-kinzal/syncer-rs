use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value as Json, json};
use syncer_language::Rule;
use toml_edit::{DocumentMut, Item, Table, Value};

fn value_json(value: &Value) -> Result<Json> {
    Ok(match value {
        Value::String(v) => json!(v.value()),
        Value::Integer(v) => json!(v.value()),
        Value::Float(v) => {
            ensure!(
                v.value().is_finite(),
                "non-finite TOML numbers are unsupported"
            );
            json!(v.value())
        }
        Value::Boolean(v) => json!(v.value()),
        Value::Datetime(v) => json!({"$toml_datetime": v.value().to_string()}),
        Value::Array(v) => Json::Array(v.iter().map(value_json).collect::<Result<_>>()?),
        Value::InlineTable(v) => Json::Object(
            v.iter()
                .map(|(k, v)| Ok((k.into(), value_json(v)?)))
                .collect::<Result<_>>()?,
        ),
    })
}
fn table_json(table: &Table) -> Result<Json> {
    Ok(Json::Object(
        table
            .iter()
            .filter(|(_, v)| !v.is_none())
            .map(|(k, v)| Ok((k.into(), item_json(v)?)))
            .collect::<Result<_>>()?,
    ))
}
fn item_json(item: &Item) -> Result<Json> {
    match item {
        Item::Value(v) => value_json(v),
        Item::Table(t) => table_json(t),
        Item::ArrayOfTables(a) => Ok(Json::Array(
            a.iter().map(table_json).collect::<Result<_>>()?,
        )),
        Item::None => bail!("missing TOML item"),
    }
}
fn to_value(value: &Json) -> Result<Value> {
    Ok(match value {
        Json::Null => bail!("TOML has no null value; use remove"),
        Json::Bool(v) => Value::from(*v),
        Json::String(v) => Value::from(v.as_str()),
        Json::Number(v) => {
            if let Some(n) = v.as_i64() {
                Value::from(n)
            } else {
                ensure!(v.is_f64(), "TOML integer exceeds signed 64-bit range");
                Value::from(v.as_f64().context("invalid TOML number")?)
            }
        }
        Json::Array(v) => Value::Array(
            v.iter()
                .map(to_value)
                .collect::<Result<toml_edit::Array>>()?,
        ),
        Json::Object(v) => {
            if v.len() == 1 && v.contains_key("$toml_datetime") {
                Value::from(
                    v["$toml_datetime"]
                        .as_str()
                        .context("datetime must be a string")?
                        .parse::<toml_edit::Datetime>()?,
                )
            } else {
                let mut table = toml_edit::InlineTable::new();
                for (key, value) in v {
                    table.insert(key, to_value(value)?);
                }
                Value::InlineTable(table)
            }
        }
    })
}
fn to_table(value: &Json) -> Result<Table> {
    let map = value
        .as_object()
        .context("TOML document/table requires an object")?;
    let mut table = Table::new();
    for (key, value) in map {
        table.insert(key, Item::Value(to_value(value)?));
    }
    Ok(table)
}
fn assign_value(node: &mut Value, path: &[String], next: Option<&Json>) -> Result<()> {
    let Some((key, rest)) = path.split_first() else {
        let mut replacement = to_value(next.context("cannot remove value without parent")?)?;
        *replacement.decor_mut() = node.decor().clone();
        *node = replacement;
        return Ok(());
    };
    match node {
        Value::InlineTable(table) => {
            if rest.is_empty() && next.is_none() {
                table.remove(key);
            } else if let Some(child) = table.get_mut(key) {
                assign_value(child, rest, next)?;
            } else if next.is_some() {
                let mut child = Value::InlineTable(toml_edit::InlineTable::new());
                assign_value(&mut child, rest, next)?;
                table.insert(key, child);
            }
        }
        Value::Array(array) => {
            let index = syncer_document::index(key)?;
            ensure!(index < array.len(), "array index is out of bounds");
            if rest.is_empty() && next.is_none() {
                array.remove(index);
            } else {
                assign_value(array.get_mut(index).unwrap(), rest, next)?;
            }
        }
        _ => bail!("patch refuses to traverse a scalar TOML parent"),
    }
    Ok(())
}
fn assign_table(table: &mut Table, path: &[String], next: Option<&Json>) -> Result<()> {
    let Some((key, rest)) = path.split_first() else {
        *table = to_table(next.context("cannot remove document root")?)?;
        return Ok(());
    };
    if rest.is_empty() && next.is_none() {
        table.remove(key);
        return Ok(());
    }
    if !table.contains_key(key) {
        if next.is_none() {
            return Ok(());
        }
        if rest.is_empty() {
            table.insert(key, Item::Value(to_value(next.unwrap())?));
            return Ok(());
        }
        let mut child = Table::new();
        child.set_implicit(true);
        table.insert(key, Item::Table(child));
    }
    assign_item(table.get_mut(key).unwrap(), rest, next)
}
fn assign_item(item: &mut Item, path: &[String], next: Option<&Json>) -> Result<()> {
    match item {
        Item::Value(value) => assign_value(value, path, next),
        Item::Table(table) => assign_table(table, path, next),
        Item::ArrayOfTables(array) => {
            if path.is_empty() {
                *item = Item::Value(to_value(next.context("cannot remove without parent")?)?);
                return Ok(());
            }
            let index = syncer_document::index(&path[0])?;
            ensure!(index < array.len(), "table array index is out of bounds");
            if path.len() == 1 && next.is_none() {
                array.remove(index);
            } else {
                assign_table(array.get_mut(index).unwrap(), &path[1..], next)?;
            }
            Ok(())
        }
        Item::None => bail!("missing TOML parent"),
    }
}
pub fn document(content: &str, rule: &Rule, action: &str) -> Result<(String, bool)> {
    let mut document: DocumentMut = content.parse()?;
    let mut expected = table_json(document.as_table())?;
    let edit = syncer_document::apply(&mut expected, rule, action)?;
    if let Some(next) = &edit.replacement {
        assign_table(
            document.as_table_mut(),
            &syncer_document::validate(rule)?,
            next.as_ref(),
        )?;
        let output = document.to_string();
        let checked: DocumentMut = output.parse()?;
        ensure!(
            table_json(checked.as_table())? == expected,
            "TOML edit did not round-trip"
        );
        Ok((output, edit.compliant))
    } else {
        Ok((content.into(), edit.compliant))
    }
}
