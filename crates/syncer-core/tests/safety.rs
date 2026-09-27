use std::fs;
use syncer_core::{Context, Extensions, Layer, plan, resolve};
fn layer(root: &std::path::Path, name: &str, rules: &str) -> Layer {
    Layer {
        policy: syncer_language::parse(&format!("schema_version=1\nname=\"{name}\"\n{rules}"))
            .unwrap(),
        roots: vec![root.into()],
        revision: "v1".into(),
    }
}
fn context(root: &std::path::Path) -> Context {
    Context {
        project: root.into(),
        home: root.into(),
    }
}
fn replacement(id: &str, target: &str, value: &str, locked: bool) -> String {
    format!(
        "rule {id:?} {{\ntarget={target:?}\nkind=\"file\"\noperation=\"replace\"\nvalue={value:?}\noverride=\"{}\"\n}}",
        if locked { "locked" } else { "free" }
    )
}
#[tokio::test]
async fn dry_plan_and_apply_backup_then_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let file = root.join("settings");
    fs::write(&file, "old").unwrap();
    let rules = resolve(
        &[layer(
            &root,
            "org",
            &replacement("a", "settings", "new", false),
        )],
        &[],
    )
    .unwrap();
    let extensions = Extensions::default();
    let p = plan(&rules, &context(&root), &extensions).await.unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "old");
    assert_eq!(p.changed(), 1);
    let state = root.join("state");
    let backup = p.apply(&state).unwrap().unwrap();
    assert_eq!(fs::read_to_string(backup.join("0.bak")).unwrap(), "old");
    assert_eq!(fs::read_to_string(&file).unwrap(), "new");
    let again = plan(&rules, &context(&root), &extensions).await.unwrap();
    assert_eq!(again.changed(), 0);
}
#[tokio::test]
async fn concurrent_edit_prevents_all_writes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::write(root.join("a"), "old-a").unwrap();
    fs::write(root.join("b"), "old-b").unwrap();
    let rules = resolve(
        &[layer(
            &root,
            "org",
            &(replacement("a", "a", "new-a", false)
                + "\n"
                + &replacement("b", "b", "new-b", false)),
        )],
        &[],
    )
    .unwrap();
    let p = plan(&rules, &context(&root), &Extensions::default())
        .await
        .unwrap();
    fs::write(root.join("b"), "personal edit").unwrap();
    assert!(p.apply(&root.join("state")).is_err());
    assert_eq!(fs::read_to_string(root.join("a")).unwrap(), "old-a");
    assert_eq!(fs::read_to_string(root.join("b")).unwrap(), "personal edit");
}
#[tokio::test]
async fn different_id_cannot_bypass_locked_content() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let rules = resolve(
        &[
            layer(&root, "org", &replacement("a", "x", "required", true)),
            layer(&root, "personal", &replacement("z", "x", "bypass", false)),
        ],
        &[],
    )
    .unwrap();
    assert!(
        plan(&rules, &context(&root), &Extensions::default())
            .await
            .is_err()
    );
    assert!(!root.join("x").exists());
}
#[tokio::test]
async fn constrained_override_does_not_drop_ancestor_constraints() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let upper = format!("{}\n", replacement("a", "x", "allowed", false)).replace(
        "override=\"free\"",
        "override=\"constrained\"\nconstraints { enum=[\"allowed\",\"also-allowed\"] }",
    );
    let middle = replacement("a", "x", "also-allowed", false);
    let lower = replacement("a", "x", "forbidden", false);
    let rules = resolve(
        &[
            layer(&root, "org", &upper),
            layer(&root, "team", &middle),
            layer(&root, "person", &lower),
        ],
        &[],
    )
    .unwrap();
    assert!(
        plan(&rules, &context(&root), &Extensions::default())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn unresolved_match_does_not_write_other_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let rules=resolve(&[layer(&root,"org",&(replacement("a","a","new",false)+"\nrule \"z\" {\ntarget=\"b\"\nkind=\"text\"\noperation=\"match\"\npattern=\"mandatory\"\n}"))],&[]).unwrap();
    let p = plan(&rules, &context(&root), &Extensions::default())
        .await
        .unwrap();
    assert!(!p.compliant());
    assert!(p.apply(&root.join("state")).is_err());
    assert!(!root.join("a").exists());
}
#[tokio::test]
async fn non_idempotent_replacement_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::write(root.join("x"), "a").unwrap();
    let rules=resolve(&[layer(&root,"org","rule \"a\" {\ntarget=\"x\"\nkind=\"text\"\noperation=\"replace\"\npattern=\"a\"\nvalue=\"aa\"\n}")],&[]).unwrap();
    assert!(
        plan(&rules, &context(&root), &Extensions::default())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn traversal_outside_enrolled_root_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let rules = resolve(
        &[layer(
            &root,
            "org",
            &replacement("a", "../escape", "bad", false),
        )],
        &[],
    )
    .unwrap();
    assert!(
        plan(&rules, &context(&root), &Extensions::default())
            .await
            .is_err()
    );
}
#[cfg(unix)]
#[tokio::test]
async fn symlink_targets_and_parents_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join("link")).unwrap();
    let rules = resolve(
        &[layer(
            &root,
            "org",
            &replacement("a", "link/settings", "bad", false),
        )],
        &[],
    )
    .unwrap();
    assert!(
        plan(&rules, &context(&root), &Extensions::default())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn existing_case_alias_cannot_bypass_ancestor_guard() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::write(root.join("Settings"), "required").unwrap();
    if !root.join("settings").exists() {
        return;
    } // Case-sensitive filesystems have distinct targets.
    let rules = resolve(
        &[
            layer(
                &root,
                "org",
                &replacement("a", "Settings", "required", true),
            ),
            layer(
                &root,
                "person",
                &replacement("b", "settings", "bypass", false),
            ),
        ],
        &[],
    )
    .unwrap();
    assert!(
        plan(&rules, &context(&root), &Extensions::default())
            .await
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(root.join("Settings")).unwrap(),
        "required"
    );
}
