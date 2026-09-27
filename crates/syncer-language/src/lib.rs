//! Strict, versioned HCL policy language. Policies contain data, never executable expressions.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub schema_version: u32,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub contact: String,
    #[serde(default)]
    pub revision: String,
    #[serde(default)]
    pub select_roles: Vec<String>,
    #[serde(default, rename = "rule")]
    pub rules: BTreeMap<String, Rule>,
    #[serde(default, rename = "role")]
    pub roles: BTreeMap<String, Role>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Role {
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "rule")]
    pub rules: BTreeMap<String, Rule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub target: String,
    /// `file`, `text`, `regex`, or an installed document extension (e.g. `json`).
    pub kind: String,
    /// replace, match, set, ensure, remove, or array.
    pub operation: String,
    /// Name of a locally enrolled asset; never an arbitrary credential-bearing URL.
    #[serde(default)]
    pub content_from: Option<String>,
    #[serde(
        default,
        deserialize_with = "nullable_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub value: Option<Value>,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub pointer: Option<String>,
    #[serde(default)]
    pub constraints: Constraints,
    #[serde(default, rename = "override")]
    pub override_mode: Override,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub added: String,
    #[serde(default)]
    pub sensitive: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Override {
    #[default]
    Free,
    Constrained,
    Locked,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Constraints {
    #[serde(default, rename = "enum")]
    pub allowed: Option<Vec<Value>>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub required_items: Vec<Value>,
    #[serde(default)]
    pub forbidden_items: Vec<Value>,
    #[serde(default)]
    pub allowed_items: Option<Vec<Value>>,
    #[serde(default)]
    pub empty: Option<bool>,
    #[serde(default)]
    pub pattern: Option<String>,
}
impl Constraints {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
    pub fn check(&self, value: &Value) -> Result<()> {
        if let Some(allowed) = &self.allowed {
            ensure!(allowed.contains(value), "value is outside the allowed enum");
        }
        if self.min.is_some() || self.max.is_some() {
            let n = value
                .as_f64()
                .context("numeric constraint requires a number")?;
            if let Some(min) = self.min {
                ensure!(n >= min, "value is below minimum {min}");
            }
            if let Some(max) = self.max {
                ensure!(n <= max, "value exceeds maximum {max}");
            }
        }
        if !self.required_items.is_empty()
            || !self.forbidden_items.is_empty()
            || self.allowed_items.is_some()
        {
            let array = value
                .as_array()
                .context("item constraints require an array")?;
            ensure!(
                self.required_items.iter().all(|x| array.contains(x)),
                "array is missing required items"
            );
            ensure!(
                self.forbidden_items.iter().all(|x| !array.contains(x)),
                "array contains forbidden items"
            );
            if let Some(allowed) = &self.allowed_items {
                ensure!(
                    array.iter().all(|x| allowed.contains(x)),
                    "array contains items outside allowed_items"
                );
            }
        }
        if let Some(empty) = self.empty {
            let actual = match value {
                Value::String(s) => s.is_empty(),
                Value::Array(a) => a.is_empty(),
                Value::Object(o) => o.is_empty(),
                _ => bail!("empty constraint requires a string, array or object"),
            };
            ensure!(actual == empty, "empty constraint is not satisfied");
        }
        if let Some(pattern) = &self.pattern {
            ensure!(
                regex::Regex::new(pattern)?.is_match(
                    value
                        .as_str()
                        .context("pattern constraint requires a string")?
                ),
                "pattern constraint is not satisfied"
            );
        }
        Ok(())
    }
    fn validate(&self) -> Result<()> {
        if let (Some(min), Some(max)) = (self.min, self.max) {
            ensure!(min <= max, "min exceeds max");
        }
        ensure!(
            !self
                .required_items
                .iter()
                .any(|x| self.forbidden_items.contains(x)),
            "an item cannot be required and forbidden"
        );
        if let Some(allowed) = &self.allowed_items {
            ensure!(
                self.required_items.iter().all(|x| allowed.contains(x)),
                "required item is outside allowed_items"
            );
        }
        ensure!(
            self.empty != Some(true) || self.required_items.is_empty(),
            "empty array cannot require items"
        );
        if let Some(pattern) = &self.pattern {
            regex::Regex::new(pattern).context("invalid constraint regex")?;
        }
        if let Some(values) = &self.allowed {
            ensure!(!values.is_empty(), "enum must not be empty");
        }
        Ok(())
    }
}
impl Rule {
    pub fn validate(&self) -> Result<()> {
        ensure!(!self.target.is_empty(), "target must not be empty");
        ensure!(!self.kind.is_empty(), "kind must not be empty");
        self.constraints.validate()?;
        ensure!(
            self.kind == "file" || self.content_from.is_none(),
            "content_from is only supported for file replacement"
        );
        if self.override_mode == Override::Constrained {
            ensure!(
                !self.constraints.is_empty(),
                "constrained override requires constraints"
            );
        }
        if self.kind == "file" {
            ensure!(self.operation == "replace", "file supports only replace");
            ensure!(
                (self.value.as_ref().is_some_and(Value::is_string) && self.content_from.is_none())
                    || (self.value.is_none()
                        && self.content_from.as_ref().is_some_and(|s| valid_name(s))),
                "file replace requires exactly one string value or content_from asset name"
            );
        }
        if self.kind == "text" || self.kind == "regex" {
            ensure!(
                matches!(self.operation.as_str(), "match" | "replace"),
                "text/regex supports match or replace"
            );
            ensure!(
                self.pattern.as_ref().is_some_and(|x| !x.is_empty()),
                "text/regex requires a nonempty pattern"
            );
            if self.kind == "regex" {
                regex::Regex::new(self.pattern.as_ref().unwrap()).context("invalid regex")?;
            }
            if self.operation == "replace" {
                ensure!(
                    self.value.as_ref().is_some_and(Value::is_string),
                    "replace requires a string value"
                );
            }
        }
        if !self.added.is_empty() {
            ensure!(
                valid_date(&self.added),
                "added must be a valid YYYY-MM-DD date"
            );
        }
        Ok(())
    }
    /// Override may change the desired value, never the identity of the protected setting.
    pub fn same_setting(&self, other: &Self) -> bool {
        self.target == other.target
            && self.kind == other.kind
            && self.pointer == other.pointer
            && self.pattern == other.pattern
            && self.operation == other.operation
    }
}
fn nullable_value<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

fn valid_date(s: &str) -> bool {
    let parts: Vec<_> = s.split('-').collect();
    if parts.len() != 3 || parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        return false;
    }
    let (Ok(y), Ok(m), Ok(d)) = (
        parts[0].parse::<u32>(),
        parts[1].parse::<usize>(),
        parts[2].parse::<u32>(),
    ) else {
        return false;
    };
    let days = [
        31,
        if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            29
        } else {
            28
        },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    (1..=12).contains(&m) && d >= 1 && d <= days[m - 1]
}

pub fn parse(input: &str) -> Result<Policy> {
    let body = hcl::parse(input).context("invalid HCL")?;
    validate_body(&body)?;
    use hcl::eval::Evaluate;
    let body = body
        .evaluate(&hcl::eval::Context::new())
        .context("invalid literal value")?;
    let policy: Policy = hcl::de::from_body(body).context("invalid syncer policy schema")?;
    ensure!(
        policy.schema_version == 1,
        "unsupported schema_version {}; expected 1",
        policy.schema_version
    );
    ensure!(
        valid_name(&policy.name),
        "policy name must contain only ASCII letters, digits, '-', '_' or '.'"
    );
    for (id, rule) in &policy.rules {
        ensure!(valid_name(id), "invalid rule ID {id}");
        rule.validate().with_context(|| format!("rule {id}"))?;
    }
    for (name, role) in &policy.roles {
        ensure!(valid_name(name), "invalid role name {name}");
        for (id, rule) in &role.rules {
            ensure!(valid_name(id), "invalid rule ID {id}");
            rule.validate()
                .with_context(|| format!("role {name}, rule {id}"))?;
        }
    }
    Ok(policy)
}
pub fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        && s != "."
        && s != ".."
}

fn validate_body(body: &hcl::Body) -> Result<()> {
    let mut keys = BTreeSet::new();
    for item in body.iter() {
        match item {
            hcl::Structure::Attribute(a) => {
                ensure!(
                    keys.insert(format!("a:{}", a.key)),
                    "duplicate attribute {}",
                    a.key
                );
                literal(&a.expr)?;
            }
            hcl::Structure::Block(b) => {
                ensure!(
                    keys.insert(format!("b:{}:{:?}", b.identifier, b.labels)),
                    "duplicate block {}",
                    b.identifier
                );
                ensure!(
                    !body
                        .attributes()
                        .any(|a| a.key.as_str() == b.identifier.as_str()),
                    "attribute/block name collision {}",
                    b.identifier
                );
                validate_body(&b.body)?;
            }
        }
    }
    Ok(())
}
fn literal(expr: &hcl::Expression) -> Result<()> {
    use hcl::Expression as E;
    match expr {
        E::Null | E::Bool(_) | E::Number(_) | E::String(_) => Ok(()),
        E::TemplateExpr(expr) => {
            let template = hcl::Template::from_expr(expr)?;
            ensure!(
                template
                    .elements()
                    .iter()
                    .all(|e| matches!(e, hcl::template::Element::Literal(_))),
                "interpolation and directives are not executed"
            );
            Ok(())
        }
        E::Array(a) => {
            for v in a {
                literal(v)?;
            }
            Ok(())
        }
        E::Object(o) => {
            for (k, v) in o {
                if let hcl::expr::ObjectKey::Expression(e) = k {
                    literal(e)?;
                }
                literal(v)?;
            }
            Ok(())
        }
        _ => bail!(
            "schema v1 accepts literal values only; expressions and interpolation are not executed"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hcl_rules_and_roles() {
        let p = parse(
            r#"schema_version=1
name="org"
role "dev" {
rule "a" {
target="a"
kind="file"
operation="replace"
value="x"
override="locked"
}
}"#,
        )
        .unwrap();
        assert_eq!(p.roles["dev"].rules["a"].override_mode, Override::Locked);
    }
    #[test]
    fn reject_mistakes() {
        for input in [
            "schema_version=1\nname=\"a\"\nwat=true",
            "schema_version=1\nname=\"a\"\nname=\"b\"",
            "schema_version=1\nname=env(\"HOME\")",
            "schema_version=1\nname=\"${bad}\"",
        ] {
            assert!(parse(input).is_err(), "{input}");
        }
    }
    #[test]
    fn constraints_check_types_and_membership() {
        let c = Constraints {
            required_items: vec![Value::from("good")],
            forbidden_items: vec![Value::from("bad")],
            ..Default::default()
        };
        assert!(c.check(&serde_json::json!(["good", "personal"])).is_ok());
        assert!(c.check(&serde_json::json!(["good", "bad"])).is_err());
        assert!(c.check(&Value::Null).is_err());
    }
}
