use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
use syncer_language::{Override, Policy, Rule};
#[derive(Debug, Clone)]
pub struct Layer {
    pub policy: Policy,
    pub roots: Vec<PathBuf>,
    pub revision: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct EffectiveRule {
    pub id: String,
    pub origin: String,
    pub revision: String,
    pub rule: Rule,
    pub roots: Vec<PathBuf>,
}
#[derive(Debug, Clone)]
pub struct Guard {
    pub effective: EffectiveRule,
    pub constraints_only: bool,
}
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    pub rules: Vec<EffectiveRule>,
    pub guards: Vec<Guard>,
}

pub fn resolve(layers: &[Layer], selected: &[String]) -> Result<Resolved> {
    let mut names = BTreeSet::new();
    let mut roles = BTreeMap::new();
    let mut chosen = BTreeSet::new();
    for (i, layer) in layers.iter().enumerate() {
        ensure!(
            names.insert(&layer.policy.name),
            "duplicate policy name {}",
            layer.policy.name
        );
        for (name, role) in &layer.policy.roles {
            roles.insert(format!("{}/{}", layer.policy.name, name), (i, role));
        }
        for name in &layer.policy.select_roles {
            let full = if name.contains('/') {
                name.clone()
            } else {
                format!("{}/{}", layer.policy.name, name)
            };
            ensure!(
                roles.contains_key(&full),
                "unknown or lower-layer role {full}; select roles from this or an earlier layer"
            );
            chosen.insert(full);
        }
    }
    for name in selected {
        ensure!(
            roles.contains_key(name),
            "unknown selected role {name}; use policy/role"
        );
        chosen.insert(name.clone());
    }
    let mut resolved = Resolved::default();
    let mut indices: BTreeMap<String, usize> = BTreeMap::new();
    for (i, layer) in layers.iter().enumerate() {
        let mut rules = layer.policy.rules.clone();
        for name in &chosen {
            let (owner, role) = roles.get(name).context("unknown role")?;
            if *owner == i {
                for (id, rule) in &role.rules {
                    ensure!(
                        rules.insert(id.clone(), rule.clone()).is_none(),
                        "rule {id} is defined more than once in layer {} (including selected roles)",
                        layer.policy.name
                    );
                }
            }
        }
        for (id, rule) in rules {
            let mut next = EffectiveRule {
                id: id.clone(),
                origin: layer.policy.name.clone(),
                revision: layer.revision.clone(),
                rule,
                roots: layer.roots.clone(),
            };
            if let Some(index) = indices.get(&id) {
                let previous = &resolved.rules[*index];
                match previous.rule.override_mode {
                    Override::Locked => {
                        ensure!(
                            previous.rule == next.rule,
                            "rule {id} is locked by {}; lower layers cannot override it",
                            previous.origin
                        );
                        continue;
                    }
                    Override::Constrained => {
                        ensure!(
                            previous.rule.same_setting(&next.rule),
                            "rule {id} cannot change the protected setting identity"
                        );
                        resolved.guards.push(Guard {
                            effective: previous.clone(),
                            constraints_only: true,
                        });
                        if next.rule.override_mode == Override::Free {
                            next.rule.override_mode = Override::Constrained;
                        }
                    }
                    Override::Free => {}
                }
                // All inherited root approvals also apply when overriding a protected rule.
                if previous.rule.override_mode != Override::Free {
                    next.roots = previous.roots.clone();
                }
                resolved.rules[*index] = next;
            } else {
                indices.insert(id, resolved.rules.len());
                resolved.rules.push(next);
            }
        }
    }
    for effective in &resolved.rules {
        match effective.rule.override_mode {
            Override::Locked => resolved.guards.push(Guard {
                effective: effective.clone(),
                constraints_only: false,
            }),
            Override::Constrained => resolved.guards.push(Guard {
                effective: effective.clone(),
                constraints_only: true,
            }),
            Override::Free => {}
        }
    }
    Ok(resolved)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn layer(name: &str, rule: &str) -> Layer {
        let p =
            syncer_language::parse(&format!("schema_version=1\nname=\"{name}\"\n{rule}")).unwrap();
        Layer {
            policy: p,
            roots: vec![],
            revision: String::new(),
        }
    }
    #[test]
    fn locked_cannot_be_replaced() {
        let a = layer(
            "org",
            r#"rule "a" {
target="x"
kind="file"
operation="replace"
value="a"
override="locked"
}"#,
        );
        let b = layer(
            "user",
            r#"rule "a" {
target="x"
kind="file"
operation="replace"
value="b"
}"#,
        );
        assert!(resolve(&[a, b], &[]).is_err());
    }
    #[test]
    fn lower_selects_upper_and_own_role() {
        let a = layer(
            "org",
            r#"role "dev" {
rule "a" {
target="x"
kind="file"
operation="replace"
value="a"
}
}"#,
        );
        let b = layer(
            "user",
            "select_roles=[\"org/dev\",\"local\"]\nrole \"local\" {} ",
        );
        assert_eq!(resolve(&[a, b], &[]).unwrap().rules.len(), 1);
    }
}
