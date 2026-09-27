use anyhow::{Context, Result, bail, ensure};
use hcl::edit::{
    Decorate, Ident,
    expr::{Expression, ObjectKey},
    structure::{Attribute, Body},
};
use serde_json::{Value, json};
use syncer_language::Rule;

fn key_name(key: &ObjectKey) -> Result<String> {
    match key {
        ObjectKey::Ident(v) => Ok(v.to_string()),
        ObjectKey::Expression(expr) => Ok(expr
            .as_str()
            .context("computed HCL object keys cannot be patched")?
            .into()),
    }
}
fn literal(expr: &Expression) -> Result<Value> {
    Ok(match expr {
        Expression::Null(_) => Value::Null,
        Expression::Bool(v) => json!(v.value()),
        Expression::Number(v) => serde_json::from_str(&v.value().to_string())?,
        Expression::String(v) => json!(v.value()),
        Expression::Array(values) => {
            Value::Array(values.iter().map(literal).collect::<Result<_>>()?)
        }
        Expression::Object(values) => {
            let mut map = serde_json::Map::new();
            for (key, value) in values.iter() {
                ensure!(
                    map.insert(key_name(key)?, literal(value.expr())?).is_none(),
                    "duplicate HCL object key"
                );
            }
            Value::Object(map)
        }
        _ => bail!(
            "selected HCL value is an expression; only literal data can be patched (expressions elsewhere are preserved)"
        ),
    })
}
fn expression(value: &Value) -> Result<Expression> {
    // HCL's serializer escapes template markers in literal strings.
    Ok(hcl::to_expression(value)?.to_string().parse()?)
}
fn expr_get(expr: &Expression, path: &[String]) -> Result<Option<Value>> {
    let Some((key, rest)) = path.split_first() else {
        return Ok(Some(literal(expr)?));
    };
    match expr {
        Expression::Object(map) => {
            let mut found = None;
            for (candidate, value) in map.iter() {
                if key_name(candidate)? == *key {
                    ensure!(found.is_none(), "ambiguous HCL key");
                    found = Some(value.expr());
                }
            }
            match found {
                Some(child) => expr_get(child, rest),
                None => Ok(None),
            }
        }
        Expression::Array(array) => match array.get(syncer_document::index(key)?) {
            Some(child) => expr_get(child, rest),
            None => Ok(None),
        },
        _ => bail!("patch refuses to traverse an HCL expression or scalar parent"),
    }
}
fn expr_set(expr: &mut Expression, path: &[String], next: Option<&Value>) -> Result<()> {
    let Some((key, rest)) = path.split_first() else {
        let mut replacement = expression(next.context("cannot remove expression without parent")?)?;
        *replacement.decor_mut() = expr.decor().clone();
        *expr = replacement;
        return Ok(());
    };
    match expr {
        Expression::Object(map) => {
            let existing = map.iter().find_map(|(candidate, _)| {
                (key_name(candidate).ok().as_ref() == Some(key)).then(|| candidate.clone())
            });
            if let Some(existing) = existing {
                if rest.is_empty() && next.is_none() {
                    map.remove(&existing);
                } else {
                    expr_set(map.get_mut(&existing).unwrap().expr_mut(), rest, next)?;
                }
            } else if next.is_some() {
                let mut child = expression(&json!({}))?;
                expr_set(&mut child, rest, next)?;
                let key = ObjectKey::Expression(Expression::from(key.clone()));
                map.insert(key, child);
            }
        }
        Expression::Array(array) => {
            let index = syncer_document::index(key)?;
            ensure!(index < array.len(), "HCL array index is out of bounds");
            if rest.is_empty() && next.is_none() {
                array.remove(index);
            } else {
                expr_set(array.get_mut(index).unwrap(), rest, next)?;
            }
        }
        _ => bail!("patch refuses to traverse an HCL expression or scalar parent"),
    }
    Ok(())
}
fn block_match(body: &Body, path: &[String]) -> Result<Option<(usize, usize)>> {
    let mut found = None;
    for (index, structure) in body.iter().enumerate() {
        if let Some(block) = structure.as_block() {
            let used = 1 + block.labels.len();
            if path.len() >= used
                && block.has_ident(&path[0])
                && block
                    .labels
                    .iter()
                    .zip(&path[1..])
                    .all(|(label, token)| label.as_str() == token)
            {
                ensure!(
                    found.is_none(),
                    "HCL pointer matches multiple blocks; repeated identical block labels are ambiguous"
                );
                found = Some((index, used));
            }
        }
    }
    ensure!(
        found.is_none() || !body.has_attribute(&path[0]),
        "HCL pointer is ambiguous between an attribute and a block"
    );
    Ok(found)
}
fn get(body: &Body, path: &[String]) -> Result<Option<Value>> {
    ensure!(
        !path.is_empty(),
        "HCL pointer must select an attribute, optionally through block type and labels"
    );
    if let Some((index, used)) = block_match(body, path)? {
        return get(
            &body.get(index).unwrap().as_block().unwrap().body,
            &path[used..],
        );
    }
    match body.get_attribute(&path[0]) {
        Some(attr) => expr_get(&attr.value, &path[1..]),
        None => Ok(None),
    }
}
fn set(body: &mut Body, path: &[String], next: Option<&Value>) -> Result<()> {
    ensure!(
        !path.is_empty(),
        "select an HCL attribute, not a block body"
    );
    if let Some((index, used)) = block_match(body, path)? {
        return set(
            &mut body.get_mut(index).unwrap().as_block_mut().unwrap().body,
            &path[used..],
            next,
        );
    }
    if path.len() == 1 && next.is_none() {
        body.remove_attribute(&path[0]);
    } else if let Some(mut attr) = body.get_attribute_mut(&path[0]) {
        expr_set(attr.value_mut(), &path[1..], next)?;
    } else if let Some(next) = next {
        ensure!(
            path.len() == 1,
            "HCL parent is missing; create the literal attribute or block explicitly first"
        );
        body.push(Attribute::new(path[0].parse::<Ident>()?, expression(next)?));
    }
    Ok(())
}
pub fn document(content: &str, rule: &Rule, action: &str) -> Result<(String, bool)> {
    let mut body: Body = content.parse()?;
    let path = syncer_document::validate(rule)?;
    let current = get(&body, &path)?;
    let edit = syncer_document::edit(current.as_ref(), rule, action)?;
    if let Some(next) = &edit.replacement {
        set(&mut body, &path, next.as_ref())?;
        let output = body.to_string();
        let checked: Body = output.parse()?;
        ensure!(
            get(&checked, &path)? == *next,
            "HCL edit did not round-trip"
        );
        Ok((output, edit.compliant))
    } else {
        Ok((content.into(), edit.compliant))
    }
}
