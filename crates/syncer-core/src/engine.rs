use crate::{
    Extensions,
    policy::{EffectiveRule, Resolved},
    storage,
};
use anyhow::{Context as _, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use syncer_language::Rule;

pub struct Context {
    pub home: PathBuf,
    pub project: PathBuf,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleStatus {
    pub id: String,
    pub policy: String,
    pub revision: String,
    pub before: bool,
    pub after: bool,
}
#[derive(Debug)]
pub struct FileChange {
    pub path: PathBuf,
    pub before: Option<Vec<u8>>,
    pub after: Vec<u8>,
    pub sensitive: bool,
    pub permissions: Option<fs::Permissions>,
}
#[derive(Debug, Default)]
pub struct Plan {
    pub files: Vec<FileChange>,
    pub status: Vec<RuleStatus>,
}
impl Plan {
    pub fn changed(&self) -> usize {
        self.files
            .iter()
            .filter(|f| f.before.as_ref() != Some(&f.after))
            .count()
    }
    pub fn compliant(&self) -> bool {
        self.status.iter().all(|s| s.after)
    }
    pub fn summary(&self, show_diff: bool) -> Value {
        json!({"changed_files":self.changed(),"compliant":self.compliant(),"rules":self.status,"files":self.files.iter().filter(|f|f.before.as_ref()!=Some(&f.after)).map(|f|{
            let diff=if show_diff && !f.sensitive && std::str::from_utf8(&f.after).is_ok() && std::str::from_utf8(f.before.as_deref().unwrap_or_default()).is_ok() {Some(similar::TextDiff::from_lines(std::str::from_utf8(f.before.as_deref().unwrap_or_default()).unwrap_or_default(),std::str::from_utf8(&f.after).unwrap_or_default()).unified_diff().header("before","after").to_string())} else {None};
            json!({"path":f.path,"before_sha256":f.before.as_ref().map(|b|crate::digest(b)),"after_sha256":crate::digest(&f.after),"diff":diff,"sensitive":f.sensitive,"binary":std::str::from_utf8(&f.after).is_err() || std::str::from_utf8(f.before.as_deref().unwrap_or_default()).is_err()})
        }).collect::<Vec<_>>()})
    }
    /// Per-file atomic replacement; a journal and backups enable recovery across process crashes.
    pub fn apply(&self, state: &Path) -> Result<Option<PathBuf>> {
        ensure!(
            self.compliant(),
            "plan has unresolved violations; no files were changed"
        );
        let changes: Vec<_> = self
            .files
            .iter()
            .filter(|f| f.before.as_ref() != Some(&f.after))
            .collect();
        if changes.is_empty() {
            return Ok(None);
        }
        for f in &self.files {
            ensure!(
                !storage::is_within(&f.path, state),
                "target is inside syncer state directory"
            );
            storage::verify_unchanged(&f.path, &f.before)?;
        }
        let transaction = state.join("backups").join(uuid::Uuid::new_v4().to_string());
        storage::private_dir(&transaction)?;
        let entries:Vec<_>=changes.iter().enumerate().map(|(i,f)|json!({"path":f.path,"backup":if f.before.is_some(){Some(format!("{i}.bak"))}else{None},"before_sha256":f.before.as_ref().map(|b|crate::digest(b)),"after_sha256":crate::digest(&f.after)})).collect();
        for (i, f) in changes.iter().enumerate() {
            if let Some(before) = &f.before {
                storage::atomic_write(&transaction.join(format!("{i}.bak")), before, None)?;
            }
        }
        let journal = transaction.join("journal.json");
        storage::atomic_write(
            &journal,
            &serde_json::to_vec_pretty(&json!({"state":"prepared","files":entries}))?,
            None,
        )?;
        let mut written: Vec<&FileChange> = vec![];
        for f in changes {
            let result = (|| -> Result<()> {
                storage::verify_unchanged(&f.path, &f.before)?;
                storage::atomic_write(&f.path, &f.after, f.permissions.clone())
            })();
            if let Err(error) = result {
                let mut failures = vec![];
                for previous in written.iter().rev() {
                    let rollback = (|| -> Result<()> {
                        storage::verify_unchanged(&previous.path, &Some(previous.after.clone()))?;
                        if let Some(before) = &previous.before {
                            storage::atomic_write(
                                &previous.path,
                                before,
                                previous.permissions.clone(),
                            )?;
                        } else {
                            fs::remove_file(&previous.path)?;
                        }
                        Ok(())
                    })();
                    if let Err(e) = rollback {
                        failures.push(e.to_string());
                    }
                }
                bail!(
                    "apply failed: {error:#}; backups: {}; rollback errors: {:?}",
                    transaction.display(),
                    failures
                );
            }
            written.push(f);
        }
        storage::atomic_write(
            &journal,
            &serde_json::to_vec_pretty(&json!({"state":"applied","files":entries}))?,
            None,
        )?;
        Ok(Some(transaction))
    }
}
pub async fn target(
    rule: &EffectiveRule,
    context: &Context,
    extensions: &Extensions,
) -> Result<PathBuf> {
    let raw = &rule.rule.target;
    let path = if let Some((scheme, _)) = raw.split_once("://") {
        let result = extensions
            .capability(
                "target",
                scheme,
                "resolve",
                json!({"target":raw,"home":context.home,"project":context.project}),
            )
            .await?;
        PathBuf::from(
            result["path"]
                .as_str()
                .context("target extension returned no path")?,
        )
    } else if let Some(rest) = raw.strip_prefix("~/") {
        context.home.join(rest)
    } else {
        let p = Path::new(raw);
        if p.is_absolute() {
            p.into()
        } else {
            context.project.join(p)
        }
    };
    storage::checked_path(&path, &rule.roots)
}
pub async fn plan(resolved: &Resolved, context: &Context, extensions: &Extensions) -> Result<Plan> {
    plan_with_assets(resolved, context, extensions, &BTreeMap::new()).await
}
pub async fn plan_with_assets(
    resolved: &Resolved,
    context: &Context,
    extensions: &Extensions,
    assets: &BTreeMap<String, Vec<u8>>,
) -> Result<Plan> {
    let mut plan = Plan::default();
    let mut indices = BTreeMap::new();
    let mut targets = vec![];
    for effective in &resolved.rules {
        let path = target(effective, context, extensions).await?;
        for existing in &plan.files {
            ensure!(
                existing.path == path || !storage::same_path(&existing.path, &path),
                "two target spellings refer to the same file; use one consistent path: {} and {}",
                existing.path.display(),
                path.display()
            );
        }
        let index = if let Some(i) = indices.get(&path) {
            *i
        } else {
            let before = storage::read_optional(&path)?;
            let index = plan.files.len();
            plan.files.push(FileChange {
                permissions: fs::metadata(&path).ok().map(|m| m.permissions()),
                path: path.clone(),
                after: before.clone().unwrap_or_default(),
                before,
                sensitive: false,
            });
            indices.insert(path, index);
            index
        };
        targets.push(index);
        let file = &mut plan.files[index];
        file.sensitive |= effective.rule.sensitive;
        let original = file.before.as_deref().unwrap_or_default();
        let (_, before) = document_bytes(&effective.rule, original, "check", extensions, assets)
            .await
            .with_context(|| format!("rule {} ({})", effective.id, effective.origin))?;
        let content = &file.after;
        let (after, _) = document_bytes(&effective.rule, content, "apply", extensions, assets)
            .await
            .with_context(|| format!("rule {} ({})", effective.id, effective.origin))?;
        ensure!(
            after.len() <= syncer_extension_sdk::MAX_BYTES,
            "transformed file too large"
        );
        file.after = after;
        plan.status.push(RuleStatus {
            id: effective.id.clone(),
            policy: effective.origin.clone(),
            revision: effective.revision.clone(),
            before: before && (effective.rule.kind != "file" || file.before.is_some()),
            after: false,
        });
    }
    for (i, effective) in resolved.rules.iter().enumerate() {
        let content = &plan.files[targets[i]].after;
        let (again, compliant) =
            document_bytes(&effective.rule, content, "check", extensions, assets).await?;
        let _ = again;
        plan.status[i].after = compliant;
    }
    for guard in &resolved.guards {
        let path = target(&guard.effective, context, extensions).await?;
        let content = if let Some(index) = indices.get(&path) {
            plan.files[*index].after.clone()
        } else {
            storage::read_optional(&path)?.unwrap_or_default()
        };
        let (_, ok) = document_bytes(
            &guard.effective.rule,
            &content,
            if guard.constraints_only {
                "constraints"
            } else {
                "check"
            },
            extensions,
            assets,
        )
        .await?;
        if !plan
            .status
            .iter()
            .any(|s| s.policy == guard.effective.origin && s.id == guard.effective.id)
        {
            let original = indices
                .get(&path)
                .and_then(|i| plan.files[*i].before.as_deref())
                .unwrap_or_default();
            let action = if guard.constraints_only {
                "constraints"
            } else {
                "check"
            };
            let (_, before) =
                document_bytes(&guard.effective.rule, original, action, extensions, assets).await?;
            plan.status.push(RuleStatus {
                id: guard.effective.id.clone(),
                policy: guard.effective.origin.clone(),
                revision: guard.effective.revision.clone(),
                before,
                after: ok,
            });
        }
        ensure!(
            ok,
            "final content violates inherited {} rule {} from {}; no files changed",
            if guard.constraints_only {
                "constraints for"
            } else {
                "locked"
            },
            guard.effective.id,
            guard.effective.origin
        );
    }
    Ok(plan)
}
async fn document_bytes(
    rule: &Rule,
    content: &[u8],
    action: &str,
    extensions: &Extensions,
    assets: &BTreeMap<String, Vec<u8>>,
) -> Result<(Vec<u8>, bool)> {
    if rule.kind == "file" {
        let desired = if let Some(name) = &rule.content_from {
            assets
                .get(name)
                .with_context(|| {
                    format!("asset {name} is not enrolled; use syncer add {name} URI --asset")
                })?
                .as_slice()
        } else {
            rule.value
                .as_ref()
                .and_then(Value::as_str)
                .context("missing file value")?
                .as_bytes()
        };
        let check_constraints = |bytes: &[u8]| -> Result<()> {
            if !rule.constraints.is_empty() {
                rule.constraints.check(&Value::String(
                    std::str::from_utf8(bytes)
                        .context("string constraints cannot be applied to binary files")?
                        .into(),
                ))?;
            }
            Ok(())
        };
        let compliant =
            (action == "constraints" || content == desired) && check_constraints(content).is_ok();
        if action == "apply" {
            check_constraints(desired)?;
            Ok((desired.to_vec(), compliant))
        } else {
            Ok((content.to_vec(), compliant))
        }
    } else {
        let content =
            std::str::from_utf8(content).context("text and document patches require UTF-8")?;
        let (after, compliant) = document(rule, content, action, extensions).await?;
        Ok((after.into_bytes(), compliant))
    }
}
async fn document(
    rule: &Rule,
    content: &str,
    action: &str,
    extensions: &Extensions,
) -> Result<(String, bool)> {
    if !matches!(rule.kind.as_str(), "file" | "text" | "regex") {
        let out = extensions
            .capability(
                "kind",
                &rule.kind,
                "document",
                json!({"content":content,"rule":rule,"action":action}),
            )
            .await?;
        return Ok((
            out["content"]
                .as_str()
                .context("document extension omitted content")?
                .into(),
            out["compliant"]
                .as_bool()
                .context("document extension omitted compliant")?,
        ));
    }
    if action == "constraints" {
        return Ok((
            content.into(),
            rule.constraints
                .check(&Value::String(content.into()))
                .is_ok(),
        ));
    }
    let after = match (rule.kind.as_str(), rule.operation.as_str()) {
        ("file", "replace") => rule
            .value
            .as_ref()
            .and_then(Value::as_str)
            .context("missing file value")?
            .to_string(),
        ("text", "match") => {
            return Ok((
                content.into(),
                content.contains(rule.pattern.as_ref().context("missing pattern")?)
                    && rule
                        .constraints
                        .check(&Value::String(content.into()))
                        .is_ok(),
            ));
        }
        ("regex", "match") => {
            return Ok((
                content.into(),
                regex::Regex::new(rule.pattern.as_ref().context("missing pattern")?)?
                    .is_match(content)
                    && rule
                        .constraints
                        .check(&Value::String(content.into()))
                        .is_ok(),
            ));
        }
        ("text", "replace") => content.replace(
            rule.pattern.as_ref().context("missing pattern")?,
            rule.value
                .as_ref()
                .and_then(Value::as_str)
                .context("missing replacement")?,
        ),
        ("regex", "replace") => {
            regex::Regex::new(rule.pattern.as_ref().context("missing pattern")?)?
                .replace_all(
                    content,
                    rule.value
                        .as_ref()
                        .and_then(Value::as_str)
                        .context("missing replacement")?,
                )
                .into_owned()
        }
        _ => bail!("unsupported built-in operation"),
    };
    let compliant = after == content
        && rule
            .constraints
            .check(&Value::String(content.into()))
            .is_ok();
    if action != "apply" {
        return Ok((content.into(), compliant));
    }
    // A second application must be a no-op; otherwise unattended runs would keep corrupting the file.
    let twice = match rule.kind.as_str() {
        "text" => after.replace(
            rule.pattern.as_ref().unwrap(),
            rule.value.as_ref().unwrap().as_str().unwrap(),
        ),
        "regex" => regex::Regex::new(rule.pattern.as_ref().unwrap())?
            .replace_all(&after, rule.value.as_ref().unwrap().as_str().unwrap())
            .into_owned(),
        _ => after.clone(),
    };
    ensure!(
        twice == after,
        "replacement is not idempotent; narrow the pattern or use a managed JSON field"
    );
    rule.constraints
        .check(&Value::String(after.clone()))
        .context("result violates constraints")?;
    Ok((after, compliant))
}
